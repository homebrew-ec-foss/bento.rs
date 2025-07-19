use nix::sched::{unshare, CloneFlags};
use anyhow::{Result, anyhow};

/// Detaches the current process into new namespaces using the provided flags.
/// Returns Ok on success.
pub fn unshare_namespaces(flags: CloneFlags) -> Result<()> {
    unshare(flags).map_err(|e| anyhow!("Unshare failed: {}", e))?;
    println!("Unshared namespaces with flags: {:?}", flags);
    Ok(())
}

