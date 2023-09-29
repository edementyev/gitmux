use anyhow::anyhow;
use clap::{Arg, ArgAction};
use log::{info, trace};
use regex::{Captures, Regex};
use serde::Deserialize;
use std::io::{stdout, Write};
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

use std::env;

const EMPTY_STR: &str = "";
// const RUST_LIB_BACKTRACE: &str = "RUST_LIB_BACKTRACE";

static APP_NAME: &str = "gitmux";
static DEFAULT_CONFIG_PATH: &str = "${XDG_CONFIG_HOME}/gitmux/config.json";

fn main() {
    match main_() {
        Ok(_) => {
            std::process::exit(exitcode::OK);
        }
        Err(e) => {
            eprintln!("{}", e);
            match e {
                GitmuxError::Config(_) => std::process::exit(exitcode::CONFIG),
                GitmuxError::Descend(_) => std::process::exit(exitcode::DATAERR),
                GitmuxError::Output(_) => std::process::exit(exitcode::IOERR),
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
enum ConfigError {
    #[error("Parse: {0}")]
    Parse(#[from] serde_jsonc::Error),
    #[error("Cmd arguments: {0}")]
    CmdArg(&'static str),
    #[error("Read path: {0}")]
    ReadPath(String),
}

#[derive(thiserror::Error, Debug)]
enum GitmuxError {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),
    #[error("Descend error: {0}")]
    Descend(#[from] anyhow::Error),
    #[error("Output error: {0}")]
    Output(#[from] std::io::Error),
}

fn main_() -> Result<(), GitmuxError> {
    // turn on backtrace for anyhow::Error if it is not explicitly set
    // if env::var(RUST_LIB_BACKTRACE).is_err() {
    //     env::set_var(RUST_LIB_BACKTRACE, "1");
    // }
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
    let matches = cmd.get_matches();
    // parse config
    let p: String = expand(
        matches
            .get_one::<String>("config")
            .ok_or(ConfigError::CmdArg(
                "error: wrong type used for arg --config",
            ))?,
    );
    let file = std::fs::read_to_string(&p)
        .map_err(|e| ConfigError::ReadPath(format!("error reading path {}: {}", p, e)))?;
    let config: Config = serde_jsonc::from_str(file.as_str()).map_err(ConfigError::Parse)?;
    trace!("config {:#?}", config);
    let mut output_vec = vec![];
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
            descend_recursive(expand(path).as_str(), 0, &mut output_vec, &config_entry)?;
        }
    }
    let mut out = stdout();
    let mut output: String = EMPTY_STR.to_string();
    for r in output_vec {
        output += (r + "\n").as_str();
    }
    trace!("output {:#?}", output);
    Ok(out.write_fmt(format_args!("{}", output))?)
}

fn descend_recursive(
    path: &str,
    depth: u8,
    output: &mut Vec<String>,
    config: &ConfigEntry,
) -> Result<bool, GitmuxError> {
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
                .ok_or(anyhow!(
                    "entry is not utf8 string: {:#?}",
                    entry.file_name()
                ))?
                .to_string();
            if is_dir(entry) && !is_dot_dir(&name) && !is_in_ignore(&name, &config.exclude) {
                children.push(String::from(entry.path().to_str().expect("path err")));
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
    // not ignored
    for ignore_pat in ignore_dirs {
        if *ignore_pat == name {
            return true;
        }
    }
    false
}

fn is_dir(entry: &DirEntry) -> bool {
    entry.file_type().expect("err on file_type").is_dir()
}

fn is_dot_dir(name: &str) -> bool {
    // not a dot dir
    name.starts_with('.')
}

fn expand(path: &str) -> String {
    let re = Regex::new(r"\$\{?([^\}/]+)\}?").expect("invalid regex");
    let result: String = re
        .replace_all(path, |captures: &Captures| match &captures[1] {
            EMPTY_STR => EMPTY_STR.to_string(),
            varname => {
                env::var(OsStr::new(varname)).unwrap_or_else(|e| panic!("no such var error: {}", e))
            }
        })
        .into();
    result
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
