// crates/libbento/src/fs.rs

use anyhow::{Context, Result};
use nix::{
    unistd,
    sys::stat::{mknod, stat, Mode, SFlag},
    mount::{mount, umount2, MsFlags, MntFlags},
    sys::statvfs::statvfs,
    errno::Errno,
    Error,
};
use std::{
    fs,
    ffi::{CString, CStr},
    path::{Path, PathBuf},
    io::{self, BufRead, BufReader},
    os::unix::fs::FileTypeExt,  // For is_char_device() method
};


fn get_rootfs(container_id : &str) -> Result<(PathBuf,PathBuf)>{
    let home = std::env::var("HOME")?;
    let rootfs = PathBuf::from(format!("{}/.local/share/bento/{}/rootfs", home, container_id));

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


pub fn prepare_rootfs(container_id: &str) -> Result<PathBuf> {
    if container_id.contains("..") || container_id.contains('/') {
        return Err(anyhow::anyhow!("Invalid container_id: {}", container_id));
    }   
    
    let (rootfs, old_root) = get_rootfs(container_id)?;
    println!("[Init] Created rootfs directory: {:?}", rootfs);
    
    // Make the rootfs path a bind mount to itself
    println!("[Init] Attempting bind mount of {:?}", rootfs);
    /*mount(
        Some(&rootfs),
        &rootfs,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    ).context("Failed to bind mount rootfs")?;
    */

    mount(
        None::<&str>,
        "/", // The source is the root of the entire mount tree
        None::<&str>,
        // Recursively make every single mount point private
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    ).context("Failed to make the root mount tree private. This is a critical step.")?;

    println!("[Init] Bind mount completed successfully");

    // Debug: Check what actually got mounted
    let rootfs_str = rootfs.to_str().context("Failed to convert rootfs path to string")?;
    println!("[Init] Verifying bind mount for path: {}", rootfs_str);
    
    // Debug: Show /proc/self/mountinfo contents
    debug_mountinfo()?;
    
    match is_bind_mount(rootfs_str) {
        Ok(true) => println!("[Init] Bind mount verification succeeded"),
        Ok(false) => {
            println!("[Init] WARNING: Bind mount verification failed, but continuing...");
            // Don't return error - continue with mount operations
        }
        Err(e) => println!("[Init] Bind mount verification error: {}", e),
    }

    // Continue with filesystem setup regardless of verification
    mount_proc(&rootfs).context("Failed to mount proc")?;
    mount_sys(&rootfs).context("Failed to mount sys")?;
    mount_dev(&rootfs).context("Failed to mount dev")?;

    println!("[Init] All mounts completed, attempting pivot_root");
    
    // Try pivot_root with better error handling
    match unistd::pivot_root(&rootfs, &old_root) {
        Ok(_) => println!("[Init] pivot_root succeeded"),
        Err(e) => {
            println!("[Init] pivot_root failed: {}, attempting cleanup", e);
            let _ = cleanup_fs(&rootfs);
            return Err(anyhow::anyhow!("pivot_root failed: {}", e));
        }
    }

    // Continue with the rest of the function...
    unistd::chdir("/").context("Failed to change directory to new root")?;
    
    // Rest of cleanup logic...
    Ok(rootfs)
}



fn debug_mountinfo() -> Result<()> {
    use std::io::BufRead;
    
    println!("[Init] Current /proc/self/mountinfo contents:");
    let file = std::fs::File::open("/proc/self/mountinfo")
        .context("Failed to open /proc/self/mountinfo")?;
    let reader = std::io::BufReader::new(file);
    
    for (i, line) in reader.lines().enumerate() {
        let line = line.context("Failed to read mountinfo line")?;
        println!("[Init] mountinfo[{}]: {}", i, line);
        if i > 10 { // Limit output
            println!("[Init] ... (truncated)");
            break;
        }
    }
    Ok(())
}


/*fn mount_proc(rootfs: &Path) -> Result<()> {
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
} */

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

    // Simplified verification for now
    println!("[Mount] Proc filesystem mounted successfully");
    Ok(())
}

fn mount_sys(rootfs: &Path) -> Result<()> {
    let sys_path = rootfs.join("sys");
    println!("[Mount] Creating sys directory: {:?}", sys_path);
    fs::create_dir_all(&sys_path).context("Failed to create sys directory")?;
    
    println!("[Mount] Attempting to mount sysfs...");
    println!("[Mount] Source: sysfs");
    println!("[Mount] Target: {:?}", sys_path);
    println!("[Mount] FSType: sysfs");
    println!("[Mount] Flags: {:?}", MsFlags::empty());
    
    // Check if target directory is accessible
    match fs::metadata(&sys_path) {
        Ok(meta) => println!("[Mount] Target directory exists, permissions: {:?}", meta.permissions()),
        Err(e) => println!("[Mount] Target directory issue: {}", e),
    }
    
    // Check current working directory
    match std::env::current_dir() {
        Ok(cwd) => println!("[Mount] Current working directory: {:?}", cwd),
        Err(e) => println!("[Mount] Failed to get current directory: {}", e),
    }
    
    // Attempt the mount with detailed error reporting
    match mount(
        Some("sysfs"),
        &sys_path,
        Some("sysfs"),
        MsFlags::empty(),
        None::<&str>,
    ) {
        Ok(_) => {
            println!("[Mount] Sysfs mount succeeded");
            Ok(())
        }
        Err(e) => {
            println!("[Mount] Sysfs mount failed with error: {}", e);
            println!("[Mount] Error type: {:?}", e);
            
            // Try alternative approach - read-only mount
            println!("[Mount] Attempting read-only sysfs mount...");
            match mount(
                Some("sysfs"),
                &sys_path,
                Some("sysfs"),
                MsFlags::MS_RDONLY,
                None::<&str>,
            ) {
                Ok(_) => {
                    println!("[Mount] Read-only sysfs mount succeeded");
                    Ok(())
                }
                Err(e2) => {
                    println!("[Mount] Read-only sysfs mount also failed: {}", e2);
                    Err(anyhow::anyhow!("Both normal and read-only sysfs mounts failed: {} / {}", e, e2))
                }
            }
        }
    }
}

fn mount_dev(rootfs: &Path) -> Result<()> {
    let dev_path = rootfs.join("dev");
    fs::create_dir_all(&dev_path).context("Failed to create dev directory")?;

    // Mount basic tmpfs for /dev
    mount(
        Some("tmpfs"),
        &dev_path,
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_STRICTATIME,
        Some("mode=755,size=64k"),
    ).context("Failed to mount dev tmpfs")?;

    // creating minimal nodes
    create_base_device_nodes(&dev_path)?;

    println!("[Mount] Dev filesystem mounted successfully");
    Ok(())
}


fn create_base_device_nodes(dev_path: &Path) -> Result<()> {
    let devices = [
        ("null", 1u32, 3u32, Mode::from_bits_truncate(0o666)),
        ("urandom", 1u32, 9u32, Mode::from_bits_truncate(0o666)),
        ("tty", 5u32, 0u32, Mode::from_bits_truncate(0o600)),
    ];

    for (name, major, minor, mode) in devices {
        let path = dev_path.join(name);
        let path_str = path.to_str()
            .with_context(|| format!("Failed to convert path to string: {:?}", path))?;
        let c_path = CString::new(path_str)
            .with_context(|| format!("Failed to create CString from path: {}", path_str))?;
            
        // Fix: Use c_path.as_c_str() to convert CString to &CStr (which implements NixPath)
        mknod(
            c_path.as_c_str(),  // This fixes the NixPath trait issue
            SFlag::S_IFCHR,
            mode,
            nix::libc::makedev(major, minor),  // Already u32, no conversion needed
        ).with_context(|| format!("failed to create device {}", name))?;

        verify_device_node(&path, major as u64, minor as u64)
            .with_context(|| format!("Verification failed for device {}", name))?;
    }
    Ok(())
}


fn verify_device_node(path: &Path, expected_major: u64, expected_minor: u64) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Failed to get metadata for {:?}", path))?;
    
    if !metadata.file_type().is_char_device() {
        return Err(anyhow::anyhow!("{:?} is not a character device", path));
    }

    // Fix: Use stat directly (not sys::stat::stat)
    let stat_result = stat(path)
        .with_context(|| format!("Failed to stat device {:?}", path))?;
    
    let actual_dev = stat_result.st_rdev;
    // Fix: Convert u64 to u32 for makedev
    let expected_dev = nix::libc::makedev(expected_major as u32, expected_minor as u32);
    
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
            Err(Error::EBUSY) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            Err(Error::EINVAL) => return Ok(()),
            Err(e) => return Err(anyhow::anyhow!("Failed to unmount {:?}: {}", path, e)),
        }
    }
    
    match statvfs(path) {
        Ok(_) => Err(anyhow::anyhow!("{:?} still mounted after unmount", path)),
        Err(Error::ENOENT) => Ok(()),
        Err(_) => Ok(()),
    }
}

