use nix::sched::CloneFlags;
use nix::unistd::getpid;
use nix::Error;  // For Result error type

use crate::syscalls_fork::fork_intermediate;
use crate::syscalls_unshare::unshare_namespaces;
//use crate::syscalls_clone::clone_init;

pub struct Config {
    pub root_path: &'static str,
    pub args: Vec<&'static str>,
    pub hostname: &'static str,
    pub rootless: bool,  // Toggle for rootless mode
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_path: "/tmp/bento-rootfs",
            args: vec!["/usr/bin/id"],
            hostname: "bento-container",
            rootless: true,  // Default to rootless for Bento.rs goals
        }
    }
}

use std::ffi::CString;
use nix::unistd::execvp;

pub fn create_container(config: &Config) -> Result<(), Error> {
    println!("Parent PID: {}", getpid());

    fork_intermediate(|| {
        println!("Intermediate PID: {}", getpid());

        let mut flags = CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWNS;
        if config.rootless {
            flags |= CloneFlags::CLONE_NEWUSER;
        }

        if unshare_namespaces(flags).is_err() {
            eprintln!("Error: unshare_namespaces failed");
            return 1;
        }

        // The clone_init call fails because we don't have CAP_SYS_ADMIN after unsharing the user namespace without mapping.
        // We will comment it out for now and execute the command directly in the intermediate process.
        /*
        if clone_init(config, flags).is_err() {
            return 1;
        }
        */

        println!("Namespaces unshared successfully. Executing command...");

        let command = CString::new(config.args[0]).unwrap();
        let args: Vec<CString> = config.args.iter().map(|s| CString::new(*s).unwrap()).collect();

        match execvp(&command, &args) {
            Ok(_) => 0, // execvp replaces the process, so this is not reached on success.
            Err(e) => {
                eprintln!("Error: execvp failed: {}", e);
                1
            }
        }
    })?;

    Ok(())
}

