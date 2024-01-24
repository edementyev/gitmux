use log::trace;
use std::process;

use crate::config::{read_config, Session};
use crate::fs::{expand, get_pane_name, get_session_name};
use crate::selectors::{pick_project, select_from_list};
use crate::tmux::{execute_tmux_command, execute_tmux_command_with_stdin};

use clap::{Arg, ArgAction};

static APP_NAME: &str = "pfp";
static CONFIG_PATH_DEFAULT: &str = "${XDG_CONFIG_HOME}/pfp/config.json";

const KILL_SESSION_SUBC: &str = "kill-session";
const SESSIONS_SUBC: &str = "sessions";
const START_SUBC: &str = "start";
const NEW_SESSION_SUBC: &str = "new-session";
const NEW_PANE_SUBC: &str = "new-pane";

const CONFIG_ARG: &str = "config";
const START_INHERIT_STDIN_ARG: &str = "attach"; // inherit stdin

pub(crate) fn cli() -> Result<(), super::Error> {
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
        .subcommand(
            clap::Command::new(KILL_SESSION_SUBC)
                .about("Kill current session and switch to last/previous session"),
        )
        .subcommand(
            clap::Command::new(SESSIONS_SUBC)
                .about("Show list of active sessions, select one to switch to it"),
        )
        .subcommand(
            clap::Command::new(START_SUBC)
                .about("Start tmux sessions from predefined list")
                .arg(
                    Arg::new(START_INHERIT_STDIN_ARG)
                        .short('a')
                        .long(START_INHERIT_STDIN_ARG)
                        .action(ArgAction::SetTrue)
                        .help("attach to tmux session after start"),
                ),
        );

    let help = cmd.render_help();
    let arg_matches = cmd.get_matches();

    let path = expand(
        arg_matches
            .get_one::<String>(CONFIG_ARG)
            .ok_or_else(|| super::Error::CmdArg(format!("error: wrong type used for {}", CONFIG_ARG)))?,
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
            let mut pick = select_from_list(
                &sessions,
                "Active sessions:",
                &[
                    "--layout",
                    "reverse",
                    "--preview",
                    "tmux capture-pane -ept {}",
                    "--preview-window",
                    "right:nohidden",
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
            let stdin_opt = match arg_matches.get_one(START_INHERIT_STDIN_ARG).unwrap_or(&false) {
                true => process::Stdio::inherit(),
                false => process::Stdio::piped(),
            };
            if config.sessions.is_empty() {
                execute_tmux_command_with_stdin("tmux", stdin_opt)?;
                return Ok(());
            }
            let mut sessions = String::from_utf8(execute_tmux_command("tmux list-sessions -F '#S'")?.stdout)?;
            sessions.retain(|x| x != '\'');
            let pick = select_from_list(
                &config
                    .sessions
                    .iter()
                    .map(|s| s.name)
                    .collect::<Vec<&str>>()
                    .join("\n"),
                "Start sessions:",
                &[
                    "-m",
                    "--layout",
                    "reverse",
                    "--preview",
                    &format!(
                        "echo '{}'",
                        config
                            .sessions
                            .iter()
                            .map(Session::to_string)
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    "--preview-window",
                    "right:nohidden",
                ],
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
                    let mut iter = session.windows.iter();
                    let home = expand("$HOME")?;
                    let first_pane = &expand(iter.next().unwrap_or(&home.as_str()))?;
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
            execute_tmux_command_with_stdin("tmux attach", stdin_opt)?;
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
            let mut pane_name = get_pane_name(&pick)?;
            let session_name = get_session_name(&pane_name);
            execute_tmux_command(&format!(
                "tmux new-session -d -s {} -n {} -c {}",
                session_name, pane_name, &pick
            ))?;
            pane_name.retain(|x| x != '\'' && x != '\n');
            execute_tmux_command(&format!("tmux switch-client -t {}:1", session_name))?;
        }
        // no subcommand
        _ => {
            println!("{}", help);
        }
    }

    Ok(())
}
