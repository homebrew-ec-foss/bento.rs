use anyhow::{Context, Result};
use nix::{unistd, Error,sys::stat::{mknod, Mode, SFlag, sys}, mount::{statvfs, mount, umount, umount2, MsFlags, MntFlags}};
use std::{fs, ffi::CString, path::{Path, PathBuf}, io};

fn get_rootfs(container_id : &str) -> Result<(PathBuf,PathBuf)>{
    let rootfs = PathBuf::from(format!("/var/lib/container/{}/rootfs", container_id));

    fs::create_dir_all(&rootfs)
        .context("Failed to create the rootfs directory.")?;
      
    let old_root = rootfs.join("old_root");
             fs::create_dir_all(&old_root)
                    .context("Failed to create old_root directory")?;
    Ok((rootfs, old_root))
}

// this is to verify the rootfs mount
fn is_bind_mount(path: &str) -> Result<bool> {
    let file = fs::File::open("/proc/self/mountinfo").context("Failed to open /proc/self/mountinfo")?;
    let reader = io::BufReader::new(file);
    let target = Path::new(path).canonicalize().context("Failed to canonicalize path")?;
    let target_str = target.to_str().context("Invalid path encoding")?;

    for line in reader.lines() {
        let line = line.context("Failed to read mountinfo line")?;
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 10 && fields[4] == target_str && fields[9].contains("bind") {
            return Ok(true);
        }
    }
    Ok(false)
}

// This func creates the rootfs dir - call this function from the container creation process with
// container_id as argument.

fn prepare_rootfs(container_id: &str) -> Result<PathBuf> {
    
    if container_id.contains("..") || container_id.contains('/') {
        return Err(anyhow::anyhow!("Invalid container_id: {}", container_id));
    }   
    
    let (rootfs,old_root) = get_rootfs(container_id);
    
    // Make the rootfs path a bind mount to itself
    mount(
        Some(&rootfs),
        &rootfs,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    ).context("Failed to bind mount rootfs")?;

    if !is_bind_mount(&rootfs) {
            return Err(anyhow::anyhow!("rootfs mount failed"));
    } 

    mount_proc(&rootfs).context("Failed to mount proc")?;
    mount_sys(&rootfs).context("Failed to mount sys")?;
    mount_dev(&rootfs).context("Failed to mount dev")?;

    // moving the current root to the new rootfs
    if let Err(e) = unistd::pivot_root(&rootfs, &old_root) {
              cleanup_fs(&rootfs).context("failed to cleanup after pivot error");
              return Err(e).context("failed to cleanup after pivot error");
    }

    // Change working dir to new root
    unistd::chdir("/")
        .context("Failed to change directory to new root")?;

    // Checking if old_root path exists before unmounting it.
    if !Path::new("/.old_root").exists() {
           return Err(anyhow::anyhow!("Old root path does not exist before unmount"));
    }

    umount2("/.old_root", MntFlags::MNT_DETACH).context("Failed to unmount old root")?;

    if statvfs("/.old_root").is_ok() {
        return Err(anyhow::anyhow!("Unmount verification failed - '/.old_root' still mounted"));
    }
   
    // Remove the old root directory
    fs::remove_dir_all("/.old_root")
        .context("Failed to remove old root directory")?;
 
    fs::remove_dir_all("/.old_root").context("Failed to remove old root directory")?;
      
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
        ("null", 1, 3, Mode::from_bits_truncate(0o666)),   // /dev/null : this discards all data written to it : read + write for all 
        ("urandom", 1, 9, Mode::from_bits_truncate(0o666)),// /dev/urandom : critical for security sensitive apps : read + write for all 
        ("tty", 5, 0, Mode::from_bits_truncate(0o600)),    // /dev/tty : interactive terminal for the container : read + write only for owner
    ];

    for (name, major, minor, mode) in devices {
        let path = dev_path.join(name);
        let path_str = path.to_str()
            .with_context(|| format!("Failed to convert path to string: {:?}", path))?;
        let c_path = CString::new(path_str)
            .with_context(|| format!("Failed to create CString from path: {}", path_str))?;
            
        mknod(
            &c_path, 
            SFlag::S_IFCHR,
            mode,
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
    fs::remove_dir_all(&rootfs).context("Failed to remove the rootfs dir.")?;
    Ok(())
}

// Here we are using umount2 as its more reliable, and wont fail like umount.
fn force_unmount(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let path_str = path.to_str().context("Invalid path encoding")?;
    for _ in 0..3 {
        match umount2(path_str, MntFlags::MNT_DETACH) {
            Ok(_) => break,
            Err(Error::Sys(nix::errno::Errno::EBUSY)) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            Err(Error::Sys(nix::errno::Errno::EINVAL)) => return Ok(()),
            Err(e) => return Err(e).context(format!("Failed to unmount {:?}", path)),
        }
    }
    match statvfs(path) {
        Ok(_) => Err(anyhow::anyhow!("{:?} still mounted after unmount", path)),
        Err(Error::Sys(nix::errno::Errno::ENOENT)) => Ok(()),
        Err(e) => Err(e).context(format!("Failed to verify unmount of {:?}", path)),
    }
}
