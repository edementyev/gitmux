use anyhow::anyhow;
use clap::{Arg, ArgAction};
use log::{info, trace};
use regex::{Captures, Regex};
use serde::Deserialize;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::{ffi::OsStr, fs::DirEntry, time::Instant};

#[derive(Deserialize, Debug, Default)]
struct Config<'a> {
    #[serde(borrow = "'a")]
    markers: Vec<&'a str>,
    ignore: Vec<&'a str>,
    entries: Vec<ConfigEntry<'a>>,
}

#[derive(Deserialize, Debug, Default)]
struct ConfigEntry<'a> {
    #[serde(borrow = "'a")]
    paths: Vec<&'a str>,
    #[serde(default)]
    exclude: Vec<&'a str>,
    #[serde(default)]
    depth: Option<u8>,
    #[serde(default)]
    include_all: bool,
    #[serde(default)]
    markers: Vec<&'a str>,
}

use std::env::{self, VarError};

const EMPTY_STR: &str = "";

static APP_NAME: &str = "pfp";
static DEFAULT_CONFIG_PATH: &str = "${XDG_CONFIG_HOME}/gitmux/config.json";

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Parse config error: {0}")]
    ParseConfig(#[from] serde_jsonc::Error),
    #[error("Cmd arguments error: {0}")]
    CmdArg(&'static str),
    #[error("Descend error: {0}")]
    Descend(#[from] anyhow::Error),
    #[error("Output error: {0}")]
    Output(#[from] std::io::Error),
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("Env var error: {0}")]
    EnvVar(#[from] VarError),
}

fn main() {
    match _main() {
        Ok(_) => std::process::exit(exitcode::OK),
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(exitcode::DATAERR);
        }
    }
}

fn _main() -> Result<(), Error> {
    // parse cli args
    let cmd = clap::Command::new(APP_NAME).arg(
        Arg::new("config")
            .short('c')
            .long("config")
            .action(ArgAction::Set)
            .default_value(DEFAULT_CONFIG_PATH)
            .value_name("FILE")
            .help("config file full path"),
    );
    // TODO: cmd flag to measure each dir walk time
    //
    // ).arg();
    let config_path = expand(
        cmd.get_matches()
            .get_one::<String>("config")
            .ok_or_else(|| Error::CmdArg("error: wrong type used for --config"))?,
    )?;
    // parse config
    let config_content = std::fs::read_to_string(config_path)?;
    let config: Config = serde_jsonc::from_str(&config_content)?;
    trace!("config {:#?}", config);

    let dir_list = get_dir_list(config)?;

    let pick = fuzzy_pick_from(dir_list)?;
    if pick.is_empty() {
        println!("Empty pick");
        std::process::exit(exitcode::OK)
    } else {
        trace!("{}", pick);
    }

    tmux_pane(pick.clone(), get_pane_name(pick)?)?;
    Ok(())
}

fn expand(path: &str) -> Result<String, Error> {
    let re = Regex::new(r"\$\{?([^\}/]+)\}?")?;
    let caps = re.captures(path);
    if caps.is_some() {
        let mut errors: Vec<VarError> = Vec::new();
        let result: String = re
            .replace_all(path, |captures: &Captures| match &captures[1] {
                EMPTY_STR => EMPTY_STR.to_string(),
                varname => env::var(OsStr::new(varname))
                    .map_err(|e| errors.push(e))
                    .unwrap_or_default(),
            })
            .into();
        if let Some(error) = errors.pop() {
            return Err(Error::EnvVar(error));
        }
        Ok(result)
    } else {
        Ok(path.to_owned())
    }
}

fn get_dir_list(config: Config) -> Result<String, Error> {
    let mut out = vec![];
    for mut config_entry in config.entries {
        // exclude and markers behaviour:
        // use root list if current entry does not have it's own list
        // or if it explicitly includes root with "*"
        let use_root_exclude = *config_entry.exclude.first().unwrap_or(&EMPTY_STR) == "*";
        if use_root_exclude {
            config_entry.exclude.pop();
        }
        if config_entry.exclude.is_empty() || use_root_exclude {
            config_entry.exclude.extend(&config.ignore);
        }
        let use_root_markers = *config_entry.markers.first().unwrap_or(&EMPTY_STR) == "*";
        if use_root_markers {
            config_entry.markers.pop();
        }
        if config_entry.markers.is_empty() || use_root_markers {
            config_entry.markers.extend(&config.markers);
        }
        for path in &config_entry.paths {
            descend_recursive(expand(path)?.as_str(), 0, &mut out, &config_entry)?;
        }
    }
    Ok(out.join("\n"))
}

fn descend_recursive(
    path: &str,
    depth: u8,
    output: &mut Vec<String>,
    config: &ConfigEntry,
) -> Result<bool, Error> {
    if config.depth.is_some() && depth > config.depth.unwrap_or(u8::MAX) {
        return Ok(false);
    }
    // always include root path
    let mut include_this_path = depth == 0 || config.include_all;
    // match current path against markers
    for marker in &config.markers {
        if std::fs::metadata(format!("{}/{}", path, marker)).is_ok() {
            trace!("match found {}", path);
            output.push(path.to_string());
            return Ok(true);
        }
    }
    if let Ok(iter) = std::fs::read_dir(path) {
        let mut children = vec![];
        for ref entry in iter.flatten() {
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| anyhow!("entry is not utf8 string: {:#?}", entry.file_name()))?
                .to_string();
            if is_dir(entry)? && !is_dot_dir(&name) && !is_in_ignore(&name, &config.exclude) {
                children.push(String::from(entry.path().to_str().ok_or_else(|| {
                    anyhow!("entry.path() is not valid utf8: {:#?}", entry.path())
                })?));
            }
        }
        // walk current dir's children
        for child in children {
            if descend_recursive(child.as_str(), depth + 1, output, config)? && !include_this_path {
                // if child is included, also include parent
                include_this_path = true;
            };
        }
        if include_this_path {
            output.push(path.to_string());
        }
        // pass inclusion flag up the call tree
        Ok(include_this_path)
    } else {
        Ok(false)
    }
}

fn is_in_ignore(name: &str, ignore_dirs: &Vec<&str>) -> bool {
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
        .arg("--preview")
        .arg("'tree -C {}'")
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

fn get_parent_path_name(path: &str) -> Result<String, anyhow::Error> {
    let mut result = String::new();
    let mut cmd = Command::new("realpath")
        .stdout(Stdio::piped())
        .arg(path.to_owned() + "/../../")
        .spawn()?;
    {
        let stdout = cmd.stdout.as_mut().unwrap();
        stdout.read_to_string(&mut result)?;
        cmd.wait()?;
    }
    Ok(result.trim_end().to_owned())
}

fn get_pane_name(path: String) -> Result<String, anyhow::Error> {
    let mut result = String::new();
    let parent_path = get_parent_path_name(&path)?;
    let mut cmd = Command::new("realpath")
        .stdout(Stdio::piped())
        .arg(format!("--relative-to={}", parent_path))
        .arg(path)
        .spawn()?;
    {
        let stdout = cmd.stdout.as_mut().unwrap();
        stdout.read_to_string(&mut result)?;
        cmd.wait()?;
    }
    // TODO: trim first part of path to 4 characters
    Ok(result)
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
