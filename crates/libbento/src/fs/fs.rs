use anyhow::{Context, Result};
use nix::mount::{mount, umount, MsFlags};
use nix::unistd::chdir;
use std::path::Path;
use std::fs;
use nix::sys::stat::{mknod, Mode, SFlag};
use std::ffi::CString;

pub fn setup_fs(rootfs: &Path) -> Result<()> {
    // THis to check if we are in root
    chdir(rootfs).context("Failed to change directory to rootfs")?;

    // Mounting proc, sys and dev fsystem
    mount_proc(rootfs).context("Failed to mount proc")?;
    mount_sys(rootfs).context("Failed to mount sys")?;
    mount_dev(rootfs).context("Failed to mount dev")?;
    Ok(())
}

/// Unmounts pseudo filesystems
pub fn cleanup_fs(rootfs: &Path) -> Result<()> {
    // Unmount in reverse order of mounting
    let _ = umount(rootfs.join("dev").to_str().expect("Failed to convert dev path to str"));
    let _ = umount(rootfs.join("sys").to_str().expect("Failed to convert sys path to str"));

    umount(rootfs.join("proc").to_str()
        .expect("Failed to convert proc path to str"))
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
    ).context("Failed to mount proc filesystem")?;

    Ok(())
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
    ).context("Failed to mount sys filesystem")?;
    Ok(())
}

fn mount_dev(rootfs: &Path) -> Result<()> {
    let dev_path = rootfs.join("dev");
    fs::create_dir_all(&dev_path).context("Failed to create dev directory")?;

    // Mount basic tmpfs for /dev
    mount(
        Some("tmpfs"),
        &dev_path,
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_STRICTATIME,
        Some("mode=755"),
    ).context("Failed to mount dev tmpfs")?;

    // creating minimal nodes
    create_base_device_nodes(&dev_path)?;
    Ok(())
}

fn create_base_device_nodes(dev_path: &Path) -> Result<()> {
    let devices = [
        ("null", 1, 3),   // /dev/null : this discards all data written to it 
        ("urandom", 1, 9),// /dev/urandom : critical for security sensitive apps
        ("tty", 5, 0),    // /dev/tty : interactive terminal for the container
    ];

    for (name, major, minor) in devices {
        let path = dev_path.join(name);
        let path_str = path.to_str()
            .with_context(|| format!("Failed to convert path to string: {:?}", path))?;
        let c_path = CString::new(path_str)
            .with_context(|| format!("Failed to create CString from path: {}", path_str))?;
            
        mknod(
            &c_path, 
            SFlag::S_IFCHR,
            Mode::from_bits(0o666).unwrap(),
            nix::libc::makedev(major, minor),
        ).context(format!("failed to create device {}", name))?;
    }
    Ok(())
}
