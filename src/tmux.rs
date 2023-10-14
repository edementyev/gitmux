use std::process;

pub(crate) fn execute_tmux_command(s: &str) -> std::io::Result<process::Output> {
    let args = s.split(' ').skip(1);
    process::Command::new("tmux").args(args).output()
}

pub(crate) fn execute_tmux_command_with_stdin(
    cmd: &str,
    stdin: process::Stdio,
) -> std::io::Result<process::Output> {
    let args = cmd.split(' ').skip(1);
    process::Command::new("tmux").stdin(stdin).args(args).output()
}
