use std::process;

use crate::fs::{expand, path_is_file};

pub(crate) fn execute_tmux_command_with_stdin(
    cmd: &str,
    stdin: process::Stdio,
) -> std::io::Result<process::Output> {
    let args = cmd.split(' ').skip(1);
    process::Command::new("tmux").stdin(stdin).args(args).output()
}

pub(crate) fn execute_tmux_command(cmd: &str) -> std::io::Result<process::Output> {
    execute_tmux_command_with_stdin(cmd, process::Stdio::piped())
}

/// Executes tmux new-window/new-session with shell-command depending on target filetype. 
/// If target is a file, launches this file in $EDITOR instead of just opening path in new window.
/// IMPORTANT: '-c' flag (specifying working directory for the window) should be placed at the end of the command, as we want to trim filename from that path.
pub(crate) fn execute_tmux_window_command(cmd: &str, target: &str) -> Result<process::Output, anyhow::Error> {
    if path_is_file(target) {
        let split = cmd.split('/');
        Ok(execute_tmux_command_with_stdin(
            &format!(
                "{} {} {}",
                split
                    .clone()
                    .take(split.count() - 1)
                    .collect::<Vec<&str>>()
                    .join("/"),
                expand("$EDITOR")?,
                target
            ),
            process::Stdio::piped(),
        )?)
    } else {
        Ok(execute_tmux_command_with_stdin(cmd, process::Stdio::piped())?)
    }
}
