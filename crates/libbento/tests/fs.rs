#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use tempfile::TempDir;
    use std::{fs,io};

    // Mock implementations for testing 
    mod mock {
        use super::*;
        use std::collections::HashMap;
        use std::sync::Mutex;
        
        // Global state to track mock ops
        lazy_static::lazy_static! {
            static ref MOCK_MOUNTS: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
            static ref MOCK_DEVICES: Mutex<Vec<String>> = Mutex::new(Vec::new());
        }
        
        pub fn mock_mount(target: &str, fstype: &str) -> Result<(), io::Error> {
            let mut mounts = MOCK_MOUNTS.lock().unwrap();
            mounts.insert(target.to_string(), fstype.to_string());
            Ok(())
        }
        
        pub fn mock_umount(target: &str) -> Result<(),io::Error> {
            let mut mounts = MOCK_MOUNTS.lock().unwrap();
            mounts.remove(target);
            Ok(())
        }
        
        pub fn mock_mknod(path: &str, _major: u64, _minor: u64) -> Result<(),io::Error> {
            let mut devices = MOCK_DEVICES.lock().unwrap();
            devices.push(path.to_string());
            std::fs::File::create(path)?;
            Ok(())
        }
        
        pub fn get_mock_mounts() -> HashMap<String, String> {
            MOCK_MOUNTS.lock().unwrap().clone()
        }
        
        pub fn get_mock_devices() -> Vec<String> {
            MOCK_DEVICES.lock().unwrap().clone()
        }
        
        pub fn clear_mocks() {
            MOCK_MOUNTS.lock().unwrap().clear();
            MOCK_DEVICES.lock().unwrap().clear();
        }
    }

    #[test]
    fn test_directory_creation() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let rootfs = temp_dir.path();
        
        // Testing whether directories are created properly
        let proc_path = rootfs.join("proc");
        let sys_path = rootfs.join("sys");
        let dev_path = rootfs.join("dev");
        
        fs::create_dir_all(&proc_path).expect("Failed to create proc dir");
        fs::create_dir_all(&sys_path).expect("Failed to create sys dir");
        fs::create_dir_all(&dev_path).expect("Failed to create dev dir");
        
        assert!(proc_path.exists());
        assert!(proc_path.is_dir());
        assert!(sys_path.exists());
        assert!(sys_path.is_dir());
        assert!(dev_path.exists());
        assert!(dev_path.is_dir());
    }

    #[test]
    fn test_device_node_creation_logic() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let dev_path = temp_dir.path().join("dev");
        fs::create_dir_all(&dev_path).expect("Failed to create dev directory");
        
        //  Here it tests device creation  
          let devices = [
            ("null", 1, 3),
            ("urandom", 1, 9),
            ("tty", 5, 0),
        ];
        
        for (name, major, minor) in devices {
            let path = dev_path.join(name);
            let path_str = path.to_str().expect("Failed to convert path to string");
            
            assert!(path_str.contains(name));
            assert!(path.parent().unwrap() == dev_path);
            
            mock::mock_mknod(path_str, major, minor).expect("Mock device creation failed");
        }
        
        let created_devices = mock::get_mock_devices();
        assert_eq!(created_devices.len(), 3);
        assert!(created_devices.iter().any(|d| d.contains("null")));
        assert!(created_devices.iter().any(|d| d.contains("urandom")));
        assert!(created_devices.iter().any(|d| d.contains("tty")));
        
        mock::clear_mocks();
    }

    #[test]
    fn test_path_handling() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let rootfs = temp_dir.path();
        
        // Testing path joining logic
        let proc_path = rootfs.join("proc");
        let sys_path = rootfs.join("sys");
        let dev_path = rootfs.join("dev");
        
        assert_eq!(proc_path.file_name().unwrap(), "proc");
        assert_eq!(sys_path.file_name().unwrap(), "sys");
        assert_eq!(dev_path.file_name().unwrap(), "dev");
        
        // Test path to str conversion
        assert!(proc_path.to_str().is_some());
        assert!(sys_path.to_str().is_some());
        assert!(dev_path.to_str().is_some());
    }

    #[test]
    fn test_error_handling_paths() {
        let bad_path = PathBuf::from("");
        let result = bad_path.to_str();
        assert!(result.is_some()); // Empty path should still convert
        
        let relative_path = PathBuf::from("./test");
        assert!(relative_path.to_str().is_some());
    }

    #[test]
    fn test_mount_sequence_logic() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let rootfs = temp_dir.path();
        
        // Create directories
        fs::create_dir_all(rootfs.join("proc")).expect("Failed to create proc");
        fs::create_dir_all(rootfs.join("sys")).expect("Failed to create sys");
        fs::create_dir_all(rootfs.join("dev")).expect("Failed to create dev");
        
        // Mock the mounting sequence
        mock::mock_mount(&rootfs.join("proc").to_string_lossy(), "proc")
            .expect("Mock proc mount failed");
        mock::mock_mount(&rootfs.join("sys").to_string_lossy(), "sysfs")
            .expect("Mock sys mount failed");
        mock::mock_mount(&rootfs.join("dev").to_string_lossy(), "tmpfs")
            .expect("Mock dev mount failed");
        
        let mounts = mock::get_mock_mounts();
        assert_eq!(mounts.len(), 3);
        assert!(mounts.values().any(|v| v == "proc"));
        assert!(mounts.values().any(|v| v == "sysfs"));
        assert!(mounts.values().any(|v| v == "tmpfs"));
        
        mock::clear_mocks();
    }

    #[test]
    fn test_cleanup_sequence_logic() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let rootfs = temp_dir.path();
        
        // Setting up mock mounts
        let dev_path = rootfs.join("dev").to_string_lossy().to_string();
        let sys_path = rootfs.join("sys").to_string_lossy().to_string();
        let proc_path = rootfs.join("proc").to_string_lossy().to_string();
        
        mock::mock_mount(&proc_path, "proc").expect("Mock proc mount failed");
        mock::mock_mount(&sys_path, "sysfs").expect("Mock sys mount failed");
        mock::mock_mount(&dev_path, "tmpfs").expect("Mock dev mount failed");
        
        assert_eq!(mock::get_mock_mounts().len(), 3);
        
        // Test cleanup sequence : reverse
        mock::mock_umount(&dev_path).expect("Mock dev umount failed");
        mock::mock_umount(&sys_path).expect("Mock sys umount failed");
        mock::mock_umount(&proc_path).expect("Mock proc umount failed");
        
        assert_eq!(mock::get_mock_mounts().len(), 0);
        
        mock::clear_mocks();
    }

    #[test]
    fn test_device_major_minor_numbers() {
        let devices = [
            ("null", 1, 3),
            ("urandom", 1, 9),
            ("tty", 5, 0),
        ];
        
        for (name, expected_major, expected_minor) in devices {
            match name {
                "null" => {
                    assert_eq!(expected_major, 1);
                    assert_eq!(expected_minor, 3);
                }
                "urandom" => {
                    assert_eq!(expected_major, 1);
                    assert_eq!(expected_minor, 9);
                }
                "tty" => {
                    assert_eq!(expected_major, 5);
                    assert_eq!(expected_minor, 0);
                }
                _ => panic!("Unexpected device: {}", name),
            }
        }
    }

    #[test]
    fn test_file_permissions_logic() {
        let mode = 0o666;
        
        // Checking permissions  below 
        // owner read and write
        assert_eq!(mode & 0o400, 0o400); 
        assert_eq!(mode & 0o200, 0o200); 
        // grp read and write
        assert_eq!(mode & 0o040, 0o040); 
        assert_eq!(mode & 0o020, 0o020); 
        // others read and write
        assert_eq!(mode & 0o004, 0o004);  
        assert_eq!(mode & 0o002, 0o002); 
        
        assert_eq!(mode & 0o111, 0); // no execute for anyone
    }

    #[test]
    fn test_cstring_conversion() {
        let test_paths = vec![
            "/dev/null",
            "/dev/urandom",
            "/dev/tty",
            "simple_path",
            "/complex/path/with/multiple/components",
        ];
        
        for path in test_paths {
            let cstring_result = std::ffi::CString::new(path);
            assert!(cstring_result.is_ok(), "Failed to convert path to CString: {}", path);
            
            let cstring = cstring_result.unwrap();
            let back_to_str = cstring.to_str();
            assert!(back_to_str.is_ok(), "Failed to convert CString back to str");
            assert_eq!(back_to_str.unwrap(), path);
        }
    }

    #[test]
    fn test_makedev_calculation() {
        // Test that makedev calculation works as expected
        use nix::libc::makedev;
        
        let test_devices = [
            (1, 3),  // null
            (1, 9),  // urandom
            (5, 0),  // tty
        ];
        
        for (major, minor) in test_devices {
            let dev_id = makedev(major, minor);
            
            // Extract major and minor back from dev_id to verify
            let extracted_major = (dev_id >> 8) & 0xff;
            let extracted_minor = dev_id & 0xff;
            
            assert_eq!(extracted_major as u64, major.into());
            assert_eq!(extracted_minor as u64, minor.into());
        }
    }
}
