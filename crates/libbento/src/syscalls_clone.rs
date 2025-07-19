use nix::sched::{clone, CloneFlags};
use nix::sys::wait::waitpid;
use nix::unistd::{getpid, execvp, sethostname};
use std::ffi::CString;
use libc;

use crate::process::Config;  // Correct import

/// Clones a new init process with the given flags, running the container command.
/// Executes in the isolated namespace.
pub fn clone_init(config: &Config, flags: CloneFlags) -> Result<(), nix::Error> {
    let mut stack = [0u8; 4096 * 4];

    let cb = Box::new(move || {
        println!("Init process: PID {}", getpid());

        if let Err(e) = sethostname(config.hostname) {
            println!("sethostname failed: {}", e);
            return 1;
        }

        let args: Vec<CString> = config.args.iter().map(|&s| CString::new(s).unwrap()).collect();
        let ref_args: Vec<&CString> = args.iter().collect();
        let cmd = &args[0];

        match execvp(cmd, &ref_args) {
            Ok(_) => 0,  // Success (execvp replaces process, so this won't run)
            Err(e) => {
                println!("execvp failed: {}", e);
                1
            }
        }
    });

    let pid = unsafe { clone(cb, &mut stack, flags, Some(libc::SIGCHLD)) }?;
    waitpid(pid, None)?;
    Ok(())
}

