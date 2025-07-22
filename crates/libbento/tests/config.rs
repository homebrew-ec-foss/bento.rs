use libbento::config::Config;

#[cfg(test)]

mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_full_config_parsing() {
        let config = r#"
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

        write!(file, "{}", config).expect("Failed to write to temp file");

        file.flush().expect("Failed to flush");

        let cfg = Config::load(file.path()).expect("Failed to parse config.");
        
        assert_eq!(cfg.root.readonly, true);
        assert_eq!(cfg.process.unwrap().args, vec!["/bin/sh"]);

        fs::remove_dir_all(&rootfs_path).expect("Failed to remove rootfs directory");
    }
}
