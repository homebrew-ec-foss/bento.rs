// crates/libbento/src/syscalls.rs

use anyhow::{Result, anyhow};
use libc;
use nix::sched::{CloneFlags, clone, unshare};
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, Pid, execvp, fork, getgid, getpid, getuid, sethostname};
use std::ffi::CString;
use std::fs;
use std::process::Command;

use crate::process::Config;

// ============================================================================
// FORK AND PROCESS CREATION
// ============================================================================

/// Fork wrapper with parent and child logic separation
pub fn fork_intermediate<P, C>(parent_logic: P, child_logic: C) -> Result<Pid>
where
    P: FnOnce(Pid) -> Result<()>,
    C: FnOnce() -> isize,
{
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            parent_logic(child)?;
            waitpid(child, None)?;
            Ok(child)
        }
        Ok(ForkResult::Child) => {
            let exit_code = child_logic();
            std::process::exit(exit_code as i32);
        }
        Err(e) => Err(anyhow!("Fork failed: {}", e)),
    }
}

/// Clones a new init process with the given flags, running the container command.
/// Executes in the isolated namespace.
pub fn clone_init(config: &Config, flags: CloneFlags) -> Result<()> {
    let mut stack = [0u8; 4096 * 4];

    let cb = Box::new(move || {
        println!("Init process: PID {}", getpid());

        if let Err(e) = sethostname(&config.hostname) {
            eprintln!("sethostname failed: {e}");
            return 1;
        }

        let args: Vec<CString> = config
            .args
            .iter()
            .map(|s| CString::new(s.as_str()).unwrap())
            .collect();
        let ref_args: Vec<&CString> = args.iter().collect();
        let cmd = &args[0];

        match execvp(cmd, &ref_args) {
            Ok(_) => 0, // Success (execvp replaces process, so this won't run)
            Err(e) => {
                eprintln!("execvp failed: {e}");
                1
            }
        }
    });

    let pid = unsafe { clone(cb, &mut stack, flags, Some(libc::SIGCHLD)) }
        .map_err(|e| anyhow!("Clone failed: {}", e))?;

    waitpid(pid, None).map_err(|e| anyhow!("Waitpid failed for cloned process: {}", e))?;

    Ok(())
}

// ============================================================================
// NAMESPACE OPERATIONS
// ============================================================================

/// Phase 1: Create only the user namespace (unprivileged operation)
/// This can be done by any unprivileged user
pub fn unshare_user_namespace() -> Result<()> {
    let flags = CloneFlags::CLONE_NEWUSER;
    unshare(flags).map_err(|e| anyhow!("Failed to unshare user namespace: {}", e))?;
    println!("[Bridge] Created user namespace successfully");
    Ok(())
}

/// Phase 2: Create remaining namespaces (requires CAP_SYS_ADMIN from UID mapping)
/// This can only be done after the parent has mapped UID/GID
pub fn unshare_remaining_namespaces() -> Result<()> {
    let flags = CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWNS;
    unshare(flags).map_err(|e| anyhow!("Failed to unshare remaining namespaces: {}", e))?;
    println!("[Bridge] Created remaining namespaces: {flags:?}");
    Ok(())
}

// ============================================================================
// USER NAMESPACE AND UID/GID MAPPING
// ============================================================================

/// Child-side operation: Disable setgroups for safe GID mapping
/// Must be called by the child process after creating user namespace
pub fn disable_setgroups_for_child() -> Result<()> {
    fs::write("/proc/self/setgroups", "deny")
        .map_err(|e| anyhow!("Failed to disable setgroups: {}", e))?;
    println!("[Bridge] Disabled setgroups for safe mapping");
    Ok(())
}

/// Execute newuidmap or newgidmap command with comprehensive error handling
fn execute_mapping_helper(
    command: &str,
    child_pid: Pid,
    container_id: u32,
    host_id: u32,
    count: u32,
) -> Result<()> {
    println!("[Orchestrator] Executing: {command} {child_pid} {container_id} {host_id} {count}");

    let output = Command::new(command)
        .arg(child_pid.to_string())
        .arg(container_id.to_string())
        .arg(host_id.to_string())
        .arg(count.to_string())
        .output()
        .map_err(|e| {
            anyhow!(
                "Failed to execute {}: {}. Is the uidmap package installed?",
                command,
                e
            )
        })?;

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
            command,
            child_pid,
            container_id,
            host_id,
            count,
            stdout,
            stderr,
            command
        ));
    }

    println!("[Orchestrator] {command} succeeded");
    Ok(())
}

/// Parent-side operation: Perform rootless UID/GID mapping using newuidmap/newgidmap helpers
/// Static mapping: maps host user to container root (UID/GID 0)
pub fn map_user_namespace_rootless(child_pid: Pid) -> Result<()> {
    let host_uid = getuid().as_raw();
    let host_gid = getgid().as_raw();

    println!("[Orchestrator] Starting rootless mapping for child {child_pid}");
    println!("[Orchestrator] Host UID: {host_uid}, Host GID: {host_gid}");

    // Map GID first (standard practice to avoid permission issues)
    // Format: newgidmap <pid> <container-gid> <host-gid> <count>
    execute_mapping_helper("newgidmap", child_pid, 0, host_gid, 1)?;

    // Map UID second
    // Format: newuidmap <pid> <container-uid> <host-uid> <count>
    execute_mapping_helper("newuidmap", child_pid, 0, host_uid, 1)?;

    println!("[Orchestrator] Rootless mapping complete: host user -> container root");
    Ok(())
}
