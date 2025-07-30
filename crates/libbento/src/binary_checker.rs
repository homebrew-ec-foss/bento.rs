use anyhow::{Result, anyhow};
use std::process::Command;

pub struct BinaryChecker;

impl BinaryChecker {
    pub fn binary_exists(binary: &str) -> bool {
        Command::new("which")
            .arg(binary)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    pub fn validate_required_binaries() -> Result<()> {
        if !Self::binary_exists("rootlesskit") {
            return Err(anyhow!(
                "rootlesskit not found. Install from https://github.com/rootless-containers/rootlesskit or via apt"
            ));
        }
        Ok(())
    }

    pub fn get_available_drivers() -> Vec<String> {
        let mut drivers = Vec::new();

        if Self::binary_exists("pasta") {
            drivers.push("pasta".to_string());
        }

        if Self::binary_exists("slirp4netns") {
            drivers.push("slirp4netns".to_string());
        }

        drivers
    }

    pub fn driver_available(driver: &str) -> bool {
        match driver {
            "pasta" => Self::binary_exists("pasta"),
            "slirp4netns" => Self::binary_exists("slirp4netns"),
            _ => false,
        }
    }

    pub fn check_system() -> Result<()> {
        println!("🔍 Checking system capabilities...\n");

        Self::validate_required_binaries()?;
        println!("✅ rootlesskit found");

        let drivers = Self::get_available_drivers();
        if drivers.is_empty() {
            return Err(anyhow!(
                "No network drivers found. Install with: sudo apt-get install passt slirp4netns"
            ));
        }

        println!("✅ Network drivers found: {}", drivers.join(", "));

        let max_ns = std::fs::read_to_string("/proc/sys/user/max_user_namespaces")
            .map_err(|_| anyhow!("Cannot check user namespace support"))?;

        let max: i32 = max_ns
            .trim()
            .parse()
            .map_err(|_| anyhow!("Invalid user namespace configuration"))?;

        if max <= 0 {
            return Err(anyhow!(
                "User namespaces disabled. Enable with: echo 1000 | sudo tee /proc/sys/user/max_user_namespaces"
            ));
        }

        println!("✅ User namespaces enabled (max: {})", max);
        println!("\n🎉 System ready for networking!");

        Ok(())
    }
}
