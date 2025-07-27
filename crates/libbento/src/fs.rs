use anyhow::{Context, Result};
use nix::{unistd, Error,sys::stat::{mknod, Mode, SFlag, sys}, mount::{statvfs, mount, umount, umount2, MsFlags, MntFlags}};
use std::{fs, ffi::CString, Path::{Path, PathBuf}};


// This func creates the rootfs dir.
fn prepare_rootfs(container_id: &str) -> Result<PathBuf> {
    let rootfs = Path::new("/var/lib/container").join(container_id).join("rootfs");
    fs::create_dir_all(&rootfs)
        .context("Failed to create the rootfs directory.")?;
    
    let old_root = rootfs.join(".old_root");
    fs::create_dir_all(&old_root)
        .context("Failed to create old_root directory")?;

    // Make the rootfs path a bind mount to itself
    mount(
        Some(&rootfs),
        &rootfs,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    ).context("Failed to bind mount rootfs")?;


    // Here we are trying to verify the mount
    let stat = mount::statvfs(rootfs)
    .context("Failed to get filesystem stats")?;

    // Check if it's a bind mount by looking at mount flags
    if stat.flags().contains(MsFlags::MS_BIND) {
             println!("Bind mount successful");
    } else {
             return Err(anyhow::anyhow!("Bind mount verification failed"));
    }

    mount_proc(&rootfs).context("Failed to mount proc")?;
    mount_sys(&rootfs).context("Failed to mount sys")?;
    mount_dev(&rootfs).context("Failed to mount dev")?;

    // moving the current root to the new rootfs
    unistd::pivot_root(&rootfs, &old_root)
        .context("Failed to pivot root")?;

    // Change working dir to new root
    unistd::chdir("/")
        .context("Failed to change directory to new root")?;

    // unmount the old root
    mount(
        None::<&str>,
        "/.old_root",
        None::<&str>,
        MsFlags::MS_DETACH | MsFlags::MS_PRIVATE,
        None::<&str>,
    ).context("Failed to unmount old root")?;

    // Here i am trying to verify the unmount : it's slightly different than mount verification
    let unmount_verified = match mount::statvfs("/.old_root") {
        Ok(stats) => {
             false
        },
        Err(Error::Sys(nix::errno::Errno::ENOENT)) => {
             true // path doesnt exist - unmount successful
        },
        Err(_) => {
             false // for other errors
        }
    };

    if !unmount_verified {
         return Err(anyhow::anyhow!("Unmount verification failed - /.old_root still mounted"));
    }

    // Remove the old root directory
    fs::remove_dir_all("/.old_root")
        .context("Failed to remove old root directory")?;
    
    Ok(rootfs)
}

fn mount_proc(rootfs: &Path) -> Result<()> {
    let proc_path = rootfs.join("proc");
    fs::create_dir_all(&proc_path).context("Failed to create proc directory")?;

    mount(
        Some("proc"),
        &proc_path,
        Some("proc"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
        None::<&str>,
    ).context("Failed to mount proc filesystem")?;

    let stat = statvfs(&proc_path).context("Failed to get proc filesystem stats")?;
        if stat.filesystem_type() != nix::mount::PROC_SUPER_MAGIC {
              return Err(anyhow::anyhow!("Proc filesystem verification failed"));
    }
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

    let stat = statvfs(&sys_path).context("Failed to get sys filesystem stats")?;
        if stat.filesystem_type() != nix::mount::SYSFS_MAGIC {
             return Err(anyhow::anyhow!("Sysfs verification failed"));
    }
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
        MsFlags::MS_NOSUID |MsFlags::MS_NOEXEC | MsFlags::MS_STRICTATIME,
        Some("mode=755, size=64k"),
    ).context("Failed to mount dev tmpfs")?;

    // creating minimal nodes
    create_base_device_nodes(&dev_path)?;


    let stat = statvfs(&dev_path).context("Failed to get dev filesystem stats")?;
        if stat.filesystem_type() != nix::mount::TMPFS_MAGIC {
              return Err(anyhow::anyhow!("Tmpfs verification failed"));
        }

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
            Mode::S_IRUSR | Mode::S_IWUSR | Mode::S_IRGRP | Mode::S_IWGRP | Mode::S_IROTH | Mode::S_IWOTH,
            nix::libc::makedev(major, minor),
        ).context(format!("failed to create device {}", name))?;

        verify_device_node(&path, major, minor)
            .context(format!("Verification failed for device {}", name))?;
    }
    Ok(())
}

fn verify_device_node(path: &Path, expected_major: u64, expected_minor: u64) -> Result<()> {
    let metadata = fs::metadata(path)
        .context(format!("Failed to get metadata for {:?}", path))?;
    
    if !metadata.file_type().is_char_device() {
        return Err(anyhow::anyhow!("{:?} is not a character device", path));
    }

    let stat = sys::stat::stat(path)
        .context(format!("Failed to stat device {:?}", path))?;
    
    let actual_dev = stat.st_rdev;
    let expected_dev = nix::libc::makedev(expected_major, expected_minor);
    
    if actual_dev != expected_dev {
        return Err(anyhow::anyhow!(
            "Device {:?} has wrong device numbers (expected {}:{}, got {})",
            path,
            expected_major,
            expected_minor,
            actual_dev
        ));
    }

    Ok(())
}

/// Unmounts pseudo filesystems
pub fn cleanup_fs(rootfs: &Path) -> Result<()> {
    // Unmounting in reverse order of mounting
    force_unmount(rootfs.join("dev")).context("Failed to unmount dev")?;
    force_unmount(rootfs.join("sys")).context("Failed to unmount sys")?;
    force_unmount(rootfs.join("proc")).context("Failed to unmount proc")?;
    force_unmount(rootfs).context("Failed to unmount rootfs")?;
    
    Ok(())
}

// Here we are using umount2 as its more reliable, and wont fail like umount.
fn force_unmount(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let path_str = path.to_str().context("Invalid path encoding")?;

    // Try normal unmount first
    if let Err(e) = umount(path_str) {
        if e == nix::errno::Errno::EINVAL {
            return Ok(()); // Already unmounted
        }
        // Trying lazy unmount if normal fails
        umount2(path_str, MntFlags::MNT_DETACH)
            .context(format!("Failed to unmount {:?}", path))?;
    }

    // Verification
    match statvfs(path) {
        Ok(_) => Err(anyhow::anyhow!("{:?} still mounted after unmount", path)),
        Err(Error::Sys(nix::errno::Errno::ENOENT)) => Ok(()), // Success
        Err(e) => Err(e.into()),
    }
}
