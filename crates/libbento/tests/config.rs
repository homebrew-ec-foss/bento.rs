use libbento::config;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_full_config_parsing() {
        let config_content = r#"
        {
            "ociVersion": "1.0.2",
            "root": {
                "path": "rootfs",
                "readonly": true
            },
            "process": {
                "args": ["/bin/sh"],
                "cwd": "/",
                "env": ["PATH=/usr/bin"],
                "noNewPrivileges": true
            },
            "mounts": [
                {
                    "destination": "/proc",
                    "type": "proc",
                    "source": "proc",
                    "options": ["nosuid", "noexec"]
                }
            ],
            "hostname": "test-container",
            "linux": {
                "uidMappings": [
                    {"container_id": 0, "host_id": 1000, "size": 1},
                    {"container_id": 1, "host_id": 1001, "size": 99}
                ],
                "gidMappings": [
                    {"container_id": 0, "host_id": 1000, "size": 1},
                    {"container_id": 1, "host_id": 1001, "size": 99}
                ],
                "namespaces": [
                    {"type": "pid"},
                    {"type": "mount"}
                ],
                "resources": {
                    "memory": {
                        "limit": 536870912
                    },
                    "cpu": {
                        "shares": 256
                    }
                }
            },
            "runtime": {
                "pivot_root": false
            },
            "extraField": "should-be-preserved"
        }"#;
           
        let mut file = NamedTempFile::new().expect("Failed to create temp file.");
        println!("File path: {:?}", file.path());
        assert!(file.path().exists());

        let temp_dir = file.path().parent().expect("Failed to get parent directory");
        let rootfs_path = temp_dir.join("rootfs");
        fs::create_dir_all(&rootfs_path).expect("Failed to create rootfs directory");

        write!(file, "{}", config_content).expect("Failed to write to temp file");

        file.flush().expect("Failed to flush");

        let _cfg = config::Config::load(file.path()).expect("Failed to parse config.");
        
        assert_eq!(_cfg.root.readonly, true);
        assert_eq!(_cfg.process.as_ref().unwrap().args, vec!["/bin/sh"]);

        fs::remove_dir_all(&rootfs_path).expect("Failed to remove rootfs directory");
    }

    #[test]
    fn test_config_load() {
        // Create a temporary config file
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        
        // Write some test config content - adjust this based on your actual config format
        let config_content = r#"{
            "ociVersion": "1.0.0",
            "root": {
                "path": "rootfs",
                "readonly": true
            },
            "linux": {
                "uidMappings": [
                    {"container_id": 0, "host_id": 1000, "size": 1}
                ],
                "gidMappings": [
                    {"container_id": 0, "host_id": 1000, "size": 1}
                ]
            }
        }"#;
        
        write!(file, "{}", config_content).expect("Failed to write to temp file");
        
        let _cfg = config::Config::load(file.path()).expect("Failed to parse config.");
    }

    #[test]
    fn test_config_invalid_file() {
        use std::path::Path;
        
        // Test with non-existent file
        let result = config::Config::load(Path::new("/non/existent/file.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_config_invalid_json() {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        
        // Write invalid JSON
        let invalid_content = r#"{
            "ociVersion": "1.0.0"
            "missing_comma": "value"
        }"#;
        
        write!(file, "{}", invalid_content).expect("Failed to write to temp file");
        
        let result = config::Config::load(file.path());
        assert!(result.is_err());
    }
}
