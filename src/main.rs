mod config;

use crate::config::{Config, ConfigError, IncludeEntry};

use anyhow::anyhow;
use clap::{Arg, ArgAction};
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
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Unwrap IO stream error: {0}")]
    UnwrapIOStream(&'static str),
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

    let path = expand(
        cmd.get_matches()
            .get_one::<String>("config")
            .ok_or_else(|| Error::CmdArg("error: wrong type used for --config"))?,
    )?;

    let config = {
        fn parse_config(path: &str) -> Result<Config, ConfigError> {
            let contents = Box::leak(Box::new(std::fs::read_to_string(path)?));
            let cfg: Config = serde_jsonc::from_str(contents)?;
            Ok(cfg)
        }
        let cfg = parse_config(&path);
        if cfg.is_err() && path == CONFIG_PATH_DEFAULT {
            // default value is used for --config and it does not exist in file system
            // -> use default config value
            cfg.map_err(|e| println!("{}, config path={}, using default config", e, path))
                .unwrap_or_default()
        } else {
            // either read_config succeeded, or it failed with provided custom --config path
            // -> continue or propagate error
            cfg?
        }
    };
    trace!("config {:#?}", config);

    // get dirs' paths
    let dirs = {
        let mut list = vec![];
        for include_entry in config.include.iter() {
            for path in &include_entry.paths {
                let expanded_path = expand(path)?;
                // always include root of the tree
                list.push(expanded_path.clone());
                descend_recursive(&expanded_path, 0, &mut list, include_entry, &config)?;
            }
        }
        list.join("\n")
    };

    // pick one from list with fzf
    let pick = {
        let mut result = String::new();
        let mut cmd = Command::new("fzf")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .args(["--layout", "reverse"])
            .args(["--preview", "tree -C '{}'"])
            .args(["--preview-window", "right:nohidden"])
            .spawn()?;
        {
            let stdin = cmd
                .stdin
                .as_mut()
                .ok_or_else(|| Error::UnwrapIOStream("Could not get cmd.stdin"))?;
            stdin.write_all(dirs.as_bytes())?;
            let stdout = cmd
                .stdout
                .as_mut()
                .ok_or_else(|| Error::UnwrapIOStream("Could not get cmd.stdout"))?;
            stdout.read_to_string(&mut result)?;
            cmd.wait()?;
        }
        result = result.trim_end().to_owned();
        if result.is_empty() {
            trace!("Empty pick");
            std::process::exit(exitcode::OK)
        } else {
            trace!("{}", result);
            result
        }
    };

    // spawn tmux pane
    Command::new("tmux")
        .arg("neww")
        .args(["-c", &pick])
        .args(["-n", &get_pane_name(&pick)?])
        .spawn()?
        .wait()?;
    Ok(())
}

fn get_pane_name(path: &str) -> Result<String, anyhow::Error> {
    let re = Regex::new(r"/(?P<first>[^/]+)/{1}(?P<second>[^/]+)$")?;
    let mut iter = re.captures_iter(path);
    if let Some(caps) = iter.next() {
        Ok(format!(
            "{}/{}",
            caps["first"].chars().take(4).collect::<String>(),
            &caps["second"]
        ))
    } else {
        Ok(path.to_string())
    }
}

fn descend_recursive(
    path: &str,
    depth: u8,
    output: &mut Vec<String>,
    include_entry: &IncludeEntry,
    config: &Config,
) -> Result<bool, Error> {
    let mut include_this_path = false;
    if !include_this_path {
        let markers_chain =
            include_entry
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

#[allow(dead_code)]
fn measure<F>(name: &str, mut f: F)
where
    F: FnMut(),
{
    let start = Instant::now();
    f();
    info!("Time elapsed for {} is: {:?}", name, start.elapsed());
}
