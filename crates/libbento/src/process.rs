// crates/libbento/src/process.rs

use nix::sys::stat::Mode;
use std::io::{Read, Write};
//use std::os::unix::io::FromRawFd;
use crate::fs;
use crate::syscalls::{
    disable_setgroups_for_child, fork_intermediate, map_user_namespace_rootless,
    unshare_remaining_namespaces, unshare_user_namespace,
};
use anyhow::{Result, anyhow, Context};
use nix::unistd::{Pid, ForkResult, fork, getpid, pipe, read, write, mkfifo};
use std::os::unix::io::{AsRawFd, OwnedFd};

use serde::{Serialize, Deserialize};
use std::fs as std_fs;
use std::path::PathBuf;
use nix::sys::wait::waitpid;  // For waitpid function
use nix::sys::signal::{kill, Signal};

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
    let home = std::env::var("HOME")
        .context("HOME environment variable not set")?;
    let state_dir = PathBuf::from(format!("{}/.local/share/bento/state", home));
    std_fs::create_dir_all(&state_dir)
        .context("Failed to create bento state directory")?;
    Ok(state_dir)
}

fn save_container_state(container_id: &str, state: &ContainerState) -> Result<PathBuf> {
    let state_dir = get_state_dir()?;
    let state_file = state_dir.join(format!("{}.json", container_id));
    
    let json_content = serde_json::to_string_pretty(state)
        .context("Failed to serialize container state")?;
    
    std_fs::write(&state_file, json_content)
        .context("Failed to write state file")?;
    
    println!("[State] Container state saved to: {:?}", state_file);
    Ok(state_file)
}

fn load_container_state(container_id: &str) -> Result<ContainerState> {
    let state_dir = get_state_dir()?;
    let state_file = state_dir.join(format!("{}.json", container_id));
    
    if !state_file.exists() {
        return Err(anyhow!("Container '{}' not found", container_id));
    }
    
    let json_content = std_fs::read_to_string(&state_file)
        .context("Failed to read state file")?;
    
    let state: ContainerState = serde_json::from_str(&json_content)
        .context("Failed to parse state file")?;
    
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

pub struct Config {
    pub root_path: String,
    pub args: Vec<String>,
    pub hostname: String,
    pub rootless: bool,
    pub bundle_path: String,
    pub container_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_path: "/tmp/bento-rootfs".to_string(),
            args: vec!["/bin/cat".to_string(), "/proc/self/stat".to_string()],
            hostname: "bento-container".to_string(),
            rootless: true,
            bundle_path: ".".to_string(),
            container_id: "default".to_string(),
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

	let start_pipe = 
            pipe().map_err(|e| anyhow!("Failed to create start pipe: {}", e))?;
        println!("[Sync] All pipes created (sync + start)");
        /*println!("[Sync] Pipes created:");
        println!(
            " Orchestrator->Bridge: read_fd={}, write_fd={}",
            orchestrator_to_bridge.0.as_raw_fd(),
            orchestrator_to_bridge.1.as_raw_fd()
        );
        println!(
            " Bridge->Orchestrator: read_fd={}, write_fd={}",
            bridge_to_orchestrator.0.as_raw_fd(),
            bridge_to_orchestrator.1.as_raw_fd()
        );*/

        let orchestrator_pipes = OrchestratorPipes {
            read_fd: bridge_to_orchestrator.0,
            write_fd: orchestrator_to_bridge.1,
	    start_write_fd: start_pipe.1, // Orchestrator keeps write end for state.json
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
    start_write_fd: OwnedFd, // For writing to unblock init
}


struct BridgePipes {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
    start_read_fd: OwnedFd, // Pass through to init
}

struct InitPipes {
    start_read_fd: OwnedFd, // For blocking until start
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
    let orchestrator_logic = |bridge_pid| orchestrator_handler(bridge_pid, orchestrator_pipes, config);
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
    let home = std::env::var("HOME")
        .context("HOME environment variable not set")?;
    let container_rootfs = format!("{}/.local/share/bento/{}/rootfs", home, config.container_id);
    let start_pipe_path = format!("{}/tmp/bento-start-{}", container_rootfs, config.container_id);
    
    // Ensure tmp directory exists in container rootfs
    std_fs::create_dir_all(format!("{}/tmp", container_rootfs))?;
    
    // Create FIFO in container's filesystem
    if let Err(e) = mkfifo(start_pipe_path.as_str(), Mode::S_IRUSR | Mode::S_IWUSR) {
        return Err(anyhow!("Failed to create start pipe: {}", e));
    }
    
    println!("[Orchestrator] Created start pipe in container rootfs: {}", start_pipe_path);

    // Create and save state.json
    let mut container_state = ContainerState::new(
        config.container_id.clone(),
        final_container_pid,
        config.bundle_path.clone()
    );

    // Store the container-relative path (what init will see after pivot_root)
    container_state.start_pipe_path = Some(format!("/tmp/bento-start-{}", config.container_id));
    
    save_container_state(&config.container_id, &container_state)
        .context("Failed to save container state")?;


    // NEW: Wait for bridge to exit (proper daemonless cleanup)
    println!("[Orchestrator] Waiting for bridge process to exit...");
    match waitpid(bridge_pid, None) {
        Ok(status) => println!("[Orchestrator] Bridge exited with status: {:?}", status),
        Err(e) => println!("[Orchestrator] Warning: Failed to wait for bridge: {}", e),
    }

    println!("[Orchestrator] Container '{}' created successfully (status: created)", 
             config.container_id);
    println!("[Orchestrator] Use 'bento start {}' to run the container", 
             config.container_id);

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
    //create_init_with_pid_communication(config, &pipes)
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
        Ok(ForkResult::Parent { child: init_process }) => {
            // Parent (bridge) - close start_pipe and send PID
            drop(&pipes.start_read_fd); // Close in bridge
            
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


/*
fn create_init_with_pid_communication(config: &Config, pipes: &BridgePipes) -> isize {
    println!("[Bridge] Creating init process...");

    match unsafe { fork() } {
        Ok(ForkResult::Parent {
            child: init_process,
        }) => {
            println!("[Bridge] Created init PID: {init_process}");

            // Send init process PID to orchestrator
            let pid_bytes = init_process.as_raw().to_le_bytes();
            if let Err(e) = write(&pipes.write_fd, &pid_bytes) {
                eprintln!("[Bridge] Failed to send init PID: {e}");
                return 1;
            }
            println!("[Bridge] Sent init PID to orchestrator");
            println!("[Bridge] Mission complete - exiting");
            0
        }
        Ok(ForkResult::Child) => init_handler(config),
        Err(e) => {
            eprintln!("[Bridge] Failed to fork init process: {e}");
            1
        }
    }
}
*/


// ============================================================================
// INIT PROCESS LOGIC (Container Init - PID 1)
// ============================================================================
/*
fn init_handler(config: &Config) -> isize {
    println!("[Init] I am PID 1 in container: {}", getpid());
    
    if let Err(e) = debug_namespace_info() {
        eprintln!("[Init] Failed to debug namespace info: {}", e);
    }

    // Phase 1: Filesystem preparation 
    match fs::prepare_rootfs(&config.container_id) {
        Ok(_) => println!("[Init] Filesystem prepared successfully"),
        Err(e) => {
            eprintln!("[Init] Filesystem preparation failed: {}", e);
            return 1;
        }
    }
  

    set_container_hostname("my-container");    

    // Phase 3: Basic setup complete - enter PAUSE state
    println!("[Init] Container setup complete - entering PAUSE state");
    println!("[Init] Waiting for 'bento start' command...");
    
    // Block on start_pipe read - this is the key pause mechanism
    let mut start_buffer = [0u8; 1];
    match nix::unistd::read(start_pipe_fd, &mut start_buffer) {
        Ok(_) => {
            println!("[Init] Received start signal - proceeding to exec");
        }
        Err(e) => {
            eprintln!("[Init] Failed to read from start pipe: {}", e);
            return 1;
        }
    }    
*/

fn init_handler_with_pause(config: &Config, _start_pipe_fd: i32) -> isize {
    println!("[Init] I am PID 1 in container: {}", getpid());
    
    if let Err(e) = debug_namespace_info() {
        eprintln!("[Init] Failed to debug namespace info: {}", e);
    }

    // Phase 1: Filesystem preparation
    match fs::prepare_rootfs(&config.container_id) {
        Ok(_) => println!("[Init] Filesystem prepared successfully"),
        Err(e) => {
            eprintln!("[Init] Filesystem preparation failed: {}", e);
            return 1;
        }
    }

    // Phase 2: Set hostname
    if let Err(e) = set_container_hostname(&config.hostname) {
        eprintln!("[Init] Failed to set hostname: {}", e);
        return 1;
    }

    // Phase 3: Enter PAUSE state - BLOCK HERE until bento start
    let start_pipe_path = format!("/tmp/bento-start-{}", config.container_id);
    println!("[Init] Container setup complete - entering PAUSE state");
    println!("[Init] Waiting for signal at: {}", start_pipe_path);
// Open named pipe for reading (this blocks until writer opens)
    use std::fs::OpenOptions;
    use std::io::Read;
    
    match std_fs::OpenOptions::new().read(true).open(&start_pipe_path) {
        Ok(mut pipe) => {
            let mut buffer = [0u8; 1];
            match pipe.read(&mut buffer) {
                Ok(_) => {
                    println!("[Init] Received start signal - proceeding to exec");
                }
                Err(e) => {
                    eprintln!("[Init] Failed to read from start pipe: {}", e);
                    return 1;
                }
            }
        }
        Err(e) => {
            eprintln!("[Init] Failed to open start pipe: {}", e);
            return 1;
        }
    }

    // Phase 4: Execute user command
    println!("[Init] Executing user command: {:?}", config.args);
    exec_user_command(config)
    // Temporary: Keep current isolation test
    //println!("[Init] Testing namespace isolation...");
    //execute_isolation_test()
    
    //todo!("Setup environment variables");
    //todo!("Apply security contexts");

    // TODO: Start pipe mechanism
    //todo!("Create start_pipe for pause/resume");
    //todo!("Block on start_pipe until 'bento start'");

    // TODO: Execute user command
    //todo!("Execute config.args instead of test command");

}


fn set_container_hostname(hostname: &str) -> Result<()> {
    println!("[Init] Setting container hostname to: {}", hostname);
    
    match nix::unistd::sethostname(hostname) {
        Ok(_) => {
            println!("[Init] Hostname successfully set to: {}", hostname);
            Ok(())
        }
        Err(e) => {
            println!("[Init] Warning: Failed to set hostname: {}", e);
            // Don't fail the container for hostname issues
            Ok(())
        }
    }
}

fn debug_namespace_info() -> Result<()> {
    use std::fs;
    
    println!("[Debug] Current process namespace information:");
    
    // Check PID namespace
    let pid_ns = fs::read_link("/proc/self/ns/pid")
        .context("Failed to read PID namespace")?;
    println!("[Debug] PID namespace: {:?}", pid_ns);
    
    // Check mount namespace  
    let mnt_ns = fs::read_link("/proc/self/ns/mnt")
        .context("Failed to read mount namespace")?;
    println!("[Debug] Mount namespace: {:?}", mnt_ns);
    
    // Check user namespace
    let user_ns = fs::read_link("/proc/self/ns/user")
        .context("Failed to read user namespace")?;
    println!("[Debug] User namespace: {:?}", user_ns);
    
    // Check UTS namespace (hostname)
    let uts_ns = fs::read_link("/proc/self/ns/uts")
        .context("Failed to read UTS namespace")?;
    println!("[Debug] UTS namespace: {:?}", uts_ns);
    
    // Check current PID as seen by process
    println!("[Debug] Current PID (should be 1 in container): {}", nix::unistd::getpid());
    
    // Check parent PID
    println!("[Debug] Parent PID: {}", nix::unistd::getppid());
    
    Ok(())
}

/*
fn execute_isolation_test() -> isize {
    use nix::unistd::execvp;
    use std::ffi::CString;

    let args = vec![CString::new("/bin/id").unwrap()];

    match execvp(&args[0], &args) {
        Ok(_) => {
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("[Init] execvp failed: {e}");
            std::process::exit(1);
        }
    }
}
*/

fn execute_isolation_test() -> isize {
    println!("[Init] Testing container isolation without external binaries");
    
    // Test 1: Verify we're PID 1
    let pid = nix::unistd::getpid();
    println!("[Test] Current PID: {}", pid);
    assert_eq!(pid.as_raw(), 1, "Expected to be PID 1 in container");
    
    // Test 2: Check filesystem isolation
    match std::fs::read_dir("/") {
        Ok(entries) => {
            println!("[Test] Root directory contents:");
            for entry in entries {
                if let Ok(entry) = entry {
                    println!("[Test]   - {}", entry.file_name().to_string_lossy());
                }
            }
        }
        Err(e) => println!("[Test] Failed to read root directory: {}", e),
    }
    
    // Test 3: Check mount namespaces
    match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(mounts) => {
            println!("[Test] Container has {} mount entries", mounts.lines().count());
            // Show first few mounts
            for (i, line) in mounts.lines().take(5).enumerate() {
                println!("[Test] Mount {}: {}", i, line);
            }
        }
        Err(e) => println!("[Test] Failed to read mountinfo: {}", e),
    }
    
    // Test 4: Check user namespace mapping
    match std::fs::read_to_string("/proc/self/uid_map") {
        Ok(uid_map) => println!("[Test] UID mapping: {}", uid_map.trim()),
        Err(e) => println!("[Test] Failed to read uid_map: {}", e),
    }
    
    match std::fs::read_to_string("/proc/self/gid_map") {
        Ok(gid_map) => println!("[Test] GID mapping: {}", gid_map.trim()),
        Err(e) => println!("[Test] Failed to read gid_map: {}", e),
    }
    
    // Test 5: Check hostname isolation  
    match nix::unistd::gethostname() {
        Ok(hostname) => println!("[Test] Container hostname: {}", hostname.to_string_lossy()),
        Err(e) => println!("[Test] Failed to get hostname: {}", e),
    }
    
    println!("[Test] Container isolation test completed successfully!");
    println!("[Test] Container is ready for actual workloads");
    
    // For now, just exit successfully
    // In a real container, this would be where we execute the user's command
    0
}

fn exec_user_command(config: &Config) -> isize {
    use nix::unistd::execvp;
    use std::ffi::CString;
    
    // Convert args to CString
    let c_args: Result<Vec<CString>, _> = config.args
        .iter()
        .map(|arg| CString::new(arg.as_str()))
        .collect();
        
    let c_args = match c_args {
        Ok(args) => args,
        Err(e) => {
            eprintln!("[Init] Failed to convert args to CString: {}", e);
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
            eprintln!("[Init] execvp failed: {}", e);
            1
        }
    }
}
pub fn start_container(container_id: &str) -> Result<()> {
    let state = load_container_state(container_id)?;
    
    if state.status != "created" {
        return Err(anyhow!("Container '{}' is not in 'created' state (current: {})", 
                          container_id, state.status));
    }
    
    println!("[Start] Loading container '{}' (PID: {})", container_id, state.pid);
    
    // Check if process is still alive
    if let Err(_) = kill(nix::unistd::Pid::from_raw(state.pid), Signal::SIGCONT) {
        return Err(anyhow!("Container process {} is no longer running", state.pid));
    }
    
    // Convert container path to host path for writing
    if let Some(ref container_pipe_path) = state.start_pipe_path {
        let home = std::env::var("HOME")?;
        let host_pipe_path = format!("{}/.local/share/bento/{}/rootfs{}", 
                                   home, container_id, container_pipe_path);
        
        match std_fs::OpenOptions::new().write(true).open(&host_pipe_path) {
            Ok(mut pipe) => {
                use std::io::Write;
                match pipe.write(b"start") {
                    Ok(_) => {
                        println!("[Start] Successfully signaled init process via named pipe");
                        // Clean up the named pipe from host perspective
                        let _ = std_fs::remove_file(&host_pipe_path);
                    }
                    Err(e) => return Err(anyhow!("Failed to write to start pipe: {}", e)),
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
    
    println!("[Start] Container '{}' is now running", container_id);
    Ok(())
}

