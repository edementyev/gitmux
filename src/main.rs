use regex::{Captures, Regex};
use serde::Deserialize;
use std::io::{stdout, Write};
use std::{
    ffi::OsStr,
    fs::{read_dir, read_to_string, DirEntry},
    time::Instant,
};

#[derive(Deserialize, Debug)]
struct ConfigEntry<'a> {
    #[serde(borrow = "'a")]
    include: Vec<&'a str>,
    exclude: Vec<&'a str>,
}

use std::env;

fn expand(path: &str) -> String {
    let re = Regex::new(r"\$\{?([^\}\/]+)\}?").expect("invalid regex");
    let result: String = re
        .replace_all(path, |captures: &Captures| match &captures[1] {
            "" => "".to_string(),
            varname => env::var(OsStr::new(varname)).expect("no such var"),
        })
        .into();
    result
}

use clap::*;

static APP_NAME: &str = "tmux-repoizer";
static CONFIG_PATH: &str = "/home/yev/.config/tmux-repoizer/config.json";

fn main() {
    // parse cli args
    let cmd = clap::Command::new(APP_NAME).arg(
        Arg::new("config")
            .short('c')
            .long("config")
            .action(ArgAction::Set)
            .default_value(CONFIG_PATH)
            .value_name("FILE")
            .help("Provides a config file to myprog"),
    );
    let matches = cmd.get_matches();
    // parse config
    let p: &String = matches.get_one::<String>("config").expect("no config");
    let file = read_to_string(p).expect("error reading config.json");
    let config: Vec<ConfigEntry> =
        serde_json::from_str(file.as_str()).expect("error parsing config.json");
    // println!("{:?}", config);
    // println!("{}", expand("${HOME}/${HOME}/dd"));
    let mut repos = vec![];
    let ignore_dirs: Vec<&str> = vec![
        "node_modules",
        "venv",
        "bin",
        "target",
        "debug",
        "src",
        "test",
        "tests",
        "lib",
        "docs",
        "pkg",
    ];
    let max_depth = 0;
    for entry in config {
        let local_ignore = &mut ignore_dirs.clone();
        local_ignore.extend(entry.exclude.into_iter());
        for path in entry.include {
            fill_and_descend(
                expand(path).as_str(),
                0,
                max_depth,
                &mut repos,
                &local_ignore,
            );
        }
    }
    let mut out = stdout();
    let mut output: String = "".to_string();
    // println!("{:?}", repos);
    for r in repos {
        output += (r + "\n").as_str();
    }
    out.write_fmt(format_args!("{}", output))
        .expect("error writing to stdout");
}

#[allow(dead_code)]
fn measure<F>(name: &str, mut f: F)
where
    F: FnMut(),
{
    let start = Instant::now();
    f();
    println!("Time elapsed for {} is: {:?}", name, start.elapsed());
}

fn is_valid_dir(dir_entry: &DirEntry, name: &str, ignore_dirs: &Vec<&str>) -> bool {
    // is dir
    if !dir_entry.file_type().expect("err on file_type").is_dir() {
        return false;
    }
    // not a dot dir
    if let Some(ch) = name.chars().next() {
        if ch == '.' {
            return false;
        }
    }
    for ignore_pat in ignore_dirs {
        if *ignore_pat == name {
            return false;
        }
    }
    true
}

fn fill_and_descend(
    path: &str,
    depth: u32,
    max_depth: u32,
    repos: &mut Vec<String>,
    ignore_dirs: &Vec<&str>,
) -> bool {
    if max_depth != 0 && depth >= max_depth {
        return false;
    }
    let mut include_parent = false;
    let mut is_git = false;
    match read_dir(path) {
        Ok(mut iter) => {
            let mut next = iter.next();
            let mut children = vec![];
            while let Some(Ok(ref dir_entry)) = next {
                let name = dir_entry
                    .file_name()
                    .to_str()
                    .expect("not utf8 string")
                    .to_string();
                let mut str_path = String::from(dir_entry.path().to_str().expect("path err"));
                if name == ".git" {
                    // println!("got git! {}", str_path);
                    include_parent = true;
                    is_git = true;
                    str_path.truncate(str_path.len() - 5);
                    repos.push(str_path.to_string());
                    break;
                } else {
                    if is_valid_dir(&dir_entry, &name, &ignore_dirs) {
                        // descend further
                        children.push(str_path);
                    }
                };
                next = iter.next();
            }
            if !is_git {
                for child in children {
                    if fill_and_descend(child.as_str(), depth + 1, max_depth, repos, ignore_dirs) {
                        include_parent = true;
                    };
                }
            }
            if include_parent && !is_git {
                repos.push(path.to_string());
            }
            return include_parent;
        }
        Err(_) => {
            return false;
        }
    }
}
