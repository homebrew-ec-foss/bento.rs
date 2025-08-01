// crates/libbento/src/fs.rs

use crate::process::{Config, RootfsPopulationMethod};
use anyhow::{Context, Result};
use nix::{
    mount::{MntFlags, MsFlags, mount, umount2},
    sys::stat::{Mode, SFlag, mknod},
    unistd,
};
use std::os::unix::fs::PermissionsExt;
use std::{
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

fn get_rootfs(container_id: &str) -> Result<(PathBuf, PathBuf)> {
    let home = std::env::var("HOME")?;
    let rootfs = PathBuf::from(format!(
        "{home}/.local/share/bento/{container_id}/rootfs"
    ));

    fs::create_dir_all(&rootfs).context("Failed to create the rootfs directory.")?;

    let old_root = rootfs.join("old_root");
    fs::create_dir_all(&old_root).context("Failed to create old_root directory")?;

    Ok((rootfs, old_root))
}

// FIXED: Added config parameter to function signature
pub fn prepare_rootfs(container_id: &str, config: &Config) -> Result<PathBuf> {
    println!(
        "[Init] Starting rootless-aware rootfs preparation for: {container_id}"
    );

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
        return Err(anyhow::anyhow!("Invalid container_id: {}", container_id));
    }

    let (rootfs, old_root) = get_rootfs(container_id)?;
    println!("[Init] Rootfs: {rootfs:?}, Old root: {old_root:?}");

    // Now this works because config parameter exists
    populate_rootfs_binaries(&rootfs, &config.population_method)?;

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
    rootless_mount_proc(&rootfs)?;
    rootless_mount_sys(&rootfs)?;
    rootless_mount_dev(&rootfs)?;

    // Phase 4: Switch to container filesystem
    println!("[Init] Executing pivot_root");
    unistd::pivot_root(&rootfs, &old_root).context("pivot_root failed in rootless container")?;

    unistd::chdir("/").context("Failed to chdir to new root")?;

    // Phase 5: Clean up old root
    cleanup_old_root()?;

    println!("[Init] Rootless container filesystem ready");
    Ok(PathBuf::from("/"))
}

// Strategy pattern for rootfs population
fn populate_rootfs_binaries(rootfs: &Path, method: &RootfsPopulationMethod) -> Result<()> {
    match method {
        RootfsPopulationMethod::Manual => populate_manual_binaries(rootfs),
        RootfsPopulationMethod::BusyBox => populate_busybox_binaries(rootfs),
    }
}

// Manual binary population implementation
fn populate_manual_binaries(rootfs: &Path) -> Result<()> {
    let bin_dir = rootfs.join("bin");
    let usr_bin_dir = rootfs.join("usr/bin");
    let lib_dir = rootfs.join("lib");
    let lib64_dir = rootfs.join("lib64");

    // Create comprehensive directory structure
    for dir in [&bin_dir, &usr_bin_dir, &lib_dir, &lib64_dir] {
        std::fs::create_dir_all(dir)?;
    }

    println!("[Rootfs] Using manual binary population method");

    // Essential Unix utilities with their typical locations
    let essential_binaries = [
        // Core shell utilities
        ("/bin/sh", bin_dir.join("sh")),
        ("/bin/cat", bin_dir.join("cat")),
        ("/bin/ls", bin_dir.join("ls")),
        ("/bin/echo", bin_dir.join("echo")),
        // System information utilities
        ("/usr/bin/id", usr_bin_dir.join("id")),
        ("/bin/hostname", bin_dir.join("hostname")),
        ("/usr/bin/whoami", usr_bin_dir.join("whoami")),
        ("/usr/bin/env", usr_bin_dir.join("env")),
        // File utilities
        ("/usr/bin/head", usr_bin_dir.join("head")),
        ("/usr/bin/tail", usr_bin_dir.join("tail")),
        ("/bin/mount", bin_dir.join("mount")),
        ("/bin/ps", bin_dir.join("ps")),
    ];

    // Copy binaries with error resilience
    let mut successful_copies = 0;
    for (source, dest) in &essential_binaries {
        if Path::new(source).exists() {
            match std::fs::copy(source, dest) {
                Ok(_) => {
                    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
                    println!("[Rootfs] Copied binary: {} -> {}", source, dest.display());
                    successful_copies += 1;
                }
                Err(e) => {
                    println!("[Rootfs] Warning: Failed to copy {source}: {e}");
                }
            }
        } else {
            println!("[Rootfs] Warning: {source} not found on host");
        }
    }

    // Copy shared libraries (essential for dynamic binaries)
    copy_shared_libraries(&lib_dir, &lib64_dir)?;

    println!(
        "[Rootfs] Manual population complete: {successful_copies} binaries copied"
    );
    Ok(())
}

// BusyBox population implementation - single, clean version
fn populate_busybox_binaries(rootfs: &Path) -> Result<()> {
    let bin_dir = rootfs.join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    println!("[Rootfs] Using BusyBox static binary method");

    // Look for BusyBox in multiple locations
    let busybox_locations = [
        "/tmp/bento-test-rootfs/bin/busybox", // From your earlier setup
        "/tmp/busybox-static",                // Alternative location
        "./busybox-static",                   // Local copy
    ];

    let mut busybox_source = None;
    for location in &busybox_locations {
        if Path::new(location).exists() {
            busybox_source = Some(*location);
            break;
        }
    }

    let busybox_path = busybox_source.ok_or_else(|| {
        anyhow::anyhow!(
            "BusyBox binary not found. Please download:\n\
            wget https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox -O /tmp/busybox-static\n\
            chmod +x /tmp/busybox-static"
        )
    })?;

    // Copy BusyBox binary
    let busybox_dest = bin_dir.join("busybox");
    std::fs::copy(busybox_path, &busybox_dest)?;
    std::fs::set_permissions(&busybox_dest, std::fs::Permissions::from_mode(0o755))?;
    println!("[Rootfs] Copied BusyBox binary: {}", busybox_dest.display());

    // Create comprehensive command symlinks
    let busybox_commands = [
        // Essential shell commands
        "sh", "ash", "bash", // File operations
        "cat", "ls", "cp", "mv", "rm", "mkdir", "rmdir", "touch", // Text processing
        "echo", "printf", "grep", "sed", "awk", "cut", "sort", "uniq", "head", "tail",
        // System information
        "id", "whoami", "hostname", "uname", "uptime", "ps", "top", // File system
        "mount", "umount", "df", "du", "find", "which",
        // Network utilities (for future enhancements)
        "ping", "wget", "netstat", // Archive utilities
        "tar", "gzip", "gunzip", // System utilities
        "env", "sleep", "kill", "pkill",
    ];

    let mut created_commands = 0;
    for cmd in &busybox_commands {
        let link_path = bin_dir.join(cmd);

        // Remove existing file/link if present
        let _ = std::fs::remove_file(&link_path);

        match std::os::unix::fs::symlink("busybox", &link_path) {
            Ok(()) => {
                created_commands += 1;
                println!("[Rootfs] Created command symlink: {cmd}");
            }
            Err(e) => {
                println!(
                    "[Rootfs] Warning: Failed to create symlink for {cmd}: {e}"
                );
            }
        }
    }

    println!(
        "[Rootfs] BusyBox setup complete: {created_commands} commands available"
    );
    Ok(())
}

// Shared library copying for manual method
fn copy_shared_libraries(lib_dir: &Path, lib64_dir: &Path) -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    // Critical shared libraries for most Unix utilities
    let essential_libraries = [
        // Dynamic linker (most critical)
        (
            "/lib64/ld-linux-x86-64.so.2",
            lib64_dir.join("ld-linux-x86-64.so.2"),
        ),
        // Core C library
        (
            "/lib/x86_64-linux-gnu/libc.so.6",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libm.so.6",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libdl.so.2",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libpthread.so.0",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        // PCRE2 libraries (NEW - these are what's missing!)
        (
            "/lib/x86_64-linux-gnu/libpcre2-8.so.0",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libpcre.so.3",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libpcre2-32.so.0",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        // System utilities libraries
        (
            "/lib/x86_64-linux-gnu/libnss_files.so.2",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libutil.so.1",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        // Additional libraries (add these!)
        (
            "/lib/x86_64-linux-gnu/libselinux.so.1",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libpcre.so.1",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libcap.so.2",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libcrypt.so.1",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libresolv.so.2",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libnss_files.so.2",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libnss_dns.so.2",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/libutil.so.1",
            lib_dir.join("x86_64-linux-gnu"),
        ),
        (
            "/lib/x86_64-linux-gnu/librt.so.1",
            lib_dir.join("x86_64-linux-gnu"),
        ),
    ];

    for (source_path, target_base) in essential_libraries {
        let source = Path::new(source_path);
        if !source.exists() {
            println!("[Rootfs] Library {source_path} not found, skipping");
            continue;
        }

        // Handle lib64 vs lib directory structure
        let target_file = if source_path.contains("lib64") {
            target_base
        } else {
            let arch_dir = target_base;
            fs::create_dir_all(&arch_dir)?;
            arch_dir.join(source.file_name().unwrap())
        };

        match fs::copy(source, &target_file) {
            Ok(_) => {
                let mut perms = fs::metadata(&target_file)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&target_file, perms)?;
                println!("[Rootfs] Copied library: {}", target_file.display());
            }
            Err(e) => {
                println!(
                    "[Rootfs] Warning: Failed to copy library {source_path}: {e}"
                );
            }
        }
    }

    Ok(())
}

// Remaining mount and cleanup functions (unchanged from your working version)
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
            println!("[Mount] /proc mounted successfully (rootless)");
            Ok(())
        }
        Err(e) => {
            println!(
                "[Mount] /proc mount failed: {e}, creating minimal structure"
            );
            create_minimal_proc_structure(&proc_path)?;
            Ok(())
        }
    }
}

fn rootless_mount_sys(rootfs: &Path) -> Result<()> {
    mount_sys_progressive(rootfs)
}

fn mount_sys_progressive(rootfs: &Path) -> Result<()> {
    let sys_path = rootfs.join("sys");
    fs::create_dir_all(&sys_path)?;

    println!("[Mount] Setting up /sys with progressive security strategy");

    // Strategy 1: Try real sysfs mount (ideal but often fails in rootless)
    if attempt_real_sysfs_mount(&sys_path).is_ok() {
        println!("[Mount] Real sysfs mounted successfully");
        return Ok(());
    }

    // Strategy 2: Mount tmpfs and populate with essential fake files
    if mount_tmpfs_sys_with_content(&sys_path).is_ok() {
        println!("[Mount] /sys mounted as populated tmpfs (secure isolation)");
        return Ok(());
    }

    // Strategy 3: Fallback to directory structure with fake files
    create_populated_sys_directories(&sys_path)?;
    println!("[Mount] Created populated /sys directory structure (fallback)");
    Ok(())
}

fn attempt_real_sysfs_mount(sys_path: &Path) -> Result<()> {
    mount(
        Some("sysfs"),
        sys_path,
        Some("sysfs"),
        MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
        None::<&str>,
    )
    .context("Real sysfs mount failed")
}

fn mount_tmpfs_sys_with_content(sys_path: &Path) -> Result<()> {
    // Mount read-only tmpfs
    mount(
        Some("tmpfs"),
        sys_path,
        Some("tmpfs"),
        MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("size=2M,mode=755"), // 2MB should be plenty
    )
    .context("Failed to mount tmpfs for /sys")?;

    // Temporarily remount as writable to populate, then make read-only
    mount(
        None::<&str>,
        sys_path,
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("size=2M,mode=755"),
    )?;

    // Populate with essential content
    populate_tmpfs_sys_content(sys_path)?;

    // Make read-only again
    mount(
        None::<&str>,
        sys_path,
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("size=2M,mode=755"),
    )?;

    Ok(())
}

fn populate_tmpfs_sys_content(sys_path: &Path) -> Result<()> {
    // Essential directories that applications expect
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

    // Essential files with realistic fake content
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

    println!("[Mount] Populated tmpfs /sys with essential fake files");
    Ok(())
}

fn create_populated_sys_directories(sys_path: &Path) -> Result<()> {
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

    // Create the same fake files as in tmpfs version
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

    println!("[Mount] Created populated /sys directory structure with fake files");
    Ok(())
}

fn rootless_mount_dev(rootfs: &Path) -> Result<()> {
    let dev_path = rootfs.join("dev");
    fs::create_dir_all(&dev_path)?;

    println!("[Mount] Setting up /dev for rootless container");

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

    match create_device_nodes_if_possible(&dev_path) {
        Ok(_) => println!("[Mount] Successfully created device nodes"),
        Err(_) => {
            println!("[Mount] Device node creation failed (expected in rootless)");
            create_rootless_dev_structure(&dev_path)?;
        }
    }

    Ok(())
}

fn try_bind_mount_host_dev(dev_path: &Path) -> Result<()> {
    println!("[Mount] Attempting bind mount of host /dev");

    match mount(
        Some("/dev"),
        dev_path,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    ) {
        Ok(_) => {
            println!("[Mount] Successfully bind mounted host /dev (ideal solution)");
            Ok(())
        }
        Err(e) => {
            println!("[Mount] Bind mount of /dev failed: {e}");
            Err(e.into())
        }
    }
}

fn create_device_nodes_if_possible(dev_path: &Path) -> Result<()> {
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
    println!("[Mount] Creating rootless-compatible /dev structure");

    let dirs = ["pts", "shm", "mqueue"];
    for dir in &dirs {
        fs::create_dir_all(dev_path.join(dir))?;
    }

    let fake_devices = [
        ("null", ""),
        ("zero", ""),
        ("urandom", "random data placeholder"),
        ("random", "random data placeholder"),
        ("tty", ""),
    ];

    for (name, content) in &fake_devices {
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
            println!(
                "[Mount] Warning: Failed to create symlink {link_name}: {e}"
            );
        } else {
            println!("[Mount] Created symlink: /dev/{link_name} -> {target}");
        }
    }
    Ok(())
}

fn create_minimal_proc_structure(proc_path: &Path) -> Result<()> {
    let dirs = ["self", "sys", "net"];
    for dir in &dirs {
        fs::create_dir_all(proc_path.join(dir))?;
    }

    fs::write(proc_path.join("version"), "Container Linux version\n")?;
    fs::write(proc_path.join("uptime"), "1.0 1.0\n")?;

    Ok(())
}

fn cleanup_old_root() -> Result<()> {
    println!("[Init] Cleaning up old root");

    match umount2("/old_root", MntFlags::MNT_DETACH) {
        Ok(_) => println!("[Init] Old root unmounted"),
        Err(e) => println!("[Init] Warning: Failed to unmount old root: {e}"),
    }

    match fs::remove_dir_all("/old_root") {
        Ok(_) => println!("[Init] Old root directory removed"),
        Err(e) => println!("[Init] Warning: Failed to remove old root: {e}"),
    }

    Ok(())
}
