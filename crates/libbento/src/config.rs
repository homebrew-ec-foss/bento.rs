use serde::Serialize;
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NamespaceType {
    Pid,
    Net,
    Mnt,
    Uts,
    Ipc,
    User,
    Cgroup,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawConfig {
   pub oci_version : String,
   pub root : Root,

   #[serde(default)]
   pub process: Option<Process>,

   #[serde(default)]
   pub mounts: Vec<Mount>,

   #[serde(default)]
   pub hostname: Option<String>,

   #[serde(default)]
   pub linux: Option<Linux>,

   #[serde(flatten)]
   pub extra: HashMap<String, serde_json::Value>, // This is helpful when unknown fields are received, without this runtime throws errors.
}

#[derive(Debug)]
pub struct Config {
   pub oci_version : OciVersion,
   pub root : Root,
   pub process : Option<Process>,
   pub  mounts : Vec<Mount>,
   pub hostname : Option<String>,
   pub linux : Linux, // it was Option<Linux>
   pub extra : HashMap<String, serde_json::Value>,
}

#[derive(Debug)]
pub struct OciVersion(#[allow(dead_code)]String);
// without adding this dead_code, just a warning for not using OciVersion.

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
}

#[derive(Debug, Deserialize, Clone)]
pub struct IDMap {
    pub container_id: u32,
    pub host_id: u32,
    pub size: u32,
}

#[derive(Debug, Deserialize)]
pub struct Namespace {
    #[serde(rename = "type")]
    pub ns_type: NamespaceType,
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

// THis will impl serde's deserialize on config
impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawConfig::deserialize(deserializer)?;

        // 1. Version validation
        if !raw.oci_version.starts_with("1.") {
            return Err(serde::de::Error::custom("Unsupported version."));
        }

        // 2. Rootless validation
        if Uid::effective().is_root() {
            return Err(serde::de::Error::custom("Runtime with only Rootless container."));
        }

        let linux = raw.linux.ok_or_else(|| 
            serde::de::Error::custom("linux section required for rootless")
        )?;

        // Validating root mappings
        if !linux.uid_mappings.iter().any(|m| m.container_id == 0) {
            return Err(serde::de::Error::custom("Missing uid 0 mapping"));
        }
        if !linux.gid_mappings.iter().any(|m| m.container_id == 0) {
            return Err(serde::de::Error::custom("Missing gid 0 mapping"));
        }

        Ok(Config {
            oci_version: OciVersion(raw.oci_version),
            linux,
            root : raw.root,
            process : raw.process,
            mounts : raw.mounts,
            hostname : raw.hostname,
            extra: raw.extra
        })
    }
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
         let file = File::open(&path)?;
         let reader = BufReader::new(file);
         let mut cfg: Self = serde_json::from_reader(reader)?;
         cfg.resolve_root_rel_to_bundle(&path)?;
         Ok(cfg)
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
}
