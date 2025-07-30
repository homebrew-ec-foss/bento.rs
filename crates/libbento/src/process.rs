// crates/libbento/src/process.rs

use crate::syscalls::{
    disable_setgroups_for_child, fork_intermediate, map_user_namespace_rootless,
    unshare_remaining_namespaces, unshare_user_namespace,
};
use anyhow::{Result, anyhow};
use nix::unistd::Pid;
use nix::unistd::getpid;
use nix::unistd::{pipe, read, write};
use std::os::unix::io::{AsRawFd, OwnedFd};

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
            args: vec!["/bin/cat".to_string(), "/proc/self/stat".to_string()],
            hostname: "bento-container".to_string(),
            rootless: true,
            bundle_path: ".".to_string(),
            container_id: "default".to_string(),
        }
    }
}

pub fn create_container(config: &Config) -> Result<()> {
    // Create bidirectional communication channels
    let parent_to_child =
        pipe().map_err(|e| anyhow!("Failed to create parent->child pipe: {}", e))?;
    let child_to_parent =
        pipe().map_err(|e| anyhow!("Failed to create child->parent pipe: {}", e))?;

    println!("[Sync] Pipes created:");
    println!(
        " Parent->Child: read_fd={}, write_fd={}",
        parent_to_child.0.as_raw_fd(),
        parent_to_child.1.as_raw_fd()
    );
    println!(
        " Child->Parent: read_fd={}, write_fd={}",
        child_to_parent.0.as_raw_fd(),
        child_to_parent.1.as_raw_fd()
    );

    println!("Bento.rs Rootless Container Runtime");
    println!("Parent PID: {} (running as unprivileged user)", getpid());

    // Clone FDs for proper ownership distribution
    let parent_write_fd = parent_to_child
        .1
        .try_clone()
        .map_err(|e| anyhow!("Failed to clone parent write FD: {}", e))?;
    let parent_read_fd = child_to_parent
        .0
        .try_clone()
        .map_err(|e| anyhow!("Failed to clone parent read FD: {}", e))?;
    let child_read_fd = parent_to_child
        .0
        .try_clone()
        .map_err(|e| anyhow!("Failed to clone child read FD: {}", e))?;
    let child_write_fd = child_to_parent
        .1
        .try_clone()
        .map_err(|e| anyhow!("Failed to clone child write FD: {}", e))?;

    // Cleaner functions
    let parent_sync_handler = move |child_pid| -> Result<()> {
        handle_namespace_synchronization(
            child_pid,
            parent_read_fd,
            parent_write_fd,
            parent_to_child.0,
            child_to_parent.1,
        )
    };

    let container_bootstrap = move || -> isize {
        execute_container_initialization(
            config,
            child_read_fd,
            child_write_fd,
            parent_to_child.1,
            child_to_parent.0,
        )
    };

    fork_intermediate(parent_sync_handler, container_bootstrap)?;
    Ok(())
}

// Main process function
fn handle_namespace_synchronization(
    child_pid: Pid,
    read_fd: OwnedFd,
    write_fd: OwnedFd,
    unused_parent_read: OwnedFd,
    unused_child_write: OwnedFd,
) -> Result<()> {
    println!("[Parent] Child spawned with PID: {child_pid}");

    // Close unused pipe ends in parent
    drop(unused_parent_read); // Don't need to read from parent->child
    drop(unused_child_write); // Don't need to write to child->parent

    // Wait for child namespace ready signal
    println!("[Parent] Waiting for child namespace ready signal...");
    let mut buf = [0u8; 1];
    read(read_fd, &mut buf)
        .map_err(|e| anyhow!("Failed to receive namespace ready signal: {}", e))?;

    if buf[0] != b'R' {
        return Err(anyhow!(
            "Invalid namespace ready signal received: {}",
            buf[0]
        ));
    }

    println!("[Parent] Received namespace ready signal from child");

    // Perform UID/GID mapping
    map_user_namespace_rootless(child_pid)?;
    println!("[Parent] UID/GID mapping completed successfully");

    // Signal mapping complete
    write(write_fd, b"M").map_err(|e| anyhow!("Failed to signal mapping complete: {}", e))?;
    println!("[Parent] Signaled mapping completion to child");

    Ok(())
}

// Intermediate function
fn execute_container_initialization(
    _config: &Config,
    read_fd: OwnedFd,
    write_fd: OwnedFd,
    unused_parent_write: OwnedFd,
    unused_child_read: OwnedFd,
) -> isize {
    println!(
        "[Child] Container initialization started, PID: {}",
        getpid()
    );

    // Close unused pipe ends in child
    drop(unused_parent_write); // Don't need to write to parent->child
    drop(unused_child_read); // Don't need to read from child->parent

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

    // Signal parent that namespace is ready
    println!("[Child] Signaling namespace ready to parent");
    if write(write_fd, b"R").is_err() {
        eprintln!("[Child] Failed to signal namespace ready");
        return 1;
    }

    // Wait for mapping completion
    println!("[Child] Waiting for mapping complete signal...");
    let mut buf = [0u8; 1];
    if read(read_fd, &mut buf).is_err() {
        eprintln!("[Child] Failed to read mapping complete signal");
        return 1;
    }

    if buf[0] != b'M' {
        eprintln!("[Child] Invalid mapping complete signal received");
        return 1;
    }

    println!("[Child] Received mapping complete signal");

    // Phase 2: Create remaining namespaces
    if unshare_remaining_namespaces().is_err() {
        eprintln!("[Child] Failed to create remaining namespaces");
        return 1;
    }

    // Phase 3: Execute target command
    /*println!("[Child] Executing command: {:?}", config.args);
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
    }*/
    println!("[Child] Exiting create process - container is created but not started");
    0
}
