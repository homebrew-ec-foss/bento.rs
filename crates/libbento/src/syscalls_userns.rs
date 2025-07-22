// crates/libbento/src/syscalls_userns.rs

use nix::unistd::{getuid, getgid, Pid};
use anyhow::{anyhow, Result};
use std::process::Command;
use std::fs;

/// Child-side operation: Disable setgroups for safe GID mapping
/// Must be called by the child process after creating user namespace
pub fn disable_setgroups_for_child() -> Result<()> {
    fs::write("/proc/self/setgroups", "deny")
        .map_err(|e| anyhow!("Failed to disable setgroups: {}", e))?;
    println!("[Child] Disabled setgroups for safe mapping");
    Ok(())
}

/// Execute newuidmap or newgidmap command with comprehensive error handling
fn execute_mapping_helper(command: &str, child_pid: Pid, container_id: u32, host_id: u32, count: u32) -> Result<()> {
    println!("[Parent] Executing: {} {} {} {} {}", command, child_pid, container_id, host_id, count);
    
    let output = Command::new(command)
        .arg(child_pid.to_string())
        .arg(container_id.to_string()) 
        .arg(host_id.to_string())
        .arg(count.to_string())
        .output()
        .map_err(|e| anyhow!("Failed to execute {}: {}. Is the uidmap package installed?", command, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "{} failed with exit code {}:\n\
            Command: {} {} {} {} {}\n\
            STDOUT: {}\n\
            STDERR: {}\n\
            \n\
            Possible fixes:\n\
            1. Install uidmap: sudo apt-get install uidmap\n\
            2. Configure subuid/subgid: sudo usermod --add-subuids 100000-165535 $(whoami)\n\
            3. Check permissions: ls -la $(which {})",
            command,
            output.status,
            command, child_pid, container_id, host_id, count,
            stdout,
            stderr,
            command
        ));
    }

    println!("[Parent] {} succeeded", command);
    Ok(())
}

/// Parent-side operation: Perform rootless UID/GID mapping using newuidmap/newgidmap helpers
/// Static mapping: maps host user to container root (UID/GID 0)
pub fn map_user_namespace_rootless(child_pid: Pid) -> Result<()> {
    let host_uid = getuid().as_raw();
    let host_gid = getgid().as_raw();

    println!("[Parent] Starting rootless mapping for child {}", child_pid);
    println!("[Parent] Host UID: {}, Host GID: {}", host_uid, host_gid);

    // Map GID first (standard practice to avoid permission issues)
    // Format: newgidmap <pid> <container-gid> <host-gid> <count>
    execute_mapping_helper("newgidmap", child_pid, 0, host_gid, 1)?;

    // Map UID second  
    // Format: newuidmap <pid> <container-uid> <host-uid> <count>
    execute_mapping_helper("newuidmap", child_pid, 0, host_uid, 1)?;

    println!("[Parent] Rootless mapping complete: host user -> container root");
    Ok(())
}

