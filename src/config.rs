use anyhow::Result;
use log::warn;
use serde::Deserialize;
use std::{collections::HashMap, fs::File, io::Read, path::Path};

#[derive(Deserialize, Clone)]
pub struct GeneralConfig {
    pub db_pgconn: String,
    pub change_notifier: Option<String>,
    pub path: String,
    pub discover: bool,
    pub origin: String,
    pub ttl: u64,
    pub label: String,
    pub codename: String,
    certificate: Option<String>,
    pub abbs_sync: bool,
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

#[derive(Clone)]
pub struct ReleaseConfig {
    pub origin: String,
    pub label: String,
    pub codename: String,
    pub descriptions: HashMap<String, String>,
    pub cert: Option<String>,
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
        cert: default.certificate.clone(),
    }
}

pub fn lint_config(config: &Config) {
    if config.config.discover && !config.branch.is_empty() {
        warn!("Specifying any branch when auto-discover is enabled will only get their descriptions read.");
    }
    if config.config.abbs_sync {
        warn!("ABBS sync is deprecated and will be removed in a future version. Please remove the option.");
    }
}

pub fn parse_config<P: AsRef<Path>>(path: P) -> Result<Config> {
    let mut f = File::open(path)?;
    let mut content = String::new();
    content.reserve(1024);
    f.read_to_string(&mut content)?;

    Ok(toml::from_str(&content)?)
}
