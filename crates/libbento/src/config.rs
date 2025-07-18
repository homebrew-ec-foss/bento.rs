use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};
use thiserror::Error;
use nix::unistd::Uid;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub oci_version: String,
    pub root: Root,
    #[serde(default)]
    pub process: Option<Process>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    #[serde(default)]
    pub linux: Option<Linux>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Root {
    #[serde_as(as = "DisplayFromStr")]
    pub path: PathBuf,
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Debug, Deserialize)]
pub struct Process {
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    pub cwd: String,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Mount {
    #[serde_as(as = "DisplayFromStr")]
    pub destination: PathBuf,
    #[serde(rename = "type")]
    pub fs_type: String,
    pub source: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Linux {
    #[serde(default)]
    pub uid_mappings: Vec<IDMap>,
    #[serde(default)]
    pub gid_mappings: Vec<IDMap>,
}

#[derive(Debug, Deserialize)]
pub struct IDMap {
    pub container_id: u32,
    pub host_id: u32,
    pub size: u32,
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let cfg: Self = serde_json::from_reader(reader)?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // Check version
        if !self.oci_version.starts_with("1.") {
            return Err(ConfigError::Invalid(format!(
                "unsupported ociVersion {0}",
                self.oci_version
            )));
        }

        // Check if rootfs exists
        if !self.root.path.is_dir() {
            return Err(ConfigError::Invalid(format!(
                "root.path {:?} is not a directory",
                self.root.path
            )));
        }

        // check if rootless and had gid uid mappings
        let rootless = !Uid::effective().is_root();	
        if rootless {
            let linux = self.linux.as_ref().ok_or_else(|| {
                ConfigError::Invalid("linux section required for rootless".into())
            })?;
            if linux.uid_mappings.is_empty() || linux.gid_mappings.is_empty() {
                return Err(ConfigError::Invalid(
                    "uid/gid mappings missing in rootless mode".into(),
                ));
            }
        }

        Ok(())
    }
}
