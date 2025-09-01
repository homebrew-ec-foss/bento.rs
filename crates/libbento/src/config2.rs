// This file is trying to mimic the config.json file and call the seccomp module.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{fs::File, io::Read, path::PathBuf};

#[derive(Debug, Deserialize)]
pub struct SeccompConfig {
    #[serde(rename = "defaultAction")]
    pub default_action: String, // this is for unspecified syscalls
    pub architectures: Vec<String>,
    pub syscalls: Vec<SyscallRule>,
}

#[derive(Debug, Deserialize)]
pub struct SyscallRule {
    pub names: Vec<String>,
    pub action: String, // like Allow and Kill actions
}

fn get_path(container_id: &str) -> PathBuf {
    PathBuf::from(format!("/run/container/{container_id}/config.json"))
}

pub fn load_config(container_id: &str) -> Result<SeccompConfig> {
    let config_path = get_path(container_id);
    let mut file = File::open(&config_path)
        .with_context(|| format!("Failed to open config file at {}", config_path.display()))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("Failed to read config file at {}", config_path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("Failed to parse config file at {}", config_path.display()))
}
