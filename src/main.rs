use clap::{Arg, ArgAction};
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
    #[serde(default)]
    exclude: Vec<&'a str>,
    #[serde(default)]
    depth: u8,
    #[serde(default)]
    include_all: bool,
    #[serde(default)]
    markers: Vec<&'a str>,
}

impl<'a> Default for ConfigEntry<'a> {
    fn default() -> Self {
        ConfigEntry {
            include: vec![],
            exclude: vec![],
            depth: 0,
            include_all: false,
            markers: vec![],
        }
    }
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

static APP_NAME: &str = "tmux-repoizer";
static CONFIG_PATH: &str = "/home/yev/.config/tmux-repoizer/config.json";

static MARKERS: [&str; 2] = [".git", "Cargo.toml"];
static IGNORE_DIRS: [&str; 11] = [
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

fn main() {
    // parse cli args
    let cmd = clap::Command::new(APP_NAME).arg(
        Arg::new("config")
            .short('c')
            .long("config")
            .action(ArgAction::Set)
            .default_value(CONFIG_PATH)
            .value_name("FILE")
            .help("location of config.json"),
    );
    let matches = cmd.get_matches();
    // parse config
    let p: &String = matches.get_one::<String>("config").expect("no config");
    let file = read_to_string(p).expect("error reading config.json");
    let config: Vec<ConfigEntry> =
        serde_json::from_str(file.as_str()).expect("error parsing config.json");
    // println!("{:?}", config);
    // println!("{}", expand("${HOME}/${HOME}/dd"));
    let mut output_vec = vec![];
    for mut config_entry in config {
        config_entry.exclude.extend(&IGNORE_DIRS);
        config_entry.markers.extend(&MARKERS);
        for path in &config_entry.include {
            descend(expand(path).as_str(), 1, &mut output_vec, &config_entry);
        }
    }
    let mut out = stdout();
    let mut output: String = "".to_string();
    // println!("{:?}", repos);
    for r in output_vec {
        output += (r + "\n").as_str();
    }
    out.write_fmt(format_args!("{}", output))
        .expect("error writing to stdout");
}

fn descend(path: &str, depth: u8, output: &mut Vec<String>, config: &ConfigEntry) -> bool {
    if config.depth != 0 && depth > config.depth {
        return false;
    }
    let mut include_this_path = depth == 1 || config.include_all;
    for marker in &config.markers {
        if let Ok(_) = std::fs::metadata(path.to_string() + "/" + marker) {
            // println!("is git {}", path);
            output.push(path.to_string());
            return true;
        }
    }
    if let Ok(mut iter) = read_dir(path) {
        let mut next = iter.next();
        let mut children = vec![];
        while let Some(Ok(ref dir_entry)) = next {
            let name = dir_entry
                .file_name()
                .to_str()
                .expect("not utf8 string")
                .to_string();
            if is_valid_dir(&dir_entry, &name, &config.exclude) {
                // descend further
                children.push(String::from(dir_entry.path().to_str().expect("path err")));
            }
            next = iter.next();
        }
        for child in children {
            if descend(child.as_str(), depth + 1, output, config) {
                include_this_path = true;
            };
        }
        if include_this_path {
            output.push(path.to_string());
        }
        // also include parent
        return include_this_path;
    } else {
        return false;
    }
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

#[allow(dead_code)]
fn measure<F>(name: &str, mut f: F)
where
    F: FnMut(),
{
    let start = Instant::now();
    f();
    println!("Time elapsed for {} is: {:?}", name, start.elapsed());
}
