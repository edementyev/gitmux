use crate::config::{Config, IncludeEntry};
use crate::Error;

use anyhow::anyhow;
use log::{error, trace};
use regex::{Captures, Regex};

use std::env::{self, VarError};
use std::ffi::OsStr;
use std::fs::DirEntry;
use std::fs::{self, FileType};
use std::iter::Chain;

const EMPTY_STR: &str = "";

/// tries to expand env variables in path
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

/// tries to expand env variables in path
pub(crate) fn expand_path(path: &str) -> Result<String, Error> {
    let re = Regex::new(r"\$\{?([^\}/]+)\}?")?;
    let mut errors: Vec<(VarError, String)> = Vec::new();
    let result: String = re
        .replace_all(path, |captures: &Captures| match &captures[1] {
            EMPTY_STR => EMPTY_STR.to_string(),
            varname => env::var(OsStr::new(varname))
                .map_err(|e| errors.push((e, varname.to_owned())))
                // TODO: check that this variable expands to some valid path
                .unwrap_or_default(),
        })
        .into();
    if let Some(error_tuple) = errors.pop() {
        return Err(Error::EnvVar(error_tuple.0, error_tuple.1));
    }
    Ok(result)
}

/// retains the tail of the path
pub(crate) fn trim_pane_name(path: &str) -> Result<String, anyhow::Error> {
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

/// removes all dots from original string
/// (dots are displayed as underscores in session name for some reason)
pub(crate) fn trim_session_name(pane_name: &String) -> String {
    let mut s = String::from(pane_name);
    s.retain(|x| x != '.');
    s
}

/// receives path, mutable list and config
/// updates list with entries from the path tree that should be included
/// on intermediate steps, returns include_this_path (whether to include current path in result list)
/// if any child path is included in result list, parent path (../) will also be included
pub(crate) fn get_included_paths_list(
    path: &str,
    depth: u8,
    output: &mut Vec<String>,
    include_entry: &IncludeEntry,
    config: &Config,
) -> Result<bool, Error> {
    let mut include_this_path = false;
    let read_dir = match std::fs::read_dir(path) {
        Ok(read) => read,
        Err(err) => {
            trace!("Error reading dir {}: {:#?}", path, err);
            return Ok(false);
        }
    };
    let dir_contents = read_dir.flatten().collect::<Vec<DirEntry>>();
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
            // stop_on_marker prevents from descending further than current directory
            if include_entry.stop_on_marker.unwrap_or(config.stop_on_marker) {
                return Ok(include_this_path);
            }
            break;
        }
    }

    // reached maximum depth and do not need to include files?
    if depth >= include_entry.depth && !include_entry.include_files {
        return Ok(include_this_path);
    }

    // read dir contents
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
        if (include_entry.show_hidden.unwrap_or(config.traverse_hidden) || !start_with_dot(&name))
            && !is_in_ignore(&name, ignore_chain.clone())
        {
            let path = match get_path_string(entry) {
                Ok(p) => p,
                Err(err) => {
                    error!("error getting path: {:#?}", err);
                    continue;
                }
            };
            let ft = &(match entry.file_type() {
                Ok(ft) => ft,
                Err(err) => {
                    error!("error getting filetype: {:#?}", err);
                    continue;
                }
            });
            // entry is a file
            // and include_files flag is on
            if is_file(entry, ft)? && include_entry.include_files {
                // add file to the list of included paths
                output.push(path);
            // and entry is a dir
            // and is not ignored
            } else if is_dir(entry, ft)? {
                // add to list of children to traverse next
                children.push(path);
            }
        }
    }

    // reached maximum depth (after possibly including files)
    if depth >= include_entry.depth {
        return Ok(include_this_path);
    }

    // walk current dir's children
    for child in children {
        if get_included_paths_list(&child, depth + 1, output, include_entry, config)? {
            output.push(child);
            // if child is included, also include parent
            include_this_path = true;
        };
    }
    // current entry's parent will be included also
    Ok(include_this_path)
}

fn get_path_string(entry: &DirEntry) -> Result<String, anyhow::Error> {
    Ok(String::from(entry.path().to_str().ok_or_else(|| {
        anyhow!("entry.path() is not valid utf8: {:#?}", entry.path())
    })?))
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

pub(crate) fn is_dir(entry: &DirEntry, ft: &FileType) -> Result<bool, std::io::Error> {
    if ft.is_symlink() {
        // read link and read its ft
        Ok(read_link(entry)
            .as_deref()
            .map(std::path::Path::is_dir)
            .unwrap_or(false))
    } else {
        Ok(ft.is_dir())
    }
}

pub(crate) fn is_file(entry: &DirEntry, ft: &FileType) -> Result<bool, std::io::Error> {
    if ft.is_symlink() {
        // read link and read its ft
        Ok(read_link(entry)
            .as_deref()
            .map(std::path::Path::is_file)
            .unwrap_or(false))
    } else {
        Ok(ft.is_file())
    }
}

// readlink and convert result to option, dropping error
fn read_link(entry: &DirEntry) -> Option<std::path::PathBuf> {
    match fs::read_link(entry.path().as_path()) {
        Ok(rl) => Some(rl),
        Err(err) => {
            error!("error reading link: {:#?}", err);
            None
        }
    }
}

pub(crate) fn start_with_dot(name: &str) -> bool {
    name.starts_with('.')
}

pub(crate) fn is_file_str(path: &str) -> bool {
    let meta = std::fs::metadata(path);
    match meta {
        Ok(meta) => meta.is_file(),
        Err(err) => {
            error!("error reading metadata of path {}: {}", path, err);
            // if getting metadata failed (e.g. due to insufficient rights), treat as dir
            false
        }
    }
}
