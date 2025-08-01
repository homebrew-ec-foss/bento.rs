// crates/libbento/src/process.rs

use crate::fs;
use crate::syscalls::{
    disable_setgroups_for_child, fork_intermediate, map_user_namespace_rootless,
    unshare_remaining_namespaces, unshare_user_namespace,
};
use anyhow::{Context, Result, anyhow};
use nix::unistd::Pid;
use nix::unistd::{ForkResult, fork, getpid, pipe, read, write};
use std::os::unix::io::{AsRawFd, OwnedFd};

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

        println!("[Sync] Pipes created:");
        println!(
            " Orchestrator->Bridge: read_fd={}, write_fd={}",
            orchestrator_to_bridge.0.as_raw_fd(),
            orchestrator_to_bridge.1.as_raw_fd()
        );
        println!(
            " Bridge->Orchestrator: read_fd={}, write_fd={}",
            bridge_to_orchestrator.0.as_raw_fd(),
            bridge_to_orchestrator.1.as_raw_fd()
        );

        let orchestrator_pipes = OrchestratorPipes {
            read_fd: bridge_to_orchestrator.0,
            write_fd: orchestrator_to_bridge.1,
        };

        let bridge_pipes = BridgePipes {
            read_fd: orchestrator_to_bridge.0,
            write_fd: bridge_to_orchestrator.1,
        };

        Ok((orchestrator_pipes, bridge_pipes))
    }
}

struct OrchestratorPipes {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
}

struct BridgePipes {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
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
    let orchestrator_logic = |bridge_pid| orchestrator_handler(bridge_pid, orchestrator_pipes);
    let bridge_logic = || bridge_handler(config, bridge_pipes);

    fork_intermediate(orchestrator_logic, bridge_logic)?;
    Ok(())
}

// ============================================================================
// ORCHESTRATOR PROCESS LOGIC (Container Creation Coordinator)
// ============================================================================

fn orchestrator_handler(bridge_pid: Pid, pipes: OrchestratorPipes) -> Result<()> {
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

    // TODO: State management
    //todo!("Create container state directory and save state.json with PID");
    // Commented cuz it's panicking
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
    create_init_with_pid_communication(config, &pipes)
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

// ============================================================================
// INIT PROCESS LOGIC (Container Init - PID 1)
// ============================================================================

fn init_handler(config: &Config) -> isize {
    println!("[Init] I am PID 1 in container: {}", getpid());

    if let Err(e) = debug_namespace_info() {
        eprintln!("[Init] Failed to debug namespace info: {e}");
    }

    // Phase 1: Filesystem preparation
    match fs::prepare_rootfs(&config.container_id) {
        Ok(_) => println!("[Init] Filesystem prepared successfully"),
        Err(e) => {
            eprintln!("[Init] Filesystem preparation failed: {e}");
            return 1;
        }
    }

    //todo!("Set hostname from config");
    let _ = set_container_hostname("my-container");

    // Temporary: Keep current isolation test
    println!("[Init] Testing namespace isolation...");
    execute_isolation_test()

    //TODO!("Setup environment variables");
    //todo!("Apply security contexts");

    // TODO: Start pipe mechanism
    //todo!("Create start_pipe for pause/resume");
    //todo!("Block on start_pipe until 'bento start'");

    // TODO: Execute user command
    //todo!("Execute config.args instead of test command");
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

fn execute_isolation_test() -> isize {
    // Test 1: Verify we're PID 1
    let pid = nix::unistd::getpid();
    println!("[Test] Current PID: {pid}");
    assert_eq!(pid.as_raw(), 1, "Expected to be PID 1 in container");

    // Test 2: Check filesystem isolation
    match std::fs::read_dir("/") {
        Ok(entries) => {
            println!("[Test] Root directory contents:");

            for entry in entries.filter_map(Result::ok) {
                println!("[Test]   - {}", entry.file_name().to_string_lossy());
            }
        }
        Err(e) => println!("[Test] Failed to read root directory: {e}"),
    }

    // Test 3: Check mount namespaces
    match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(mounts) => {
            println!(
                "[Test] Container has {} mount entries",
                mounts.lines().count()
            );
            // Show first few mounts
            for (i, line) in mounts.lines().take(5).enumerate() {
                println!("[Test] Mount {i}: {line}");
            }
        }
        Err(e) => println!("[Test] Failed to read mountinfo: {e}"),
    }

    // Test 4: Check user namespace mapping
    match std::fs::read_to_string("/proc/self/uid_map") {
        Ok(uid_map) => println!("[Test] UID mapping: {}", uid_map.trim()),
        Err(e) => println!("[Test] Failed to read uid_map: {e}"),
    }

    match std::fs::read_to_string("/proc/self/gid_map") {
        Ok(gid_map) => println!("[Test] GID mapping: {}", gid_map.trim()),
        Err(e) => println!("[Test] Failed to read gid_map: {e}"),
    }

    // Test 5: Check hostname isolation
    match nix::unistd::gethostname() {
        Ok(hostname) => println!("[Test] Container hostname: {}", hostname.to_string_lossy()),
        Err(e) => println!("[Test] Failed to get hostname: {e}"),
    }

    println!("[Test] Container isolation test completed successfully!");
    println!("[Test] Container is ready for actual workloads");

    // For now, just exit successfully
    // In a real container, this would be where we execute the user's command
    0
}
