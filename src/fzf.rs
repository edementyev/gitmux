use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

use crate::Error;

pub(crate) fn execute_fzf_command<'a>(
    args: impl Iterator<Item = &'a str>,
    input: &str,
) -> Result<String, crate::Error> {
    let mut child = Command::new("fzf")
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .args(args)
        .spawn()?;
    let mut result = String::new();
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| Error::UnwrapIOStream("Could not get cmd.stdin"))?;
        stdin.write_all(input.as_bytes())?;
        stdin.flush()?;
        child.wait()?;
    }
    {
        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| Error::UnwrapIOStream("Could not get cmd.stdout"))?;
        stdout.read_to_string(&mut result)?;
    }
    Ok(result)
}
