use anyhow::{Context, Result};
use nix::{
    mount::{MntFlags, MsFlags, mount, umount2},
    sys::stat::{Mode, SFlag, mknod},
    unistd,
};
use std::{
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};
use serde_json;

fn get_rootfs(container_id : &str) -> Result<(PathBuf, PathBuf)> {

    let config_path = PathBuf::from(format!("/run/container/{container_id}/config.json"));
    let config_content = fs::read_to_string(&config_path)?;
    let config : serde_json::Value = serde_json::from_str(&config_content)?;
 
    let rootfs_path = match config["root"]["path"].as_str() {
        Some(path) => path,
        None => return Err(anyhow::anyhow!("Missing or invalid root.path in config.json in {container_id}."))
    };

    let rootfs = PathBuf::from(rootfs_path);
    
    fs::create_dir_all(&rootfs).context("Failed to create the rootfs directory.")?;
 
    let old_root = rootfs.join("old_root");
    
    if let Err(e) = fs::create_dir_all(&old_root) {
        let _ = fs::remove_dir_all(&rootfs);
        return Err(e).context("Failed to create old_root - cleared rootfs.")
    }

    Ok((rootfs, old_root))
}

pub fn prepare_rootfs(container_id: &str) -> Result<PathBuf> {
    println!("[Init] Starting rootless-aware rootfs preparation for: {container_id}");

    // Phase 1: Reset mount propagation to prevent host contamination
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("Failed to make root mount tree private")?;

    if container_id.contains("..") || container_id.contains('/') {
        return Err(anyhow::anyhow!("Invalid container_id: {container_id}"));
    }

    let (rootfs, old_root) = get_rootfs(container_id)?;
    println!("[Init] Rootfs: {rootfs:?}, Old root: {old_root:?}");

    // Phase 2: Bind mount rootfs to itself (required for pivot_root)
    mount(
        Some(&rootfs),
        &rootfs,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .context("Failed to bind mount rootfs")?;

    // Phase 3: Mount pseudo-filesystems with rootless-aware strategies
 
    let proc_result = rootless_mount_proc(&rootfs).is_ok();
    let sys_result = rootless_mount_sys(&rootfs).is_ok();
    let dev_result = rootless_mount_dev(&rootfs).is_ok();


    if !proc_result || !sys_result || !dev_result {

        let _ = umount2(&rootfs, MntFlags::MNT_DETACH);
        
        if proc_result {
            let _ = umount2(&rootfs.join("proc"), MntFlags::MNT_DETACH);
        }

        if sys_result {
            let _ = umount2(&rootfs.join("sys"), MntFlags::MNT_DETACH);
        }

        if dev_result {
            let _ = umount2(&rootfs.join("dev"), MntFlags::MNT_DETACH);
        }

        return Err(anyhow::anyhow!("Failed to mount proc : {proc_result} \n sys : {sys_result} \n dev : {dev_result}"))
 
    }

    // Phase 4: Switch to container filesystem
    println!("[Init] Executing pivot_root");
    
    let pivot_result = unistd::pivot_root(&rootfs, &old_root);

    let chdir_result = if pivot_result.is_ok() {
        unistd::chdir("/")
    } else {
        pivot_result
    };

    if let Err(e) = chdir_result {

        let _ = umount2(&rootfs, MntFlags::MNT_DETACH);
        let _ = umount2(&rootfs.join("proc"), MntFlags::MNT_DETACH);
        let _ = umount2(&rootfs.join("sys"), MntFlags::MNT_DETACH);
        let _ = umount2(&rootfs.join("dev"), MntFlags::MNT_DETACH);
 
        let _ = fs::remove_dir_all(&rootfs);
        let _ = fs::remove_dir_all(&old_root);
 
        return Err(e).context("Failed to change the root dir, unmounted complete rootfs and removed rootfs.")
    };

    // Phase 5: Clean up old root
    cleanup_old_root()?;

    println!("[Init] Rootless container filesystem ready");
    Ok(PathBuf::from("/"))
}

fn rootless_mount_proc(rootfs: &Path) -> Result<()> {
    let proc_path = rootfs.join("proc");
    fs::create_dir_all(&proc_path)?;

    match mount(
        Some("proc"),
        &proc_path,
        Some("proc"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
        None::<&str>,
    ) {
        Ok(_) => {
            println!("[Init - Mount] /proc mounted successfully (rootless)");
            Ok(())
        }
        Err(e) => {
            println!("fs failed");
            Err(anyhow::anyhow!("Failed to mount fs : {e}"))
        }
    }
}

fn rootless_mount_sys(rootfs: &Path) -> Result<()> {
    let sys_path = rootfs.join("sys");
    fs::create_dir_all(&sys_path)?;

    println!("[Init] Setting up /sys");

    // Strategy 1: Try real sysfs mount (ideal but often fails in rootless)
    if mount_sysfs(&sys_path).is_ok() {
        println!("[Init] Sysfs mounted successfully");
        return Ok(());
    }

    // Strategy 2: Mount tmpfs and populate with essential directories
    if mount_tmpfs_sys(&sys_path).is_ok() {
        println!("[Init] /sys mounted as populated tmpfs");
        return Ok(());
    }

    // Strategy 3 : Fall back : Creating sub directories
    create_sys_directories(&sys_path)?;
    println!("[Init] Created populated /sys directory structure (fallback)");
    Ok(())
}

// This func to mount sys dir
fn mount_sysfs(sys_path: &Path) -> Result<()> {
    mount(
        Some("sysfs"),
        sys_path,
        Some("sysfs"),
        MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
        None::<&str>,
    )
    .context("Real sysfs mount failed")
}

// This func to mount the tmpfs sys dir
fn mount_tmpfs_sys(sys_path: &Path) -> Result<()> {
 
    // Temporarily remount as writable to populate, then make read-only
    mount(
        None::<&str>,
        sys_path,
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("size=2M,mode=755"),
    )?;

    // Creating the essential sub directories and files in tmpfs sys
    if let Err(e) = add_dir_tmpfs_sys(sys_path){
        let _ = umount2(sys_path, MntFlags::MNT_DETACH);
        return Err(e).context("Failed to mount tmpfs dirs : Sys");
    }

    // Make the mount read-only again
    mount(
        None::<&str>,
        sys_path,
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("size=2M,mode=755"),
    )?;

    Ok(())
}

fn add_dir_tmpfs_sys(sys_path: &Path) -> Result<()> {
    let essential_dirs = [
        "kernel",
        "fs",
        "class",
        "devices",
        "bus",
        "firmware",
        "class/net",
        "class/block",
        "class/tty",
    ];

    for dir in &essential_dirs {
        fs::create_dir_all(sys_path.join(dir))?;
    }

    let essential_files = [
        ("kernel/version", "5.15.0-container #1 SMP Container Kernel"),
        ("kernel/osrelease", "5.15.0-container"),
        ("kernel/hostname", "container"),
        (
            "fs/cgroup/memory/memory.limit_in_bytes",
            "9223372036854775807",
        ),
        ("fs/cgroup/memory/memory.usage_in_bytes", "134217728"),
        ("class/net/lo/operstate", "up"),
        ("devices/system/cpu/online", "0-3"),
    ];

    for (file_path, content) in &essential_files {
        let full_path = sys_path.join(file_path);

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&full_path, content)?;
    }

    println!("[Init] Created the essential sub directories and file in tmpfs /sys.");
    Ok(())
}

fn create_sys_directories(sys_path: &Path) -> Result<()> {
    // Same structure as tmpfs but on regular filesystem
    let essential_dirs = [
        "kernel",
        "fs",
        "class",
        "devices",
        "bus",
        "firmware",
        "class/net",
        "class/block",
        "class/tty",
    ];

    for dir in &essential_dirs {
        fs::create_dir_all(sys_path.join(dir))?;
    }

    let essential_files = [
        ("kernel/version", "5.15.0-container #1 SMP Container Kernel"),
        ("kernel/osrelease", "5.15.0-container"),
        ("kernel/hostname", "container"),
        (
            "fs/cgroup/memory/memory.limit_in_bytes",
            "9223372036854775807",
        ),
        ("class/net/lo/operstate", "up"),
    ];

    for (file_path, content) in &essential_files {
        let full_path = sys_path.join(file_path);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&full_path, content)?;
    }

    println!("[Init] Created essential sub directories and file for /sys directory");
    Ok(())
}

fn rootless_mount_dev(rootfs: &Path) -> Result<()> {
    let dev_path = rootfs.join("dev");
    fs::create_dir_all(&dev_path)?;

    println!("[Init] Setting up /dev for rootless container");

    // Strategy 1: Bind mount host /dev (most compatible)
    if try_bind_mount_host_dev(&dev_path).is_ok() {
        return Ok(());
    }

    // Strategy 2: Create tmpfs and populate with devices
    mount(
        Some("tmpfs"),
        &dev_path,
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_STRICTATIME,
        Some("mode=755,size=64k"),
    )
    .context("Failed to mount tmpfs for /dev")?;

    match create_device_nodes(&dev_path) {
        Ok(_) => println!("[Init] Successfully created device nodes"),
        Err(_) => {
            println!("[Init] Device node creation failed (expected in rootless)");
            create_rootless_dev_structure(&dev_path)?;
        }
    }

    Ok(())
}

fn try_bind_mount_host_dev(dev_path: &Path) -> Result<()> {
    println!("[Init] Attempting bind mount of host /dev");

    match mount(
        Some("/dev"),
        dev_path,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    ) {
        Ok(_) => {
            println!("[Init] Successfully bind mounted host /dev (ideal solution)");
            Ok(())
        }
        Err(e) => {
            println!("[Init] Bind mount of /dev failed: {e}");
            Err(e.into())
        }
    }
}

fn create_device_nodes(dev_path: &Path) -> Result<()> {
    let essential_devices = [
        ("null", 1u32, 3u32, 0o666),
        ("zero", 1u32, 5u32, 0o666),
        ("urandom", 1u32, 9u32, 0o666),
    ];

    for (name, major, minor, mode) in essential_devices {
        let path = dev_path.join(name);
        let c_path = CString::new(path.to_str().unwrap())?;

        mknod(
            c_path.as_c_str(),
            SFlag::S_IFCHR,
            Mode::from_bits_truncate(mode),
            nix::libc::makedev(major, minor),
        )
        .with_context(|| format!("Failed to create device node: {name}"))?;
    }
    Ok(())
}

fn create_rootless_dev_structure(dev_path: &Path) -> Result<()> {
    println!("[Init] Creating rootless-compatible /dev structure");

    let dirs = ["pts", "shm", "mqueue"];
    for dir in &dirs {
        fs::create_dir_all(dev_path.join(dir))?;
    }

    let _devices = [
        ("null", ""),
        ("zero", ""),
        ("urandom", "random data placeholder"),
        ("random", "random data placeholder"),
        ("tty", ""),
    ];

    for (name, content) in &_devices {
        let device_path = dev_path.join(name);
        fs::write(&device_path, content)
            .with_context(|| format!("Failed to create placeholder {name}"))?;
        println!("[Mount] Created placeholder: /dev/{name}");
    }

    create_dev_symlinks(dev_path)?;

    println!("[Mount] Rootless /dev structure complete");
    Ok(())
}

fn create_dev_symlinks(dev_path: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    let symlinks = [
        ("fd", "/proc/self/fd"),
        ("stdin", "/proc/self/fd/0"),
        ("stdout", "/proc/self/fd/1"),
        ("stderr", "/proc/self/fd/2"),
    ];

    for (link_name, target) in &symlinks {
        let link_path = dev_path.join(link_name);
        if let Err(e) = symlink(target, &link_path) {
            println!("[Mount] Warning: Failed to create symlink {link_name}: {e}");
        } else {
            println!("[Mount] Created symlink: /dev/{link_name} -> {target}");
        }
    }
    Ok(())
}

fn cleanup_old_root() -> Result<()> {
    println!("[Init] Cleaning up old root");

    match umount2("/old_root", MntFlags::MNT_DETACH) {
        Ok(_) => println!("[Init] Old root unmounted"),
        Err(e) => {
            println!("[Init] Warning: Failed to unmount old root: {e}");
            if let Err(new) = umount2("/old_root", MntFlags::MNT_DETACH | MntFlags::MNT_FORCE) {
                return Err(new).context("Failed to unmount old root : {new}");
            } else {
                match fs::remove_dir_all("/old_root") {
                    Ok(_) => println!("[Init] Old root directory removed"),
                    Err(e) => println!("[Init] Warning: Failed to remove old root: {e}"),
                }
            }
        }
    }
    
    Ok(())
}
