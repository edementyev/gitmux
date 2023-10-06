mod config;

use crate::config::{read_config, Config};

use anyhow::anyhow;
use clap::{Arg, ArgAction};
use config::{ConfigError, IncludeEntry};
use log::{info, trace};
use regex::{Captures, Regex};

use std::env::{self, VarError};
use std::io::{Read, Write};
use std::iter::Chain;
use std::process::{Command, Stdio};
use std::{ffi::OsStr, fs::DirEntry, time::Instant};

const EMPTY_STR: &str = "";

static APP_NAME: &str = "pfp";
static CONFIG_PATH_DEFAULT: &str = "${XDG_CONFIG_HOME}/pfp/config.json";

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),
    #[error("Cmd arguments error: {0}")]
    CmdArg(&'static str),
    #[error("Descend error: {0}")]
    Descend(#[from] anyhow::Error),
    #[error("Output error: {0}")]
    Output(#[from] std::io::Error),
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("Env var error: {0}: {1}")]
    EnvVar(VarError, String),
}

fn main() {
    match exec() {
        Ok(_) => std::process::exit(exitcode::OK),
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(exitcode::DATAERR);
        }
    }
}

fn exec() -> Result<(), Error> {
    // parse cli args
    let cmd = clap::Command::new(APP_NAME).arg(
        Arg::new("config")
            .short('c')
            .long("config")
            .action(ArgAction::Set)
            .default_value(CONFIG_PATH_DEFAULT)
            .value_name("FILE")
            .help("config file full path"),
    );
    let config_path = expand(
        cmd.get_matches()
            .get_one::<String>("config")
            .ok_or_else(|| Error::CmdArg("error: wrong type used for --config"))?,
    )?;
    let config_result = read_config(&config_path);
    let config = if config_result.is_err() && config_path == CONFIG_PATH_DEFAULT {
        // default value is used for --config and it does not exist in file system
        // -> use default config value
        config_result
            .map_err(|e| println!("{}, config path={}, using default config", e, config_path))
            .unwrap_or_default()
    } else {
        // either read_config succeeded, or it failed with provided custom --config path
        // -> continue or propagate error
        config_result?
    };
    trace!("config {:#?}", config);

    let dir_list = get_dir_list(config)?;

    let pick = fuzzy_pick_from(dir_list)?;
    if pick.is_empty() {
        trace!("Empty pick");
        std::process::exit(exitcode::OK)
    } else {
        trace!("{}", pick);
    }

    tmux_pane(pick.clone(), get_pane_name(pick)?)?;
    Ok(())
}

fn expand(path: &str) -> Result<String, Error> {
    let re = Regex::new(r"\$\{?([^\}/]+)\}?")?;
    let mut errors: Vec<(VarError, String)> = Vec::new();
    let result: String = re
        .replace_all(path, |captures: &Captures| match &captures[1] {
            EMPTY_STR => EMPTY_STR.to_string(),
            varname => env::var(OsStr::new(varname))
                .map_err(|e| errors.push((e, varname.to_owned())))
                .unwrap_or_default(),
        })
        .into();
    if let Some(error_tuple) = errors.pop() {
        return Err(Error::EnvVar(error_tuple.0, error_tuple.1));
    }
    Ok(result)
}

fn get_dir_list(config: Config) -> Result<String, Error> {
    let mut out = vec![];
    for include_entry in config.include.iter() {
        for path in &include_entry.paths {
            let expanded_path = expand(path)?;
            out.push(expanded_path.clone());
            descend_recursive(&expanded_path, 0, &mut out, include_entry, &config)?;
        }
    }
    Ok(out.join("\n"))
}

fn descend_recursive(
    path: &str,
    depth: u8,
    output: &mut Vec<String>,
    include_entry: &IncludeEntry,
    config: &Config,
) -> Result<bool, Error> {
    // always include start of the tree
    let mut include_this_path = false;
    if !include_this_path {
        let markers_chain = include_entry
            .markers
            .iter()
            .chain(if include_entry.use_root_markers {
                config.markers.iter()
            } else {
                [].iter()
            });
        // search current dir for markers
        for marker in markers_chain {
            if *marker == "*" || std::fs::metadata(format!("{}/{}", path, marker)).is_ok() {
                trace!("match found {}, marker {}", path, marker);
                include_this_path = true;
                if include_entry.stop_on_match.unwrap_or(config.stop_on_match) {
                    return Ok(include_this_path);
                } else {
                    break;
                }
            }
        }
    }
    if depth >= include_entry.depth {
        return Ok(include_this_path);
    }
    // read child dirs
    if let Ok(iter) = std::fs::read_dir(path) {
        let mut children = vec![];
        for ref entry in iter.flatten() {
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| anyhow!("entry is not utf8 string: {:#?}", entry.file_name()))?
                .to_string();
            let ignore_chain =
                include_entry
                    .ignore
                    .iter()
                    .chain(if include_entry.use_root_ignore {
                        config.ignore.iter()
                    } else {
                        [].iter()
                    });
            if is_dir(entry)?
                && (include_entry.show_hidden.unwrap_or(config.traverse_hidden)
                    || !is_dot_dir(&name))
                && !is_in_ignore(&name, ignore_chain)
            {
                children.push(String::from(entry.path().to_str().ok_or_else(|| {
                    anyhow!("entry.path() is not valid utf8: {:#?}", entry.path())
                })?));
            }
        }
        // walk current dir's children
        for child in children {
            if descend_recursive(&child, depth + 1, output, include_entry, config)? {
                output.push(child);
                // if child is included, also include parent
                include_this_path = true;
            };
        }
        // pass inclusion flag up the tree
        // NOTE: make this optional: include_parent
        Ok(include_this_path)
    } else {
        Ok(false)
    }
}

fn is_in_ignore(
    name: &str,
    ignore_dirs: Chain<std::slice::Iter<&str>, std::slice::Iter<&str>>,
) -> bool {
    for ignore_pat in ignore_dirs {
        if *ignore_pat == name {
            return true;
        }
    }
    false
}

fn is_dir(entry: &DirEntry) -> Result<bool, std::io::Error> {
    Ok(entry.file_type()?.is_dir())
}

fn is_dot_dir(name: &str) -> bool {
    name.starts_with('.')
}

fn fuzzy_pick_from(s: String) -> Result<String, std::io::Error> {
    let mut result = String::new();
    let mut cmd = Command::new("fzf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(["--layout", "reverse"])
        .args(["--preview", "tree -C '{}'"])
        .args(["--preview-window", "right:nohidden"])
        .spawn()?;
    {
        let stdin = cmd.stdin.as_mut().unwrap();
        stdin.write_all(s.as_bytes())?;
        let stdout = cmd.stdout.as_mut().unwrap();
        stdout.read_to_string(&mut result)?;
        cmd.wait()?;
    }
    Ok(result.trim_end().to_owned())
}

fn get_pane_name(path: String) -> Result<String, anyhow::Error> {
    let re = Regex::new(r"/(?P<first>[^/]+)/{1}(?P<second>[^/]+)$")?;
    if let Some(caps) = re.captures_iter(&path).next() {
        return Ok(format!(
            "{}/{}",
            caps["first"].chars().take(4).collect::<String>(),
            &caps["second"]
        ));
    }
    Ok(path)
}

fn tmux_pane(path: String, name: String) -> Result<(), anyhow::Error> {
    let mut cmd = Command::new("tmux")
        .arg("neww")
        .args(["-c", &path])
        .args(["-n", &name])
        .spawn()?;
    {
        cmd.wait()?;
    }
    Ok(())
}

#[allow(dead_code)]
fn measure<F>(name: &str, mut f: F)
where
    F: FnMut(),
{
    let start = Instant::now();
    f();
    info!("Time elapsed for {} is: {:?}", name, start.elapsed());
}
