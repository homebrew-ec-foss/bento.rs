// crates/libbento/src/process.rs

use nix::unistd::getpid;  // Remove pipe import
use anyhow::Result;
use std::ffi::CString;
use nix::unistd::execvp;

use crate::syscalls_fork::fork_intermediate;
use crate::syscalls_unshare::{unshare_user_namespace, unshare_remaining_namespaces};
use crate::syscalls_userns::{disable_setgroups_for_child, map_user_namespace_rootless};

pub struct Config {
    pub root_path: String,
    pub args: Vec<String>,
    pub hostname: String,
    pub rootless: bool,
    pub bundle_path: String,
    pub container_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_path: "/tmp/bento-rootfs".to_string(),
            args: vec!["/usr/bin/id".to_string()],
            hostname: "bento-container".to_string(),
            rootless: true,
            bundle_path: ".".to_string(),
            container_id: "default".to_string(),
        }
    }
}

pub fn create_container(config: &Config) -> Result<()> {
    println!("Bento.rs Rootless Container Runtime");
    println!("Parent PID: {} (running as unprivileged user)", getpid());

    let parent_logic = move |child_pid| -> Result<()> {
        println!("[Parent] Child spawned with PID: {}", child_pid);
        
        // Simple timing - give child time to create user namespace
        std::thread::sleep(std::time::Duration::from_millis(50));
        
        // Now do the mapping
        map_user_namespace_rootless(child_pid)?;
        
        println!("[Parent] Mapping complete");
        Ok(())
    };

    let child_logic = move || -> isize {
        println!("[Child] Child process started, PID: {}", getpid());
        
        // Create user namespace
        if unshare_user_namespace().is_err() {
            eprintln!("[Child] Failed to create user namespace");
            return 1;
        }
        
        // Disable setgroups
        if disable_setgroups_for_child().is_err() {
            eprintln!("[Child] Failed to disable setgroups");
            return 1;
        }
        
        // Give parent time to do mapping
        std::thread::sleep(std::time::Duration::from_millis(100));
        
        // Create remaining namespaces
        if unshare_remaining_namespaces().is_err() {
            eprintln!("[Child] Failed to create remaining namespaces");
            return 1;
        }
        
        // Execute command
        println!("[Child] Executing command: {:?}", config.args);
        let command = CString::new(config.args[0].as_str()).unwrap();
        let args: Vec<CString> = config.args.iter()
            .map(|s| CString::new(s.as_str()).unwrap())
            .collect();

        match execvp(&command, &args) {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("[Child] execvp failed: {}", e);
                1
            }
        }
    };

    fork_intermediate(parent_logic, child_logic)?;
    Ok(())
}

