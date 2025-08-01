// crates/libbento/src/process.rs

use crate::fs;
use crate::syscalls::{
    disable_setgroups_for_child, fork_intermediate, map_user_namespace_rootless,
    unshare_remaining_namespaces, unshare_user_namespace,
};
use anyhow::{Context, Result, anyhow};
use nix::sys::signal::{Signal, kill};
use nix::sys::stat::Mode;
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, Pid, fork, getpid, mkfifo, pipe, read, write};
use serde::{Deserialize, Serialize};
use std::fs as std_fs;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::path::PathBuf;

// NEW: Add the RootfsPopulationMethod enum
#[derive(Debug, Clone)]
pub enum RootfsPopulationMethod {
    Manual,
    BusyBox,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ContainerState {
    pub id: String,
    pub pid: i32,
    pub status: String,
    pub bundle_path: String,
    pub created_at: String,
    pub start_pipe_path: Option<String>, // Store for bento start to reopen
}

impl ContainerState {
    fn new(id: String, pid: i32, bundle_path: String) -> Self {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();

        Self {
            id,
            pid,
            status: "created".to_string(),
            bundle_path,
            created_at,
            start_pipe_path: None, // Will be set when created
        }
    }
}

fn get_state_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let state_dir = PathBuf::from(format!("{home}/.local/share/bento/state"));
    std_fs::create_dir_all(&state_dir).context("Failed to create bento state directory")?;
    Ok(state_dir)
}

fn save_container_state(container_id: &str, state: &ContainerState) -> Result<PathBuf> {
    let state_dir = get_state_dir()?;
    let state_file = state_dir.join(format!("{container_id}.json"));
    let json_content =
        serde_json::to_string_pretty(state).context("Failed to serialize container state")?;
    std_fs::write(&state_file, json_content).context("Failed to write state file")?;
    println!("[State] Container state saved to: {state_file:?}");
    Ok(state_file)
}

fn load_container_state(container_id: &str) -> Result<ContainerState> {
    let state_dir = get_state_dir()?;
    let state_file = state_dir.join(format!("{container_id}.json"));

    if !state_file.exists() {
        return Err(anyhow!("Container '{}' not found", container_id));
    }

    let json_content = std_fs::read_to_string(&state_file).context("Failed to read state file")?;
    let state: ContainerState =
        serde_json::from_str(&json_content).context("Failed to parse state file")?;
    Ok(state)
}

// ============================================================================
// SYNC SIGNAL DEFINITIONS
// ============================================================================

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq)]
enum SyncSignal {
    Ready = b'R',
    Mapped = b'M',
}

impl SyncSignal {
    fn as_byte(&self) -> u8 {
        *self as u8
    }

    fn from_byte(byte: u8) -> Result<Self> {
        match byte {
            b'R' => Ok(SyncSignal::Ready),
            b'M' => Ok(SyncSignal::Mapped),
            _ => Err(anyhow!("Invalid sync signal byte: {}", byte as char)),
        }
    }

    fn as_char(&self) -> char {
        self.as_byte() as char
    }
}

// UPDATED: Add population_method field to Config
#[derive(Debug, Clone)]
pub struct Config {
    pub root_path: String,
    pub args: Vec<String>,
    pub hostname: String,
    pub rootless: bool,
    pub bundle_path: String,
    pub container_id: String,
    pub population_method: RootfsPopulationMethod, // NEW: Add this field
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_path: "/tmp/bento-rootfs".to_string(),
            args: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "/bin/ls /bin; /bin/echo 'PATH test'; echo $PATH".to_string(),
            ],
            hostname: "bento-container".to_string(),
            rootless: true,
            bundle_path: ".".to_string(),
            container_id: "default".to_string(),
            population_method: RootfsPopulationMethod::BusyBox, // NEW: Default to reliable method
        }
    }
}

// ============================================================================
// PIPE MANAGEMENT HELPERS (Internal Refactoring)
// ============================================================================

struct ContainerPipes {}

impl ContainerPipes {
    fn create() -> Result<(OrchestratorPipes, BridgePipes)> {
        let orchestrator_to_bridge =
            pipe().map_err(|e| anyhow!("Failed to create orchestrator->bridge pipe: {}", e))?;
        let bridge_to_orchestrator =
            pipe().map_err(|e| anyhow!("Failed to create bridge->orchestrator pipe: {}", e))?;
        let start_pipe = pipe().map_err(|e| anyhow!("Failed to create start pipe: {}", e))?;

        println!("[Sync] All pipes created (sync + start)");

        let orchestrator_pipes = OrchestratorPipes {
            read_fd: bridge_to_orchestrator.0,
            write_fd: orchestrator_to_bridge.1,
        };

        let bridge_pipes = BridgePipes {
            read_fd: orchestrator_to_bridge.0,
            write_fd: bridge_to_orchestrator.1,
            start_read_fd: start_pipe.0, // Pass read end through bridge to init
        };

        Ok((orchestrator_pipes, bridge_pipes))
    }
}

struct OrchestratorPipes {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
    //start_write_fd: OwnedFd, // For writing to unblock init
}

struct BridgePipes {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
    start_read_fd: OwnedFd, // Pass through to init
}

// Common pipe operations (reduces repetition)
fn pipe_signal(fd: &OwnedFd, signal: SyncSignal, context: &str) -> Result<()> {
    write(fd, &[signal.as_byte()])
        .map_err(|e| anyhow!("Failed to send {} signal: {}", context, e))?;
    println!("[{}] Sent '{}' signal", context, signal.as_char());
    Ok(())
}

fn pipe_wait(fd: &OwnedFd, expected: SyncSignal, context: &str) -> Result<()> {
    let mut buf = [0u8; 1];
    read(fd, &mut buf).map_err(|e| anyhow!("Failed to receive {} signal: {}", context, e))?;
    let received = SyncSignal::from_byte(buf[0])?;
    if received != expected {
        return Err(anyhow!(
            "Expected '{}', got '{}' in {}",
            expected.as_char(),
            received.as_char(),
            context
        ));
    }

    println!("[{}] Received '{}' signal", context, received.as_char());
    Ok(())
}

// ============================================================================
// MAIN CONTAINER CREATION
// ============================================================================

pub fn create_container(config: &Config) -> Result<()> {
    let (orchestrator_pipes, bridge_pipes) = ContainerPipes::create()?;
    println!("Bento.rs Rootless Container Runtime");
    println!(
        "Orchestrator PID: {} (running as unprivileged user)",
        getpid()
    );

    // Clean closures calling purpose-driven functions
    let orchestrator_logic =
        |bridge_pid| orchestrator_handler(bridge_pid, orchestrator_pipes, config);
    let bridge_logic = || bridge_handler(config, bridge_pipes);

    fork_intermediate(orchestrator_logic, bridge_logic)?;
    Ok(())
}

// ============================================================================
// ORCHESTRATOR PROCESS LOGIC (Container Creation Coordinator)
// ============================================================================

fn orchestrator_handler(bridge_pid: Pid, pipes: OrchestratorPipes, config: &Config) -> Result<()> {
    println!("[Orchestrator] Bridge spawned with PID: {bridge_pid}");

    // Wait for bridge namespace ready signal
    println!("[Orchestrator] Waiting for bridge namespace ready signal...");
    pipe_wait(&pipes.read_fd, SyncSignal::Ready, "Orchestrator")?;

    // Perform UID/GID mapping
    map_user_namespace_rootless(bridge_pid)?;
    println!("[Orchestrator] UID/GID mapping completed successfully");

    // Signal mapping complete
    pipe_signal(&pipes.write_fd, SyncSignal::Mapped, "Orchestrator")?;

    // Receive init process PID
    println!("[Orchestrator] Waiting for final container PID...");
    let mut pid_buf = [0u8; 4];
    read(&pipes.read_fd, &mut pid_buf).map_err(|e| anyhow!("Failed to receive init PID: {}", e))?;
    let final_container_pid = i32::from_le_bytes(pid_buf);
    println!("[Orchestrator] Final container PID: {final_container_pid}");

    // State management
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let container_rootfs = format!("{}/.local/share/bento/{}/rootfs", home, config.container_id);
    let start_pipe_path = format!(
        "{}/tmp/bento-start-{}",
        container_rootfs, config.container_id
    );

    // Ensure tmp directory exists in container rootfs
    std_fs::create_dir_all(format!("{container_rootfs}/tmp"))?;

    // Create FIFO in container's filesystem
    if let Err(e) = mkfifo(start_pipe_path.as_str(), Mode::S_IRUSR | Mode::S_IWUSR) {
        return Err(anyhow!("Failed to create start pipe: {}", e));
    }

    println!(
        "[Orchestrator] Created start pipe in container rootfs: {start_pipe_path}"
    );

    // Create and save state.json
    let mut container_state = ContainerState::new(
        config.container_id.clone(),
        final_container_pid,
        config.bundle_path.clone(),
    );

    // Store the container-relative path (what init will see after pivot_root)
    container_state.start_pipe_path = Some(format!("/tmp/bento-start-{}", config.container_id));

    save_container_state(&config.container_id, &container_state)
        .context("Failed to save container state")?;

    // NEW: Wait for bridge to exit (proper daemonless cleanup)
    println!("[Orchestrator] Waiting for bridge process to exit...");
    match waitpid(bridge_pid, None) {
        Ok(status) => println!("[Orchestrator] Bridge exited with status: {status:?}"),
        Err(e) => println!("[Orchestrator] Warning: Failed to wait for bridge: {e}"),
    }

    println!(
        "[Orchestrator] Container '{}' created successfully (status: created)",
        config.container_id
    );
    println!(
        "[Orchestrator] Use 'bento start {}' to run the container",
        config.container_id
    );
    Ok(())
}

// ============================================================================
// BRIDGE PROCESS LOGIC (Namespace Builder)
// ============================================================================

fn bridge_handler(config: &Config, pipes: BridgePipes) -> isize {
    println!(
        "[Bridge] Container initialization started, PID: {}",
        getpid()
    );

    // Phase 1: User namespace setup
    if let Err(e) = setup_user_namespace(&pipes) {
        eprintln!("[Bridge] User namespace setup failed: {e}");
        return 1;
    }

    // Phase 2: Wait for UID/GID mapping
    if let Err(e) = wait_for_mapping(&pipes) {
        eprintln!("[Bridge] Mapping synchronization failed: {e}");
        return 1;
    }

    // Phase 3: Create remaining namespaces
    if let Err(e) = create_remaining_namespaces() {
        eprintln!("[Bridge] Remaining namespaces creation failed: {e}");
        return 1;
    }

    // Phase 4: Create init process and communicate PID
    create_init_with_start_pipe(config, &pipes)
}

// Helper functions for bridge phases
fn setup_user_namespace(pipes: &BridgePipes) -> Result<()> {
    unshare_user_namespace()?;
    disable_setgroups_for_child()?;
    pipe_signal(&pipes.write_fd, SyncSignal::Ready, "Bridge")?;
    Ok(())
}

fn wait_for_mapping(pipes: &BridgePipes) -> Result<()> {
    println!("[Bridge] Waiting for mapping complete signal...");
    pipe_wait(&pipes.read_fd, SyncSignal::Mapped, "Bridge")?;
    Ok(())
}

fn create_remaining_namespaces() -> Result<()> {
    unshare_remaining_namespaces()
        .map_err(|e| anyhow!("Failed to create remaining namespaces: {}", e))
}

fn create_init_with_start_pipe(config: &Config, pipes: &BridgePipes) -> isize {
    println!("[Bridge] Creating init process...");

    // Get the raw FD before fork
    let start_pipe_fd = pipes.start_read_fd.as_raw_fd();

    match unsafe { fork() } {
        Ok(ForkResult::Parent {
            child: init_process,
        }) => {
            // Parent (bridge) - properly drop the read end
            let _ = &pipes.start_read_fd; // Drop reference to allow cleanup

            let pid_bytes = init_process.as_raw().to_le_bytes();
            if let Err(e) = write(&pipes.write_fd, &pid_bytes) {
                eprintln!("[Bridge] Failed to send init PID: {e}");
                return 1;
            }

            println!("[Bridge] Mission complete - exiting");
            0
        }
        Ok(ForkResult::Child) => {
            // Child (init) - keep start_pipe_fd for blocking
            init_handler_with_pause(config, start_pipe_fd)
        }
        Err(e) => {
            eprintln!("[Bridge] Failed to fork init process: {e}");
            1
        }
    }
}

// ============================================================================
// INIT PROCESS LOGIC (Container Init - PID 1)
// ============================================================================

fn init_handler_with_pause(config: &Config, _start_pipe_fd: i32) -> isize {
    println!("[Init] I am PID 1 in container: {}", getpid());

    if let Err(e) = debug_namespace_info() {
        eprintln!("[Init] Failed to debug namespace info: {e}");
    }

    // Phase 1: Filesystem preparation - FIXED: Pass config parameter
    match fs::prepare_rootfs(&config.container_id, config) {
        Ok(_) => println!("[Init] Filesystem prepared successfully"),
        Err(e) => {
            eprintln!("[Init] Filesystem preparation failed: {e}");
            return 1;
        }
    }

    // Phase 2: Set hostname
    if let Err(e) = set_container_hostname(&config.hostname) {
        eprintln!("[Init] Failed to set hostname: {e}");
        return 1;
    }

    // Phase 3: Environment setup
    if let Err(e) = setup_container_environment() {
        eprintln!("[Init] Failed to setup environment: {e}");
        return 1;
    }

    // Phase 4: Enter PAUSE state - BLOCK HERE until bento start
    let start_pipe_path = format!("/tmp/bento-start-{}", config.container_id);
    println!("[Init] Container setup complete - entering PAUSE state");
    println!("[Init] Waiting for signal at: {start_pipe_path}");

    // Open named pipe for reading (this blocks until writer opens)
    match std_fs::OpenOptions::new().read(true).open(&start_pipe_path) {
        Ok(_pipe) => {
            let _buffer = [0u8; 1];
            
            match read_start_signal(&start_pipe_path) {
		    Ok(()) => {
        		println!("[Init] Start signal processing complete");
			}
    		    Err(e) => {
        		eprintln!("[Init] Failed to process start signal: {e}");
        		return 1;
    		    }
	    }
	}
        Err(e) => {
            eprintln!("[Init] Failed to open start pipe: {e}");
            return 1;
        }
    }

    // Phase 5: Execute user command
    println!("[Init] Executing user command: {:?}", config.args);
    exec_user_command(config)
}

// NEW: Environment setup function
fn setup_container_environment() -> Result<()> {
    unsafe {
        std::env::set_var("PATH", "/bin:/usr/bin");
        std::env::set_var("HOME", "/");
        std::env::set_var("USER", "root");
        std::env::set_var("SHELL", "/bin/sh");
        std::env::set_var("TERM", "xterm");
    }
    println!("[Container] Environment configured");
    Ok(())
}

fn set_container_hostname(hostname: &str) -> Result<()> {
    println!("[Init] Setting container hostname to: {hostname}");
    match nix::unistd::sethostname(hostname) {
        Ok(_) => {
            println!("[Init] Hostname successfully set to: {hostname}");
            Ok(())
        }
        Err(e) => {
            println!("[Init] Warning: Failed to set hostname: {e}");
            // Don't fail the container for hostname issues
            Ok(())
        }
    }
}

fn debug_namespace_info() -> Result<()> {
    use std::fs;

    println!("[Debug] Current process namespace information:");

    // Check PID namespace
    let pid_ns = fs::read_link("/proc/self/ns/pid").context("Failed to read PID namespace")?;
    println!("[Debug] PID namespace: {pid_ns:?}");

    // Check mount namespace
    let mnt_ns = fs::read_link("/proc/self/ns/mnt").context("Failed to read mount namespace")?;
    println!("[Debug] Mount namespace: {mnt_ns:?}");

    // Check user namespace
    let user_ns = fs::read_link("/proc/self/ns/user").context("Failed to read user namespace")?;
    println!("[Debug] User namespace: {user_ns:?}");

    // Check UTS namespace (hostname)
    let uts_ns = fs::read_link("/proc/self/ns/uts").context("Failed to read UTS namespace")?;
    println!("[Debug] UTS namespace: {uts_ns:?}");

    // Check current PID as seen by process
    println!(
        "[Debug] Current PID (should be 1 in container): {}",
        nix::unistd::getpid()
    );

    // Check parent PID
    println!("[Debug] Parent PID: {}", nix::unistd::getppid());

    Ok(())
}

fn exec_user_command(config: &Config) -> isize {
    use nix::unistd::execvp;
    use std::ffi::CString;

    // Convert args to CString
    let c_args: Result<Vec<CString>, _> = config
        .args
        .iter()
        .map(|arg| CString::new(arg.as_str()))
        .collect();

    let c_args = match c_args {
        Ok(args) => args,
        Err(e) => {
            eprintln!("[Init] Failed to convert args to CString: {e}");
            return 1;
        }
    };

    if c_args.is_empty() {
        eprintln!("[Init] No command specified");
        return 1;
    }

    // execvp replaces the current process
    match execvp(&c_args[0], &c_args) {
        Ok(_) => {
            // This should never be reached
            unreachable!("execvp returned successfully");
        }
        Err(e) => {
            eprintln!("[Init] execvp failed: {e}");
            1
        }
    }
}

pub fn start_container(container_id: &str) -> Result<()> {
    let state = load_container_state(container_id)?;
    if state.status != "created" {
        return Err(anyhow!(
            "Container '{}' is not in 'created' state (current: {})",
            container_id,
            state.status
        ));
    }

    println!(
        "[Start] Loading container '{}' (PID: {})",
        container_id, state.pid
    );

    // Check if process is still alive
    if kill(nix::unistd::Pid::from_raw(state.pid), Signal::SIGCONT).is_err() {
        return Err(anyhow!(
            "Container process {} is no longer running",
            state.pid
        ));
    }

    // Convert container path to host path for writing
    if let Some(ref container_pipe_path) = state.start_pipe_path {
        let home = std::env::var("HOME")?;
        let host_pipe_path = format!(
            "{home}/.local/share/bento/{container_id}/rootfs{container_pipe_path}"
        );

        match std_fs::OpenOptions::new().write(true).open(&host_pipe_path) {
            Ok(_pipe) => {
                
		match send_start_signal(&host_pipe_path) {
    		    Ok(()) => {
        		println!("[Start] Container start signal sent successfully");
                    }
    		    Err(e) => {
        		return Err(anyhow!("Failed to send start signal: {}", e));
                    }
                }

            }
            Err(e) => return Err(anyhow!("Failed to open start pipe: {}", e)),
        }
    } else {
        return Err(anyhow!("No start pipe path found in container state"));
    }

    // Update state to running
    let mut updated_state = state;
    updated_state.status = "running".to_string();
    save_container_state(container_id, &updated_state)?;

    println!("[Start] Container '{container_id}' is now running");
    Ok(())
}

fn read_start_signal(pipe_path: &str) -> Result<()> {
    use std::io::Read;
    
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .open(pipe_path)?;
    
    let mut buffer = [0u8; 5]; // Expect exactly "start" (5 bytes)
    
    // Use read_exact for atomic, complete reads
    pipe.read_exact(&mut buffer)
        .context("Failed to read complete start signal")?;
    
    // Verify signal content
    if &buffer != b"start" {
        return Err(anyhow!("Invalid start signal received: {:?}", 
                          String::from_utf8_lossy(&buffer)));
    }
    
    println!("[Init] Received complete start signal - proceeding to exec");
    Ok(())
}

// FIXED: Robust write with amount verification
fn send_start_signal(pipe_path: &str) -> Result<()> {
    use std::io::Write;
    
    let mut pipe = std::fs::OpenOptions::new()
        .write(true)
        .open(pipe_path)?;
    
    // Use write_all for atomic, complete writes
    pipe.write_all(b"start")
        .context("Failed to write complete start signal")?;
    
    // Ensure data reaches the pipe
    pipe.flush()
        .context("Failed to flush start signal to pipe")?;
    
    println!("[Start] Successfully sent complete start signal");
    Ok(())
}
