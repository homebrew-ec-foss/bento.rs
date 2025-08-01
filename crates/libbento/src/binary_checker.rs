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
        if !Self::binary_exists("unshare") {
            return Err(anyhow!(
                "unshare not found. Install with: sudo apt-get install util-linux"
            ));
        }

        if !Self::binary_exists("slirp4netns") {
            return Err(anyhow!(
                "slirp4netns not found. Install with: sudo apt-get install slirp4netns"
            ));
        }

        Ok(())
    }

    pub fn check_system() -> Result<()> {
        println!("🔍 Checking system capabilities...\n");

        Self::validate_required_binaries()?;
        println!("✅ unshare found");
        println!("✅ slirp4netns found");

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
        println!("\n🎉 System ready for direct networking!");

        Ok(())
    }
}
