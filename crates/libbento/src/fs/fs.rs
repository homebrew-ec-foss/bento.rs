use anyhow::{Context, Result};
use nix::mount::{mount, umount, MsFlags};
use nix::unistd::chdir;
use std::path::Path;
use std::fs;

pub fn setup_fs(rootfs: &Path) -> Result<()> {
    // THis to check if we are in root
    chdir(rootfs).context("Failed to change directory to rootfs")?;

    // Mounting proc, sys and dev fsystem
    mount_proc(rootfs).context("Failed to mount proc")?;
    mount_sys(rootfs).context("Failed to mount sys")?;

    Ok(())
}

/// Unmounts pseudo filesystems
pub fn cleanup_fs(rootfs: &Path) -> Result<()> {
    // Unmount in reverse order of mounting
    let _ = umount(rootfs.join("dev").to_str().unwrap());
    let _ = umount(rootfs.join("sys").to_str().unwrap());
    umount(rootfs.join("proc").to_str().unwrap())
        .context("Failed to unmount proc")?;
    Ok(())
}

fn mount_proc(rootfs: &Path) -> Result<()> {
    let proc_path = rootfs.join("proc");
    fs::create_dir_all(&proc_path).context("Failed to create proc directory")?;

    mount(
        Some("proc"),
        &proc_path,
        Some("proc"),
        MsFlags::empty(),
        None::<&str>,
    ).context("Failed to mount proc filesystem")
}

fn mount_sys(rootfs: &Path) -> Result<()> {
    let sys_path = rootfs.join("sys");
    fs::create_dir_all(&sys_path).context("Failed to create sys directory")?;

    mount(
        Some("sysfs"),
        &sys_path,
        Some("sysfs"),
        MsFlags::empty(),
        None::<&str>,
    ).context("Failed to mount sys filesystem")
}
