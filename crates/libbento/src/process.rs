use nix::sched::{clone, CloneFlags};
use nix::unistd::{fork, ForkResult, getpid, execvp, sethostname};
use nix::sys::wait::waitpid;
use std::ffi::CString;
use libc;

pub struct Config {
    pub root_path: &'static str,
    pub args: Vec<&'static str>,
    pub hostname: &'static str,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_path: "/tmp/bento-rootfs",
            args: vec!["/bin/sh"],
            hostname: "bento-container",
        }
    }
}

pub fn test_fork_clone() -> nix::Result<()> {
    let config = Config::default();

    println!("Parent (main) process: PID = {}", getpid());

    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            println!("Created intermediate process with PID = {}", child);
            waitpid(child, None)?;
        }
        ForkResult::Child => {
            println!("Intermediate process: PID = {}", getpid());

            let mut stack = [0u8; 4096 * 4];

            let flags = CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWNS;
            let cb = Box::new(move || {
                // This closure runs as "init" process
                sethostname(config.hostname).expect("hostname failed");
                println!("Init (container PID 1): new hostname set, my host PID: {}", getpid());

                let args: Vec<CString> = config.args.iter()
                    .map(|&s| CString::new(s).unwrap()).collect();
                let ref_args: Vec<&CString> = args.iter().collect();

                let sh = &args[0];
                execvp(sh, &ref_args).expect("execvp failed");

                0 // If execvp fails
            });

            let pid = unsafe { clone(cb, &mut stack, flags, Some(libc::SIGCHLD)) }?;
            waitpid(pid, None)?;
        }
    }
    Ok(())
}

