use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use std::{
    collections::HashMap,
    fs::{self, File},
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
    pub hostname: Option<String>,

    #[serde(default)]
    pub linux: Option<Linux>,
    #[serde(default)]
    pub runtime: Option<Runtime>,

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
    #[serde(default)]
    pub upperdir: Option<PathBuf>,
    #[serde(default)]
    pub workdir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct Process {
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub no_new_privileges: bool,
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
    #[serde(default)]
    pub namespaces: Vec<Namespace>,
    #[serde(default)]
    pub resources: Option<Resources>,
    #[serde(default)]
    pub network: Option<Network>,
}

#[derive(Debug, Deserialize)]
pub struct IDMap {
    pub container_id: u32,
    pub host_id: u32,
    pub size: u32,
}

#[derive(Debug, Deserialize)]
pub struct Namespace {
    #[serde(rename = "type")]
    pub ns_type: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct Resources {
    #[serde(default)]
    pub memory: Option<Memory>,
    #[serde(default)]
    pub cpu: Option<Cpu>,
}

#[derive(Debug, Deserialize)]
pub struct Memory {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct Cpu {
    #[serde(default)]
    pub shares: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct Network {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub ports: Vec<PortMapping>,
}

#[derive(Debug, Deserialize)]
pub struct PortMapping {
    pub container_port: u16,
    pub host_port: u16,
    #[serde(default)]
    pub protocol: Option<String>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Runtime {
    #[serde(default = "Runtime::default_pivot_root")]
    pub pivot_root: bool,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub pid_file: Option<PathBuf>,
}

impl Runtime { fn default_pivot_root() -> bool { true } }

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let file   = File::open(&path)?;
        let reader = BufReader::new(file);
        let mut cfg: Self = serde_json::from_reader(reader)?;
        cfg.validate(&path)?;
        Ok(cfg)
    }

    fn validate<P: AsRef<Path>>(&mut self, cfg_path: P) -> Result<(), ConfigError> {
        self.validate_version()?;
        self.resolve_root_rel_to_bundle(&cfg_path)?;
        self.validate_rootfs()?;
        self.validate_rootless()?;
        self.validate_namespaces()?;
        self.validate_mounts()?;
        self.validate_overlay()?;
        self.validate_pid_file()?;
        self.validate_network_mode()?;
        Ok(())
    }

    fn validate_version(&self) -> Result<(), ConfigError> {
        if !self.oci_version.starts_with("1.") {
            Err(ConfigError::Invalid(format!(
                "unsupported ociVersion {}", self.oci_version
            )))
        } else { Ok(()) }
    }

    fn resolve_root_rel_to_bundle<P: AsRef<Path>>(
        &mut self,
        config_path: P,
    ) -> Result<(), ConfigError> {
        if self.root.path.is_absolute() { return Ok(()) }
        let abs = config_path.as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&self.root.path);
        self.root.path = abs.canonicalize()?;
        Ok(())
    }

    fn validate_rootfs(&self) -> Result<(), ConfigError> {
        if !self.root.path.is_dir() {
            Err(ConfigError::Invalid(format!(
                "root.path {:?} is not a directory", self.root.path
            )))
        } else { Ok(()) }
    }

    fn validate_rootless(&self) -> Result<(), ConfigError> {
        if Uid::effective().is_root() { return Ok(()) }
        let linux = self.linux.as_ref().ok_or_else(|| {
            ConfigError::Invalid("linux section required for rootless".into())
        })?;
        let uid_ok = linux.uid_mappings.iter().any(|m| m.container_id == 0);
        let gid_ok = linux.gid_mappings.iter().any(|m| m.container_id == 0);
        if uid_ok && gid_ok { Ok(()) } else {
            Err(ConfigError::Invalid("uid/gid mappings missing for containerId 0".into()))
        }
    }

    fn validate_namespaces(&self) -> Result<(), ConfigError> {
        if let Some(linux) = &self.linux {
            let kinds: Vec<&str> = linux.namespaces.iter().map(|n| n.ns_type.as_str()).collect();
            if !kinds.contains(&"pid") || !kinds.contains(&"mount") {
                return Err(ConfigError::Invalid(
                    "namespaces must include at least pid & mount".into()));
            }
            if self.hostname.is_some() && !kinds.contains(&"uts") {
                return Err(ConfigError::Invalid(
                    "hostname set but uts namespace not requested".into()));
            }
        }
        Ok(())
    }

    fn validate_mounts(&self) -> Result<(), ConfigError> {
        for m in &self.mounts {
            match m.fs_type.as_str() {
                "proc" if m.destination != Path::new("/proc") =>
                    return Err(ConfigError::Invalid(
                        format!("proc mount must be at /proc, got {:?}", m.destination))),
                "sysfs" if m.destination != Path::new("/sys") =>
                    return Err(ConfigError::Invalid(
                        format!("sysfs mount must be at /sys, got {:?}", m.destination))),
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_overlay(&self) -> Result<(), ConfigError> {
        let overlay_requested = self.root.upperdir.is_some() || self.root.workdir.is_some();
        if !overlay_requested { return Ok(()) }
        let root_mount = self.mounts.iter()
            .find(|m| m.destination == Path::new("/"));
        if let Some(m) = root_mount {
            if m.fs_type != "overlay" {
                return Err(ConfigError::Invalid(
                    "upperdir/workdir provided but / mount is not overlay".into()));
            }
        } else {
            return Err(ConfigError::Invalid(
                "overlayfs requested but no mount with destination /".into()));
        }
        Ok(())
    }

    fn validate_pid_file(&self) -> Result<(), ConfigError> {
        if let Some(rt) = &self.runtime {
            if let Some(pid) = &rt.pid_file {
                let parent = pid.parent().ok_or_else(|| ConfigError::Invalid(
                    format!("pid_file {:?} has no parent directory", pid)))?;
                if !parent.is_dir() {
                    return Err(ConfigError::Invalid(
                        format!("pid_file parent {:?} does not exist", parent)));
                }
                if fs::OpenOptions::new().write(true).create(true).open(pid).is_err() {
                    return Err(ConfigError::Invalid(
                        format!("cannot write pid_file {:?}", pid)));
                }
            }
        }
        Ok(())
    }

    fn validate_network_mode(&self) -> Result<(), ConfigError> {
        const ALLOWED: &[&str] = &["slirp", "bridge", "host", "none"];
        if let Some(linux) = &self.linux {
            if let Some(net) = &linux.network {
                if let Some(mode) = &net.mode {
                    if !ALLOWED.contains(&mode.as_str()) {
                        return Err(ConfigError::Invalid(format!(
                            "unsupported network mode {:?}", mode)));
                    }
                }
            }
        }
        Ok(())
    }
}
