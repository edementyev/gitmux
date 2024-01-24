use crate::config::{Config, IncludeEntry};
use crate::Error;

use anyhow::anyhow;
use log::trace;
use regex::{Captures, Regex};

use std::env::{self, VarError};
use std::ffi::OsStr;
use std::fs::DirEntry;
use std::iter::Chain;

const EMPTY_STR: &str = "";

pub(crate) fn expand(path: &str) -> Result<String, Error> {
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

pub(crate) fn get_pane_name(path: &str) -> Result<String, anyhow::Error> {
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

pub(crate) fn get_session_name(pane_name: &String) -> String {
    let mut s = String::from(pane_name);
    s.retain(|x| x != '.');
    s
}

pub(crate) fn descend_recursive(
    path: &str,
    depth: u8,
    output: &mut Vec<String>,
    include_entry: &IncludeEntry,
    config: &Config,
) -> Result<bool, Error> {
    let mut include_this_path = false;
    let dir_contents = std::fs::read_dir(path)?.flatten().collect::<Vec<DirEntry>>();
    let markers_chain = include_entry
        .markers
        .iter()
        .chain(if include_entry.use_root_markers {
            config.markers.iter()
        } else {
            [].iter()
        });
    let markers = markers_chain.copied().collect::<Vec<&str>>();

    // search current dir for markers
    for entry in dir_contents.iter() {
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| anyhow!("entry is not utf8 string: {:#?}", entry.file_name()))?
            .to_string();
        if markers.contains(&"*") || markers.contains(&name.as_str()) {
            trace!("match found {}", path);
            include_this_path = true;
            if include_entry.stop_on_match.unwrap_or(config.stop_on_match) {
                return Ok(include_this_path);
            } else {
                break;
            }
        }
    }

    if depth >= include_entry.depth {
        return Ok(include_this_path);
    }

    // read child dirs
    let mut children = vec![];
    for entry in dir_contents.iter() {
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| anyhow!("entry is not utf8 string: {:#?}", entry.file_name()))?
            .to_string();
        let ignore_chain = include_entry
            .ignore
            .iter()
            .chain(if include_entry.use_root_ignore {
                config.ignore.iter()
            } else {
                [].iter()
            });
        if is_dir(entry)?
            && (include_entry.show_hidden.unwrap_or(config.traverse_hidden) || !is_dot_dir(&name))
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
    Ok(include_this_path)
}

pub(crate) fn is_in_ignore(
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

pub(crate) fn is_dir(entry: &DirEntry) -> Result<bool, std::io::Error> {
    Ok(entry.file_type()?.is_dir())
}

pub(crate) fn is_dot_dir(name: &str) -> bool {
    name.starts_with('.')
}
