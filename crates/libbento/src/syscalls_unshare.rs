// crates/libbento/src/syscalls_unshare.rs

use nix::sched::{unshare, CloneFlags};
use anyhow::{Result, anyhow};

/// Phase 1: Create only the user namespace (unprivileged operation)
/// This can be done by any unprivileged user
pub fn unshare_user_namespace() -> Result<()> {
    let flags = CloneFlags::CLONE_NEWUSER;
    unshare(flags).map_err(|e| anyhow!("Failed to unshare user namespace: {}", e))?;
    println!("[Child] Created user namespace successfully");
    Ok(())
}

/// Phase 2: Create remaining namespaces (requires CAP_SYS_ADMIN from UID mapping)
/// This can only be done after the parent has mapped UID/GID
pub fn unshare_remaining_namespaces() -> Result<()> {
    let flags = CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWNS;
    unshare(flags).map_err(|e| anyhow!("Failed to unshare remaining namespaces: {}", e))?;
    println!("[Child] Created remaining namespaces: {:?}", flags);
    Ok(())
}

