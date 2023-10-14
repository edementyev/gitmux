mod config;
mod fs;
mod fzf;
mod selectors;
mod tmux;

use crate::config::{read_config, ConfigError};
use crate::fs::{expand, get_pane_name};
use crate::selectors::{pick_from, pick_project};
use crate::tmux::{execute_tmux_command, execute_tmux_command_with_stdin};

use clap::{Arg, ArgAction};
use log::{info, trace};

use std::env::VarError;
use std::process;
use std::string::FromUtf8Error;
use std::time::Instant;

static APP_NAME: &str = "pfp";
static CONFIG_PATH_DEFAULT: &str = "${XDG_CONFIG_HOME}/pfp/config.json";

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
    match cli() {
        Ok(_) => std::process::exit(exitcode::OK),
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(exitcode::DATAERR);
        }
    }
}

const KILL_SESSION_SUBC: &str = "kill-session";
const SESSIONS_SUBC: &str = "sessions";
const START_SUBC: &str = "start";
const NEW_SESSION_SUBC: &str = "new-session";
const NEW_PANE_SUBC: &str = "new-pane";

const CONFIG_ARG: &str = "config";
const START_INHERIT_STDIN_ARG: &str = "inherit-stdin";

fn cli() -> Result<(), Error> {
    // parse cli args
    let mut cmd = clap::Command::new(APP_NAME)
        .about("Pfp helps you manage your projects with tmux sessions and panes")
        .arg(
            Arg::new(CONFIG_ARG)
                .short('c')
                .long(CONFIG_ARG)
                .action(ArgAction::Set)
                .default_value(CONFIG_PATH_DEFAULT)
                .value_name("FILE")
                .help("config file full path"),
        )
        .subcommand(clap::Command::new(NEW_SESSION_SUBC).about("Pick a path and create new tmux session"))
        .subcommand(clap::Command::new(NEW_PANE_SUBC).about("Pick a path and create new tmux window"))
        .subcommand(clap::Command::new(KILL_SESSION_SUBC).about("Kill current session and switch to last/previous session"))
        .subcommand(clap::Command::new(SESSIONS_SUBC).about("Show list of active sessions, select one to switch to it"))
        .subcommand(
            clap::Command::new(START_SUBC).about("Start tmux sessions from predefined list").arg(
                Arg::new(START_INHERIT_STDIN_ARG)
                    .short('i')
                    .long(START_INHERIT_STDIN_ARG)
                    .action(ArgAction::SetTrue)
                    .help(
                        "inherit stdin from parent pfp process (use it if you want to start tmux with pfp)",
                    ),
            ),
        );

    let help = cmd.render_help();
    let arg_matches = cmd.get_matches();

    let path = expand(
        arg_matches
            .get_one::<String>(CONFIG_ARG)
            .ok_or_else(|| Error::CmdArg(format!("error: wrong type used for {}", CONFIG_ARG)))?,
    )?;

    let config = {
        let cfg = read_config(&path);
        if cfg.is_err() && path == CONFIG_PATH_DEFAULT {
            // default value is used for --config and config does not exist in file system
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

    match arg_matches.subcommand() {
        Some((KILL_SESSION_SUBC, _)) => {
            let mut session_name =
                String::from_utf8(execute_tmux_command("tmux display-message -p '#S'")?.stdout)?;
            session_name.retain(|x| x != '\'' && x != '\n');
            let out = execute_tmux_command("tmux switch-client -l")?;
            if !out.status.success() {
                execute_tmux_command("tmux switch-client -p")?;
            }
            execute_tmux_command(&format!("tmux kill-session -t {}", session_name,))?;
        }
        Some((SESSIONS_SUBC, _)) => {
            let mut current_session =
                String::from_utf8(execute_tmux_command("tmux display-message -p '#S:#I'")?.stdout)?;
            current_session.retain(|x| x != '\'' && x != '\n');
            let mut sessions = String::from_utf8(
                execute_tmux_command("tmux list-sessions -F '#S:#I,#{session_id}'")?.stdout,
            )?
            .trim_end()
            .to_owned();
            sessions.retain(|x| x != '\'');
            let mut s = sessions
                .split('\n')
                .map(|x| x.split_once(',').expect("Wrong list-sessions format!"))
                .collect::<Vec<(&str, &str)>>();
            s.sort_by_key(|k| k.1);
            sessions = s.into_iter().map(|x| x.0).collect::<Vec<&str>>().join("\n");
            let idx = sessions
                .split('\n')
                .enumerate()
                .find(|x| x.1 == current_session)
                .map(|x| x.0)
                .unwrap_or(0);
            let mut pick = pick_from(
                &sessions,
                "Active sessions:",
                &[
                    "--layout",
                    "reverse",
                    "--sync",
                    "--bind",
                    &format!("load:pos({})", idx + 1),
                ],
            )?;
            pick.retain(|x| x != '\'' && x != '\n');
            if !pick.is_empty() {
                execute_tmux_command(&format!("tmux switch-client -t {}", pick))?;
            }
        }
        Some((START_SUBC, arg_matches)) => {
            if config.sessions.is_empty() {
                execute_tmux_command_with_stdin("tmux", process::Stdio::inherit())?;
                return Ok(());
            }
            let mut sessions = String::from_utf8(execute_tmux_command("tmux list-sessions -F '#S'")?.stdout)?;
            sessions.retain(|x| x != '\'');
            let pick = pick_from(
                &config
                    .sessions
                    .iter()
                    .map(|s| s.name)
                    .collect::<Vec<&str>>()
                    .join("\n"),
                "Start sessions:",
                &["-m", "--layout", "reverse"],
            )?;
            let picked_sessions = pick.split('\n').filter(|x| !x.is_empty()).collect::<Vec<&str>>();
            for session in config.sessions {
                if picked_sessions.contains(&session.name) {
                    let session_exists = sessions
                        .split('\n')
                        .find(|x| *x == session.name)
                        .map(|_| true)
                        .unwrap_or(false);
                    if session_exists {
                        println!("session {} exists", session.name);
                        continue;
                    }
                    let mut iter = session.panes.iter();
                    let home = expand("$HOME")?;
                    let home_as_str = home.as_str();
                    let first_pane = &expand(iter.next().unwrap_or(&home_as_str))?;
                    // create session with first window
                    execute_tmux_command(&format!(
                        "tmux new-session -d -s {} -n {} -c {}",
                        session.name,
                        get_pane_name(first_pane)?,
                        first_pane,
                    ))?;
                    for pane in iter {
                        let pane = &expand(pane)?;
                        let mut window = String::from_utf8(
                            execute_tmux_command(&format!(
                                "tmux new-window -d -n {} -c {} -P -F '#S:#I'",
                                get_pane_name(pane)?,
                                pane,
                            ))?
                            .stdout,
                        )?;
                        window.retain(|x| x != '\'' && x != '\n');
                        execute_tmux_command(&format!(
                            "tmux move-window -s {} -t {}:",
                            window, session.name
                        ))?;
                    }
                    // renumber panes with no-op move
                    execute_tmux_command(&format!(
                        "tmux movew -r -s {}:1 -t {}:1",
                        session.name, session.name
                    ))?;
                }
            }
            let tmux_stdin = match arg_matches.get_one(START_INHERIT_STDIN_ARG).unwrap_or(&false) {
                true => process::Stdio::inherit(),
                false => process::Stdio::piped(),
            };
            execute_tmux_command_with_stdin("tmux attach", tmux_stdin)?;
        }
        Some((NEW_PANE_SUBC, _)) => {
            let pick = pick_project(&config)?;
            execute_tmux_command(&format!(
                "tmux new-window -n {} -c {}",
                &get_pane_name(&pick)?,
                &pick
            ))?;
        }
        Some((NEW_SESSION_SUBC, _)) => {
            let pick = pick_project(&config)?;
            // spawn tmux session
            let mut name = get_pane_name(&pick)?;
            execute_tmux_command(&format!(
                "tmux new-session -d -n {} -s {} -c {}",
                name, name, &pick
            ))?;
            name.retain(|x| x != '\'' && x != '\n');
            execute_tmux_command(&format!("tmux switch-client -t {}:1", name))?;
        }
        // no subcommand
        _ => {
            println!("{}", help);
        }
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
