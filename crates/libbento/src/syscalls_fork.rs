use nix::unistd::{fork, ForkResult, Pid};
use nix::sys::wait::waitpid;
use anyhow::{Result, anyhow};

pub fn fork_intermediate<P, C>(parent_logic: P, child_logic: C) -> Result<Pid>
where
    P: FnOnce(Pid) -> Result<()>,
    C: FnOnce() -> isize,
{
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            // Run parent-side logic (e.g., UID/GID mapping)
            parent_logic(child)?;

            // Wait for the child to exit
            waitpid(child, None)?;
            Ok(child)
        }
        Ok(ForkResult::Child) => {
            // Run child-side logic and then exit
            let exit_code = child_logic();
            std::process::exit(exit_code as i32);
        }
        Err(e) => Err(anyhow!("Fork failed: {}", e)),
    }
}

