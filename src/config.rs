use serde::Deserialize;

#[derive(thiserror::Error, Debug)]
pub(crate) enum ConfigError {
    #[error("Parse config: {0}")]
    Parse(#[from] serde_jsonc::Error),
    #[error("Read config: {0}")]
    Read(#[from] std::io::Error),
}

#[derive(Deserialize, Debug)]
pub(crate) struct Config<'a> {
    #[serde(default)]
    pub sessions: Vec<Session<'a>>,
    #[serde(default, borrow = "'a")]
    pub markers: Markers<'a>,
    #[serde(default)]
    pub ignore: Ignore<'a>,
    pub include: Vec<IncludeEntry<'a>>,
}

impl<'a> Default for Config<'a> {
    fn default() -> Self {
        Self {
            sessions: vec![],
            markers: Markers::default(),
            ignore: Ignore::default(),
            include: vec![IncludeEntry {
                paths: ["$HOME"].to_vec(),
                ..Default::default()
            }],
        }
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct Session<'a> {
    pub name: &'a str,
    pub windows: Vec<&'a str>,
}

impl<'a> ToString for Session<'a> {
    fn to_string(&self) -> String {
        format!(
            "{}:\n{}\n",
            self.name,
            self.windows
                .iter()
                .map(|p| crate::fs::expand(p).unwrap_or(p.to_string()))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn default_yield_on_marker() -> bool {
    true
}

fn default_include_intermediate_paths() -> bool {
    true
}

#[derive(Deserialize, Debug)]
pub(crate) struct IncludeEntry<'a> {
    #[serde(borrow = "'a")]
    pub paths: Vec<&'a str>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub markers: Markers<'a>,
    #[serde(default)]
    pub ignore: Ignore<'a>,
    #[serde(default = "default_include_intermediate_paths")]
    pub include_intermediate_paths: bool,
    #[serde(default = "default_yield_on_marker")]
    pub yield_on_marker: bool,
    #[serde(default = "u8::max_value")]
    pub depth: u8,
}

impl<'a> Default for IncludeEntry<'a> {
    fn default() -> Self {
        Self {
            paths: vec![],
            mode: Mode::Dir,
            markers: Markers::default(),
            ignore: Ignore::default(),
            include_intermediate_paths: default_include_intermediate_paths(),
            yield_on_marker: default_yield_on_marker(),
            depth: u8::max_value(),
        }
    }
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
// #[serde(untagged)]
pub(crate) enum Mode {
    #[default]
    Dir,
    File,
}

const MARKERS_EXACT_DEFAULT: [&str; 3] = [
    ".git",
    "Cargo.toml",
    "go.mod",
    // "package.json",
    // "pom.xml",
    // "build.gradle",
];

const MARKERS_PATTERN_DEFAULT: [&str; 0] = [];

fn default_traverse_hidden() -> bool {
    true
}

fn default_chain_root_markers() -> bool {
    true
}

#[derive(Deserialize, Debug)]
pub(crate) struct Markers<'a> {
    #[serde(default, borrow = "'a")]
    pub exact: Vec<&'a str>,
    #[serde(default)]
    pub pattern: Vec<&'a str>,
    #[serde(default = "default_traverse_hidden")]
    pub traverse_hidden: bool,
    #[serde(default = "default_chain_root_markers")]
    pub chain_root_markers: bool,
}

impl<'a> Default for Markers<'a> {
    fn default() -> Self {
        Markers {
            exact: Vec::from(MARKERS_EXACT_DEFAULT),
            pattern: Vec::from(MARKERS_PATTERN_DEFAULT),
            chain_root_markers: default_chain_root_markers(),
            traverse_hidden: default_traverse_hidden(),
        }
    }
}

const IGNORE_EXACT_DEFAULT: [&str; 12] = [
    ".DS_Store",
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
const IGNORE_PATTERN_DEFAULT: [&str; 0] = [];

fn default_chain_root_ignore() -> bool {
    true
}

#[derive(Deserialize, Debug)]
pub(crate) struct Ignore<'a> {
    #[serde(default, borrow = "'a")]
    pub exact: Vec<&'a str>,
    #[serde(default)]
    pub pattern: Vec<&'a str>,
    #[serde(default = "default_chain_root_ignore")]
    pub chain_root_ignore: bool,
}

impl<'a> Default for Ignore<'a> {
    fn default() -> Self {
        Ignore {
            exact: Vec::from(IGNORE_EXACT_DEFAULT),
            pattern: Vec::from(IGNORE_PATTERN_DEFAULT),
            chain_root_ignore: default_chain_root_ignore(),
        }
    }
}

pub(crate) fn read_config(path: &str) -> Result<Config, ConfigError> {
    let contents = Box::leak(Box::new(std::fs::read_to_string(path)?));
    Ok(serde_jsonc::from_str(contents)?)
}
