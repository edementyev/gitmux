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
    for mut entry in config {
        entry.exclude.extend(&ignore_dirs);
        for path in entry.include {
            search_git(
                expand(path).as_str(),
                0,
                max_depth,
                &mut repos,
                &entry.exclude,
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

fn search_git(
    path: &str,
    depth: u32,
    max_depth: u32,
    output: &mut Vec<String>,
    ignore_dirs: &Vec<&str>,
) -> bool {
    if max_depth != 0 && depth >= max_depth {
        return false;
    }
    let mut include_parent = false;
    if let Ok(_) = std::fs::metadata(path.to_string() + "/.git") {
        // println!("is git {}", path);
        output.push(path.to_string());
        return true;
    } else if let Ok(mut iter) = read_dir(path) {
        let mut next = iter.next();
        let mut children = vec![];
        while let Some(Ok(ref dir_entry)) = next {
            let name = dir_entry
                .file_name()
                .to_str()
                .expect("not utf8 string")
                .to_string();
            if is_valid_dir(&dir_entry, &name, &ignore_dirs) {
                // descend further
                children.push(String::from(dir_entry.path().to_str().expect("path err")));
            }
            next = iter.next();
        }
        for child in children {
            if search_git(child.as_str(), depth + 1, max_depth, output, ignore_dirs) {
                include_parent = true;
            };
        }
        if include_parent {
            output.push(path.to_string());
        }
        return include_parent;
    } else {
        return false;
    }
}
