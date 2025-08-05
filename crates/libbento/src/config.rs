use anyhow::Result;
use std::{env, path::PathBuf};

pub struct Config {
    pub root_path: String,
    pub args: Vec<String>,
    pub hostname: String,
    pub rootless: bool,
    pub bundle_path: String,
    pub container_id: String,
}

impl Config {
    pub fn new_config() -> Result<Self> {
        let container_id = "default".to_string();
        let rootfs_path = Self::get_rootfs_path()?;

        Ok(Self {
            root_path: rootfs_path.to_string_lossy().to_string(),
            args: vec!["/bin/cat".to_string(), "/proc/self/stat".to_string()],
            hostname: "bento-container".to_string(),
            rootless: true,
            bundle_path: ".".to_string(),
            container_id,
        })
    }

    pub fn get_rootfs_path() -> Result<PathBuf> {
        let home = env::var("HOME")?;
        Ok(PathBuf::from(format!("{}/.local/share/bento", home)))
    }
}
