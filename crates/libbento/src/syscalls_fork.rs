use nix::unistd::{fork, ForkResult, Pid};
use nix::sys::wait::waitpid;
use nix::Error;

pub fn fork_intermediate<F>(child_logic: F) -> Result<Pid, Error>
where
    F: FnOnce() -> isize,
{
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            println!("Parent: Forked intermediate PID {}", child);
            waitpid(child, None)?;
            Ok(child)
        }
        Ok(ForkResult::Child) => {
            let exit_code = child_logic();
            std::process::exit(exit_code as i32);
        }
        Err(e) => Err(e),
    }
}

