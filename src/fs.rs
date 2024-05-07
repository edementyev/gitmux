use crate::config::{Config, IncludeEntry};
use crate::Error;

use anyhow::anyhow;
use log::{error, trace};
use regex::{Captures, Regex, RegexSet};

use std::collections::HashMap;
use std::env::{self, VarError};
use std::ffi::OsStr;
use std::fs::DirEntry;
use std::fs::{self, FileType};
use std::path::PathBuf;

const EMPTY_STR: &str = "";

/// tries to expand env variables in string
pub(crate) fn expand(path: &str) -> Result<String, Error> {
    let re = Regex::new(r"\$\{?([^\}/]+)\}?")?;
    let mut errors: Vec<(VarError, String)> = Vec::new();
    let result: String = re
        .replace_all(path, |captures: &Captures| match &captures[1] {
            EMPTY_STR => EMPTY_STR.to_string(),
            varname => env::var(OsStr::new(varname))
                .map_err(|e| {
                    errors.push((e.clone(), varname.to_owned()));
                    e
                })
                .unwrap_or_default(),
        })
        .into();
    if let Some(error_tuple) = errors.last() {
        return Err(Error::EnvVar(error_tuple.0.clone(), error_tuple.1.clone()));
    }
    Ok(result)
}

/// retains the tail of the path
pub(crate) fn trim_window_name(path: &str) -> Result<String, anyhow::Error> {
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

/// removes all dots from original name string
/// (needed because dots are displayed as underscores in session name for some reason)
pub(crate) fn trim_session_name(name: &String) -> String {
    let mut s = String::from(name);
    s.retain(|x| x != '.');
    s
}

/// receives path, mutable list and config
/// updates list with entries from the path tree that should be included
/// on intermediate steps, returns path_yields (indicates that this path yielded matches)
/// intermediate paths are included if include_intermediate_paths = true
pub(crate) fn get_included_paths_list(
    path: &str,
    depth: u8,
    output: &mut HashMap<String, ()>,
    include_entry: &IncludeEntry,
    config: &Config,
) -> Result<bool, Error> {
    let mut path_yields = false;

    // read current path contents
    let read_dir = match std::fs::read_dir(path) {
        Ok(read) => read,
        Err(err) => {
            trace!("Error reading dir {}: {:#?}", path, err);
            return Ok(false);
        }
    };
    let dir_contents = read_dir.flatten().collect::<Vec<DirEntry>>();

    // build markers lists
    let markers_exact_chain =
        include_entry
            .markers
            .exact
            .iter()
            .chain(if include_entry.markers.chain_root_markers {
                config.markers.exact.iter()
            } else {
                [].iter()
            });
    let markers_exact = markers_exact_chain.copied().collect::<Vec<&str>>();
    let markers_pattern_chain =
        include_entry
            .markers
            .pattern
            .iter()
            .chain(if include_entry.markers.chain_root_markers {
                config.markers.pattern.iter()
            } else {
                [].iter()
            });
    let markers_pattern = markers_pattern_chain.copied().collect::<Vec<&str>>();
    let markers_regex_set = RegexSet::new(markers_pattern)?;

    // do the thing according to chosen mode
    match include_entry.mode {
        crate::config::Mode::Dir => {
            // 1 - scan dir for markers
            // found marker -> include this dir in output (if yield_on_marker = true, this is the end of current path's branch)
            // reached max depth (depth = number of steps) -> return
            // 2 - start traversing its children

            // search current dir for markers
            for entry in dir_contents.iter() {
                let name = entry
                    .file_name()
                    .to_str()
                    .ok_or_else(|| anyhow!("entry is not utf8 string: {:#?}", entry.file_name()))?
                    .to_string();
                if markers_exact.contains(&name.as_str())
                    || markers_regex_set.matches(name.as_str()).len() > 0
                {
                    trace!("match found {}", path);
                    // yield_on_marker stops descending further down the fs tree
                    path_yields = true;
                    if include_entry.yield_on_marker {
                        output.insert(path.to_string(), ());
                        return Ok(path_yields);
                    }
                    break;
                }
            }

            if depth >= include_entry.depth {
                if path_yields {
                    output.insert(path.to_string(), ());
                }
                // reached maximum depth -> return
                return Ok(path_yields);
            }

            let mut children = vec![];

            let entries = get_not_ignored_dir_entries(include_entry, dir_contents, config)?;
            for (path, ft) in entries {
                // entry is a dir and is not ignored
                if is_dir(&path, &ft)? {
                    // -> add it to the list of children to traverse on next step
                    children.push(path);
                }
            }

            // walk current dir's children
            for child in children {
                // if child yields matches
                if get_included_paths_list(&child, depth + 1, output, include_entry, config)? {
                    path_yields = true;
                };
            }

            // if path yields matches and we include every step of the final match, include this path
            if path_yields && include_entry.include_intermediate_paths {
                output.insert(path.to_string(), ());
            }

            Ok(path_yields)
        }
        crate::config::Mode::File => {
            // iterate through dir contents
            // add all unignored files
            // collect directories
            let mut children = vec![];

            let entries = get_not_ignored_dir_entries(include_entry, dir_contents, config)?;
            for (path, ft) in entries {
                // entry is a dir and is not ignored
                if is_dir(&path, &ft)? {
                    // -> add it to the list of children to traverse on next step
                    children.push(path);
                // entry is a file and include_files flag is on
                } else if is_file(&path, &ft)? {
                    // -> add file to the list of included paths
                    path_yields = true;
                    output.insert(path, ());
                }
            }

            // reached maximum depth -> return
            if depth >= include_entry.depth {
                return Ok(path_yields);
            }

            // walk current dir's children
            for child in children {
                // if child yields matches
                if get_included_paths_list(&child, depth + 1, output, include_entry, config)? {
                    path_yields = true;
                };
            }

            // if path yields matches and we include every step of the final match, include this path
            if path_yields && include_entry.include_intermediate_paths {
                output.insert(path.to_string(), ());
            }

            Ok(path_yields)
        }
    }
}

fn get_not_ignored_dir_entries(
    include_entry: &IncludeEntry,
    dir_contents: Vec<DirEntry>,
    config: &Config,
) -> Result<Vec<(String, FileType)>, Error> {
    // build ignore lists
    let ignore_exact_chain =
        include_entry
            .ignore
            .exact
            .iter()
            .chain(if include_entry.ignore.chain_root_ignore {
                config.ignore.exact.iter()
            } else {
                [].iter()
            });
    let ignore_exact = ignore_exact_chain.copied().collect::<Vec<&str>>();
    let ignore_pattern_chain =
        include_entry
            .ignore
            .pattern
            .iter()
            .chain(if include_entry.ignore.chain_root_ignore {
                config.ignore.pattern.iter()
            } else {
                [].iter()
            });
    let ignore_pattern = ignore_pattern_chain.copied().collect::<Vec<&str>>();
    let ignore_regex_set = RegexSet::new(ignore_pattern)?;

    let mut result: Vec<(String, FileType)> = vec![];
    // iterate through dir contents
    for entry in dir_contents.iter() {
        // get entry(dir/file) name
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| anyhow!("entry is not utf8 string: {:#?}", entry.file_name()))?
            .to_string();
        // check if entry should be ignored
        // name is not dotfile/dir or we accept dotfiles/dirs
        if (!name.starts_with('.') || include_entry.markers.traverse_hidden)
            // name is not in ignore_exact list
            && !ignore_exact.contains(&name.as_str())
            // name does not match any ignore_pattern
            && ignore_regex_set.matches(&name).len() == 0
        {
            // get path
            let path = match get_path_string(entry) {
                Ok(p) => p,
                Err(err) => {
                    error!("error getting path: {:#?}", err);
                    continue;
                }
            };
            // get filetype
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(err) => {
                    error!("error getting filetype: {:#?}", err);
                    continue;
                }
            };
            result.push((path, ft))
        }
    }
    Ok(result)
}

fn get_path_string(entry: &DirEntry) -> Result<String, anyhow::Error> {
    Ok(String::from(entry.path().to_str().ok_or_else(|| {
        anyhow!("entry.path() is not valid utf8: {:#?}", entry.path())
    })?))
}

pub(crate) fn is_dir(path: &str, ft: &FileType) -> Result<bool, std::io::Error> {
    if ft.is_symlink() {
        // read link and read its ft
        Ok(read_link(path)
            .as_deref()
            .map(std::path::Path::is_dir)
            .unwrap_or(false))
    } else {
        Ok(ft.is_dir())
    }
}

pub(crate) fn is_file(path: &str, ft: &FileType) -> Result<bool, std::io::Error> {
    if ft.is_symlink() {
        // read link and read its ft
        Ok(read_link(path)
            .as_deref()
            .map(std::path::Path::is_file)
            .unwrap_or(false))
    } else {
        Ok(ft.is_file())
    }
}

// readlink and convert result to option, dropping error
fn read_link(path: &str) -> Option<std::path::PathBuf> {
    match fs::read_link(PathBuf::from(path)) {
        Ok(rl) => Some(rl),
        Err(err) => {
            error!("error reading link: {:#?}", err);
            None
        }
    }
}

pub(crate) fn path_is_file(path: &str) -> bool {
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
