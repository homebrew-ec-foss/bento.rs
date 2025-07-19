// crates/libbento/src/process.rs

use nix::unistd::getpid;
use nix::unistd::{pipe, read, write};
use std::os::unix::io::AsRawFd;
use anyhow::{Result, anyhow};
use std::ffi::CString;
use nix::unistd::execvp;
use crate::syscalls::{
    fork_intermediate,
    unshare_user_namespace, 
    unshare_remaining_namespaces,
    disable_setgroups_for_child, 
    map_user_namespace_rootless
};

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

//Initial Wait method
// Parent
	// Simple waiting - give child time to create user namespace
	//std::thread::sleep(std::time::Duration::from_millis(50));

//Child
        // Give parent time to do mapping
        // std::thread::sleep(std::time::Duration::from_millis(100));
pub fn create_container(config: &Config) -> Result<()> {
    // Create bidirectional communication channels
    let parent_to_child = pipe().map_err(|e| anyhow!("Failed to create parent->child pipe: {}", e))?;
    let child_to_parent = pipe().map_err(|e| anyhow!("Failed to create child->parent pipe: {}", e))?;
    
    println!("[Sync] Pipes created:");
    println!("  Parent->Child: read_fd={}, write_fd={}", parent_to_child.0.as_raw_fd(), parent_to_child.1.as_raw_fd());
    println!("  Child->Parent: read_fd={}, write_fd={}", child_to_parent.0.as_raw_fd(), child_to_parent.1.as_raw_fd());
    
    println!("Bento.rs Rootless Container Runtime");
    println!("Parent PID: {} (running as unprivileged user)", getpid());

    // Clone FDs for proper ownership distribution
    let parent_write_fd = parent_to_child.1.try_clone()
        .map_err(|e| anyhow!("Failed to clone parent write FD: {}", e))?;
    let parent_read_fd = child_to_parent.0.try_clone()
        .map_err(|e| anyhow!("Failed to clone parent read FD: {}", e))?;
    
    let child_read_fd = parent_to_child.0.try_clone()
        .map_err(|e| anyhow!("Failed to clone child read FD: {}", e))?;
    let child_write_fd = child_to_parent.1.try_clone()
        .map_err(|e| anyhow!("Failed to clone child write FD: {}", e))?;

    let parent_logic = move |child_pid| -> Result<()> {
        println!("[Parent] Child spawned with PID: {}", child_pid);
        
        // Close unused pipe ends in parent
        drop(parent_to_child.0);  // Don't need to read from parent->child
        drop(child_to_parent.1);  // Don't need to write to child->parent
        
        // Wait for child to signal that user namespace is ready
        println!("[Parent] Waiting for child namespace ready signal...");
        let mut buf = [0u8; 1];
        if let Err(e) = read(parent_read_fd, &mut buf) {
            eprintln!("[Parent] Failed to read namespace ready signal: {}", e);
            return Err(anyhow!("Failed to receive namespace ready signal"));
        }
        
        if buf[0] != b'R' {  // 'R' for Ready
            return Err(anyhow!("Invalid namespace ready signal received: {}", buf[0]));
        }
        println!("[Parent] Received namespace ready signal from child");
        
        // Now safely perform UID/GID mapping
        map_user_namespace_rootless(child_pid)?;
        println!("[Parent] Mapping complete");
        
        // Signal child that mapping is complete
        if let Err(e) = write(parent_write_fd, b"M") {  // 'M' for Mapping complete
            eprintln!("[Parent] Failed to signal mapping complete: {}", e);
            return Err(anyhow!("Failed to signal mapping complete"));
        }
        println!("[Parent] Signaled mapping complete to child");
        
        Ok(())
    };

    let child_logic = move || -> isize {
        println!("[Child] Child process started, PID: {}", getpid());
        
        // Close unused pipe ends in child
        drop(parent_to_child.1);  // Don't need to write to parent->child
        drop(child_to_parent.0);  // Don't need to read from child->parent
        
        // Phase 1: Create user namespace
        if unshare_user_namespace().is_err() {
            eprintln!("[Child] Failed to create user namespace");
            return 1;
        }
        
        // Disable setgroups for safe mapping
        if disable_setgroups_for_child().is_err() {
            eprintln!("[Child] Failed to disable setgroups");
            return 1;
        }
        
        // Signal parent that namespace is ready for mapping
        println!("[Child] Signaling namespace ready to parent");
        if let Err(e) = write(child_write_fd, b"R") {  // 'R' for Ready
            eprintln!("[Child] Failed to signal namespace ready: {}", e);
            return 1;
        }
        
        // Wait for parent to complete mapping
        println!("[Child] Waiting for mapping complete signal...");
        let mut buf = [0u8; 1];
        if let Err(e) = read(child_read_fd, &mut buf) {
            eprintln!("[Child] Failed to read mapping complete signal: {}", e);
            return 1;
        }
        
        if buf[0] != b'M' {  // 'M' for Mapping complete
            eprintln!("[Child] Invalid mapping complete signal received: {}", buf[0]);
            return 1;
        }
        println!("[Child] Received mapping complete signal");
        
        // Phase 2: Create remaining namespaces (now safe with proper UID/GID mapping)
        if unshare_remaining_namespaces().is_err() {
            eprintln!("[Child] Failed to create remaining namespaces");
            return 1;
        }
        
        // Phase 3: Execute target command
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
