use std::process;

use crate::fs::{expand, is_file_str};

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

/// executes tmux new-window/new-session with shell-command depending on target filetype
/// if target is a file, launches this file in $EDITOR instead of just opening path in new window
pub(crate) fn execute_tmux_window_command(cmd: &str, target: &str) -> Result<process::Output, anyhow::Error> {
    if is_file_str(target) {
        Ok(execute_tmux_command_with_stdin(
            &format!("{} {} {}", cmd, expand("$EDITOR")?, target),
            process::Stdio::piped(),
        )?)
    } else {
        Ok(execute_tmux_command_with_stdin(cmd, process::Stdio::piped())?)
    }
}
