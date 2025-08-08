use anyhow::{Context, Result, anyhow};
use nix::unistd::Pid;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct CgroupsConfig {
    pub memory_max: Option<String>,
    pub memory_high: Option<String>,
    pub memory_swap_max: Option<String>,
    pub cpu_max: Option<String>,
    pub cpu_weight: Option<u32>,
    pub pids_max: Option<String>,
}

impl CgroupsConfig {
    pub fn new() -> Self {
        Self {
            memory_max: Some("512M".to_string()),
            memory_high: None,
            cpu_max: Some("100000 100000".to_string()),
            pids_max: Some("1000".to_string()),
            cpu_weight: Some(100),
            ..Default::default()
        }
    }

    pub fn unlimited() -> Self {
        Self {
            memory_max: Some("max".to_string()),
            memory_high: None,
            cpu_max: None,
            pids_max: Some("max".to_string()),
            ..Default::default()
        }
    }
}

pub struct CgroupManager {
    container_id: String,
    cgroup_path: PathBuf,
    base_path: PathBuf,
}

impl CgroupManager {
    pub fn new(container_id: String) -> Result<Self> {
        let base_path = get_user_cgroup_base()?;
        let cgroup_path = base_path.join(&container_id);

        Ok(Self {
            container_id,
            cgroup_path,
            base_path,
        })
    }

    pub fn setup(&self, config: &CgroupsConfig, pid: Pid) -> Result<()> {
        println!(
            "[Cgroups] Setting up cgroups for container: {}",
            self.container_id
        );

        self.create_cgroup_directory()?;

        self.enable_controllers()?;

        self.apply_limits(config)?;

        self.add_process(pid)?;

        println!("[Cgroups] Successfully configured cgroups for PID: {}", pid);
        Ok(())
    }

    fn create_cgroup_directory(&self) -> Result<()> {
        if self.cgroup_path.exists() {
            println!(
                "[Cgroups] Cleaning up existing cgroup: {}",
                self.cgroup_path.display()
            );
            self.cleanup_internal()?;
        }

        fs::create_dir_all(&self.cgroup_path).with_context(|| {
            format!(
                "Failed to create cgroup directory: {}",
                self.cgroup_path.display()
            )
        })?;

        println!(
            "[Cgroups] Created cgroup directory: {}",
            self.cgroup_path.display()
        );
        Ok(())
    }

    fn enable_controllers(&self) -> Result<()> {
        let parent_controllers_file = self.base_path.join("cgroup.controllers");
        let available_controllers =
            if let Ok(content) = fs::read_to_string(&parent_controllers_file) {
                content
                    .trim()
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
            } else {
                println!("[Cgroups] Warning: Could not read available controllers, using defaults");
                vec!["cpu".to_string(), "memory".to_string(), "pids".to_string()]
            };

        let desired_controllers = ["cpu", "memory", "pids"];
        let enabled_controllers: Vec<String> = desired_controllers
            .iter()
            .filter(|&controller| available_controllers.contains(&controller.to_string()))
            .map(|&controller| format!("+{}", controller))
            .collect();

        if enabled_controllers.is_empty() {
            println!("[Cgroups] Warning: No controllers available for delegation");
            return Ok(());
        }

        let controllers_str = enabled_controllers.join(" ");
        let subtree_control = self.base_path.join("cgroup.subtree_control");

        let write_result = fs::write(&subtree_control, &controllers_str);
        if let Err(err) = write_result {
            println!(
                "[Cgroups] Initial enable failed at {}: {} — attempting to evacuate processes and retry",
                subtree_control.display(),
                err
            );

            // Fallback: move all processes from the parent cgroup into a leaf child
            // so the parent becomes free of internal processes and can delegate controllers.
            let evac_dir = self.base_path.join("bento-control");
            let _ = fs::create_dir_all(&evac_dir);
            let parent_procs = self.base_path.join("cgroup.procs");
            if let Ok(procs_content) = fs::read_to_string(&parent_procs) {
                for pid_line in procs_content.lines() {
                    let pid_line = pid_line.trim();
                    if pid_line.is_empty() {
                        continue;
                    }
                    let _ = fs::write(evac_dir.join("cgroup.procs"), pid_line);
                }
            }

            fs::write(&subtree_control, &controllers_str).with_context(|| {
                format!(
                    "Failed to enable controllers '{}' in: {}",
                    controllers_str,
                    subtree_control.display()
                )
            })?;
        }

        println!(
            "[Cgroups] Enabled controllers at {}: {}",
            self.base_path.display(),
            controllers_str
        );
        Ok(())
    }

    fn apply_limits(&self, config: &CgroupsConfig) -> Result<()> {
        if let Some(memory_max) = &config.memory_max {
            self.write_cgroup_file("memory.max", memory_max)?;
        }

        if let Some(memory_high) = &config.memory_high {
            self.write_cgroup_file("memory.high", memory_high)?;
        }

        if let Some(swap_max) = &config.memory_swap_max {
            self.write_cgroup_file("memory.swap.max", swap_max)?;
        }

        if let Some(cpu_max) = &config.cpu_max {
            self.write_cgroup_file("cpu.max", cpu_max)?;
        }

        if let Some(cpu_weight) = &config.cpu_weight {
            self.write_cgroup_file("cpu.weight", &cpu_weight.to_string())?;
        }

        if let Some(pids_max) = &config.pids_max {
            self.write_cgroup_file("pids.max", pids_max)?;
        }

        println!("[Cgroups] Applied resource limits");
        Ok(())
    }

    fn add_process(&self, pid: Pid) -> Result<()> {
        let procs_file = self.cgroup_path.join("cgroup.procs");
        fs::write(&procs_file, pid.to_string())
            .with_context(|| format!("Failed to add PID {} to cgroup", pid))?;

        println!("[Cgroups] Added PID {} to cgroup", pid);
        Ok(())
    }

    fn write_cgroup_file(&self, filename: &str, content: &str) -> Result<()> {
        let file_path = self.cgroup_path.join(filename);
        fs::write(&file_path, content)
            .with_context(|| format!("Failed to write to {}: {}", file_path.display(), content))?;

        println!("[Cgroups] Set {}: {}", filename, content);
        Ok(())
    }

    pub fn get_stats(&self) -> Result<CgroupStats> {
        let mut stats = CgroupStats::default();

        if let Ok(content) = fs::read_to_string(self.cgroup_path.join("memory.current")) {
            stats.memory_usage = content.trim().parse().unwrap_or(0);
        }

        if let Ok(content) = fs::read_to_string(self.cgroup_path.join("memory.max")) {
            if content.trim() != "max" {
                stats.memory_limit = content.trim().parse().ok();
            }
        }

        if let Ok(content) = fs::read_to_string(self.cgroup_path.join("cpu.stat")) {
            for line in content.lines() {
                if line.starts_with("usage_usec ") {
                    if let Ok(usage) = line.split_whitespace().nth(1).unwrap_or("0").parse::<u64>()
                    {
                        stats.cpu_usage_usec = usage;
                    }
                }
            }
        }

        if let Ok(content) = fs::read_to_string(self.cgroup_path.join("pids.current")) {
            stats.pids_current = content.trim().parse().unwrap_or(0);
        }

        if let Ok(content) = fs::read_to_string(self.cgroup_path.join("pids.max")) {
            if content.trim() != "max" {
                stats.pids_limit = content.trim().parse().ok();
            }
        }

        Ok(stats)
    }

    pub fn cleanup(&self) -> Result<()> {
        self.cleanup_internal()
    }

    fn cleanup_internal(&self) -> Result<()> {
        if !self.cgroup_path.exists() {
            return Ok(());
        }

        println!("[Cgroups] Cleaning up cgroup: {}", self.container_id);

        if let Ok(procs_content) = fs::read_to_string(self.cgroup_path.join("cgroup.procs")) {
            let parent_procs = self.base_path.join("cgroup.procs");
            for pid_line in procs_content.lines() {
                if !pid_line.trim().is_empty() {
                    let _ = fs::write(&parent_procs, pid_line);
                    println!("[Cgroups] Moved PID {} back to parent cgroup", pid_line);
                }
            }
        }

        fs::remove_dir(&self.cgroup_path).with_context(|| {
            format!(
                "Failed to remove cgroup directory: {}",
                self.cgroup_path.display()
            )
        })?;

        println!("[Cgroups] Successfully cleaned up cgroup");
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct CgroupStats {
    pub memory_usage: u64,
    pub memory_limit: Option<u64>,
    pub cpu_usage_usec: u64,
    pub pids_current: u32,
    pub pids_limit: Option<u32>,
}

impl Drop for CgroupManager {
    fn drop(&mut self) {}
}

pub fn get_user_cgroup_base() -> Result<PathBuf> {
    let cgroup_file =
        fs::read_to_string("/proc/self/cgroup").context("Failed to read /proc/self/cgroup")?;

    for line in cgroup_file.lines() {
        if line.starts_with("0::/") {
            let rel_path = line.trim_start_matches("0::");
            let mut base_path =
                PathBuf::from("/sys/fs/cgroup").join(rel_path.trim_start_matches('/'));

            while base_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "bento-control")
                .unwrap_or(false)
            {
                if let Some(parent) = base_path.parent() {
                    base_path = parent.to_path_buf();
                } else {
                    break;
                }
            }

            if can_write_to_cgroup(&base_path) {
                return Ok(base_path);
            }

            let _uid = std::env::var("USER").unwrap_or_else(|_| "1000".to_string());
            let user_service_path = base_path.parent().unwrap_or(&base_path).join(format!(
                "user@{}.service",
                std::env::var("UID").unwrap_or_else(|_| "1000".to_string())
            ));

            if can_write_to_cgroup(&user_service_path) {
                println!(
                    "[Cgroups] Using delegated user service cgroup: {}",
                    user_service_path.display()
                );
                return Ok(user_service_path);
            }

            let fallback_path = PathBuf::from("/sys/fs/cgroup/user.slice")
                .join(format!("user-{}.slice", get_current_uid()))
                .join(format!("user@{}.service", get_current_uid()));

            if can_write_to_cgroup(&fallback_path) {
                println!(
                    "[Cgroups] Using fallback user service cgroup: {}",
                    fallback_path.display()
                );
                return Ok(fallback_path);
            }

            return Err(anyhow!(
                "No writable cgroup found. Current path: {}\nTried paths: {}, {}\n\
                This usually means cgroups are not delegated to your user.\n\
                Try: sudo systemctl enable --now systemd-oomd",
                base_path.display(),
                user_service_path.display(),
                fallback_path.display()
            ));
        }
    }

    Err(anyhow!(
        "Unified cgroup v2 not found. Ensure cgroup v2 is enabled and delegated.\n\
        Try: sudo systemctl enable --now systemd-oomd"
    ))
}

fn can_write_to_cgroup(path: &PathBuf) -> bool {
    if !path.exists() {
        return false;
    }

    let test_path = path.join("cgroup-test-write-check");
    match fs::create_dir(&test_path) {
        Ok(()) => {
            let _ = fs::remove_dir(&test_path);
            true
        }
        Err(_) => {
            let subtree_control = path.join("cgroup.subtree_control");
            let procs_file = path.join("cgroup.procs");

            if let (Ok(subtree_meta), Ok(procs_meta)) =
                (fs::metadata(&subtree_control), fs::metadata(&procs_file))
            {
                use std::os::unix::fs::MetadataExt;
                let current_uid = get_current_uid().parse::<u32>().unwrap_or(1000);
                return subtree_meta.uid() == current_uid || procs_meta.uid() == current_uid;
            }
            false
        }
    }
}

fn get_current_uid() -> String {
    std::env::var("UID").unwrap_or_else(|_| {
        if let Ok(status) = fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("Uid:") {
                    if let Some(uid) = line.split_whitespace().nth(1) {
                        return uid.to_string();
                    }
                }
            }
        }
        "1000".to_string()
    })
}

pub fn setup_cgroups(
    config: &CgroupsConfig,
    container_id: &str,
    pid: Pid,
    _base: &Path,
) -> Result<PathBuf> {
    let manager = CgroupManager::new(container_id.to_string())?;
    manager.setup(config, pid)?;
    Ok(manager.cgroup_path.clone())
}

pub fn cleanup_cgroups(cgroup_path: &Path) -> Result<()> {
    if let Some(container_id) = cgroup_path.file_name().and_then(|n| n.to_str()) {
        let manager = CgroupManager::new(container_id.to_string())?;
        manager.cleanup()
    } else {
        fs::remove_dir(cgroup_path).context("Failed to remove cgroup dir")
    }
}
