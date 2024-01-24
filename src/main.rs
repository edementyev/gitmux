mod cli;
mod config;
mod fs;
mod fzf;
mod selectors;
mod tmux;

use crate::config::ConfigError;
use log::info;

use std::env::VarError;
use std::string::FromUtf8Error;
use std::time::Instant;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),
    #[error("Cmd arguments error: {0}")]
    CmdArg(String),
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
    #[error("Parse utf8 error: {0}")]
    ParseUTF8(#[from] FromUtf8Error),
    #[error("Empty pick!")]
    EmptyPick(),
}

fn main() {
    match cli::cli() {
        Ok(_) => std::process::exit(exitcode::OK),
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(exitcode::DATAERR);
        }
    }
}

#[allow(dead_code)]
pub fn measure<F>(name: &str, mut f: F)
where
    F: FnMut(),
{
    let start = Instant::now();
    f();
    info!("Time elapsed for {} is: {:?}", name, start.elapsed());
}
