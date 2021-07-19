use anyhow::Result;
use log::warn;
use serde::Deserialize;
use std::{collections::HashMap, fs::File, io::Read, path::Path};

#[derive(Deserialize, Clone)]
pub struct GeneralConfig {
    pub db_pgconn: String,
    pub path: String,
    pub discover: bool,
    pub origin: String,
    pub ttl: u64,
    pub label: String,
    pub codename: String,
}

#[derive(Deserialize, Clone)]
pub struct BranchConfig {
    pub name: String,
    #[serde(rename = "desc")]
    pub description: String,
    pub ttl: Option<u64>,
}

#[derive(Deserialize, Clone)]
pub struct Config {
    pub config: GeneralConfig,
    pub branch: Vec<BranchConfig>,
}

pub struct BranchRefreshConfig {
    pub default: u64,
    pub branch: HashMap<String, u64>,
}

#[derive(Clone)]
pub struct ReleaseConfig {
    // TODO: add cert info
    pub origin: String,
    pub label: String,
    pub codename: String,
    pub descriptions: HashMap<String, String>,
}

pub fn convert_branch_description_config(config: &Config) -> ReleaseConfig {
    let mut branch = HashMap::new();
    for b in &config.branch {
        branch.insert(b.name.clone(), b.description.clone());
    }
    let default = &config.config;

    ReleaseConfig {
        descriptions: branch,
        label: default.label.clone(),
        origin: default.origin.clone(),
        codename: default.codename.clone(),
    }
}

pub fn convert_branch_refresh_config(config: &Config) -> BranchRefreshConfig {
    let mut branch = HashMap::new();
    for b in &config.branch {
        if let Some(ttl) = b.ttl {
            branch.insert(b.name.clone(), ttl);
        }
    }

    BranchRefreshConfig {
        default: config.config.ttl,
        branch,
    }
}

pub fn lint_config(config: &Config) {
    if config.config.discover && !config.branch.is_empty() {
        warn!("Specifying any branch when auto-discover is enabled does not make sense.");
    }
}

pub fn parse_config<P: AsRef<Path>>(path: P) -> Result<Config> {
    let mut f = File::open(path)?;
    let mut content = Vec::new();
    content.reserve(1024);
    f.read_to_end(&mut content)?;

    Ok(toml::from_slice(&content)?)
}
