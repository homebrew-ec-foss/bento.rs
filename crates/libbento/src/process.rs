use nix::sched::CloneFlags;
use nix::unistd::getpid;
use nix::Error;  // For Result error type

use crate::syscalls_fork::fork_intermediate;
use crate::syscalls_unshare::unshare_namespaces;
use crate::syscalls_clone::clone_init;

pub struct Config {
    pub root_path: &'static str,
    pub args: Vec<&'static str>,
    pub hostname: &'static str,
    pub rootless: bool,  // Toggle for rootless mode
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_path: "/tmp/bento-rootfs",
            args: vec!["/bin/sh"],
            hostname: "bento-container",
            rootless: true,  // Default to rootless for Bento.rs goals
        }
    }
}

pub fn create_container(config: &Config) -> Result<(), Error> {
    println!("Parent PID: {}", getpid());

    fork_intermediate(|| {
        println!("Intermediate PID: {}", getpid());

        let mut flags = CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWNS;
        if config.rootless {
            flags |= CloneFlags::CLONE_NEWUSER;
            // TODO: Add UID/GID mapping here (write to /proc/self/uid_map)
            // Example: use std::fs::write("/proc/self/uid_map", "0 1000 1").expect("Mapping failed");
        }

        if unshare_namespaces(flags).is_err() {
            return 1;
        }

        if clone_init(config, flags).is_err() {
            return 1;
        }

        0  // Success exit code
    })?;

    Ok(())
}

