/* 

         1. validate : validates basic mapping constraints size zero and overflow.
         2. to_proc_format : Formats the mapping for writing to /proc/[pid]/uid_map or /proc/[pid]/gid_map.
         3. uid_map_entries() / gid_map_entries() : Aggregates all mappings into strings ready for /proc files.
         4. validate_mappings() : Coordinates validation of both UID and GID mappings
         5. validate_id_mappings() :  Validates arrays of ID mappings 
         6. check_overlaps() - Checks for overlapping ranges using BTreeMap
         7. validate_host_permissions() - Basic permission checks for host IDs
         8. validate_rootless_requirements() - Ensures container root (0) is mapped

Improvements : 
         Subordinate ID Validation: Needs integration with /etc/subuid and /etc/subgid
         User Namespace Coordination: Missing parent-child synchronization logic
         Capability Handling: Should consider capabilities when validating mappings
*/

use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use std::{
    collections::{HashMap, BTreeMap},
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

impl IDMap {
    /// Validate individual mapping for basic constraints
    pub fn validate(&self, mapping_type: &str) -> Result<(), ConfigError> {
        if self.size == 0 {
            return Err(ConfigError::Invalid(format!(
                "{} mapping has zero size", mapping_type
            )));
        }

        if self.container_id.checked_add(self.size - 1).is_none() {
            return Err(ConfigError::Invalid(format!(
                "{} mapping container_id {} + size {} would overflow", 
                mapping_type, self.container_id, self.size
            )));
        }

        if self.host_id.checked_add(self.size - 1).is_none() {
            return Err(ConfigError::Invalid(format!(
                "{} mapping host_id {} + size {} would overflow", 
                mapping_type, self.host_id, self.size
            )));
        }

        Ok(())
    }

    /// Format for writing to /proc/self/uid_map or /proc/self/gid_map
    pub fn to_proc_format(&self) -> String {
        format!("{} {} {}", self.container_id, self.host_id, self.size)
    }
}

impl Linux {
    /// Get all UID mappings formatted for /proc/self/uid_map
    pub fn uid_map_entries(&self) -> Vec<String> {
        self.uid_mappings.iter().map(|m| m.to_proc_format()).collect()
    }

    /// Get all GID mappings formatted for /proc/self/gid_map  
    pub fn gid_map_entries(&self) -> Vec<String> {
        self.gid_mappings.iter().map(|m| m.to_proc_format()).collect()
    }

    /// Validate mappings for fork/userns creation
    pub fn validate_mappings(&self) -> Result<(), ConfigError> {
        self.validate_id_mappings(&self.uid_mappings, "uid")?;
        self.validate_id_mappings(&self.gid_mappings, "gid")?;
        self.validate_rootless_requirements()?;
        Ok(())
    }

    fn validate_id_mappings(&self, mappings: &[IDMap], mapping_type: &str) -> Result<(), ConfigError> {
        for mapping in mappings {
            mapping.validate(mapping_type)?;
        }
        self.check_overlaps(mappings, mapping_type)?;
        Ok(())
    }

    fn check_overlaps(&self, mappings: &[IDMap], mapping_type: &str) -> Result<(), ConfigError> {
        let mut container_ranges = BTreeMap::new();
        let mut host_ranges = BTreeMap::new();
        
        for mapping in mappings {
            let container_start = mapping.container_id;
            let container_end = mapping.container_id + mapping.size - 1;
            let host_start = mapping.host_id;
            let host_end = mapping.host_id + mapping.size - 1;
            
            // Check container ID overlaps
            for (_existing_start, &existing_end) in container_ranges.range(..=container_end) {
                if existing_end >= container_start {
                    return Err(ConfigError::Invalid(format!(
                        "Overlapping {} container ID ranges", mapping_type
                    )));
                }
            }
            
            // Check host ID overlaps
            for (_existing_start, &existing_end) in host_ranges.range(..=host_end) {
                if existing_end >= host_start {
                    return Err(ConfigError::Invalid(format!(
                        "Overlapping {} host ID ranges", mapping_type
                    )));
                }
            }
            
            container_ranges.insert(container_start, container_end);
            host_ranges.insert(host_start, host_end);
        }
        
        Ok(())
    }

    /// Validate that container root (0) is mapped for rootless
    pub fn validate_rootless_requirements(&self) -> Result<(), ConfigError> {
        let has_root_uid = self.uid_mappings.iter().any(|m| {
            m.container_id == 0 || (m.container_id < 1 && m.container_id + m.size > 0)
        });
        
        if !has_root_uid {
            return Err(ConfigError::Invalid(
                "rootless containers must map container UID 0".into()
            ));
        }

        let has_root_gid = self.gid_mappings.iter().any(|m| {
            m.container_id == 0 || (m.container_id < 1 && m.container_id + m.size > 0)
        });
        
        if !has_root_gid {
            return Err(ConfigError::Invalid(
                "rootless containers must map container GID 0".into()
            ));
        }

        Ok(())
    }

    /// Check if current user can use these host IDs (basic check)
    pub fn validate_host_permissions(&self) -> Result<(), ConfigError> {
        let current_uid = Uid::effective().as_raw();
        
        // For rootless, typically can only map to subuid/subgid ranges
        // This is a basic check - full validation requires reading /etc/subuid, /etc/subgid
        for mapping in &self.uid_mappings {
            if mapping.host_id < current_uid {
                return Err(ConfigError::Invalid(
                    "Cannot map to host UIDs below current user".into()
                ));
            }
        }
        
        Ok(())
    }
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
        
        // Validate ID mappings if present
        if let Some(linux) = &self.linux {
            linux.validate_mappings()?;
            
            // Additional rootless validation
            let rootless = !Uid::effective().is_root();
            if rootless {
                linux.validate_host_permissions()?;
                
                // Check if uid/gid mappings are present for rootless
                if linux.uid_mappings.is_empty() || linux.gid_mappings.is_empty() {
                    return Err(ConfigError::Invalid(
                        "uid/gid mappings missing in rootless mode".into(),
                    ));
                }
            }
        } else {
            // Check if rootless and had gid uid mappings
            let rootless = !Uid::effective().is_root();	
            if rootless {
                return Err(ConfigError::Invalid(
                    "linux section required for rootless".into()
                ));
            }
        }
        
        Ok(())
    }
}
