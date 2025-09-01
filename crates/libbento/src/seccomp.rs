use crate::config2::SeccompConfig;
use anyhow::{Context, Result};
use libseccomp::{ScmpAction, ScmpArch, ScmpFilterContext, ScmpSyscall};

// this holds the configurations from seccompConfig, which defines the filtering rules.
pub struct SeccompFilter {
    config: SeccompConfig,
}

impl SeccompFilter {
    pub fn new(config: SeccompConfig) -> Self {
        // Here creating a new instance
        Self { config }
    }
    // the execution starts from here
    pub fn apply(&self) -> Result<()> {
        self.validate_config()?; // This is to validate the actual config.
        let default_action = self
            .parse_action(&self.config.default_action)
            .context("Invalid default action")?;
        let mut ctx = ScmpFilterContext::new_filter(default_action)
            .context("Failed to initialize seccomp filter")?;

        self.add_architectures(&mut ctx)?;
        self.add_syscall_rules(&mut ctx)?;

        // load the filter into the kernel
        ctx.load()
            .context("Failed to load seccomp filter into kernel")?;
        println!("Filter program loaded into the kernel successfully.");
        Ok(())
    }

    fn parse_action(&self, action: &str) -> Result<ScmpAction> {
        match action {
            "SCMP_ACT_ERRNO" => Ok(ScmpAction::Errno(libc::EPERM)),
            "SCMP_ACT_KILL" => Ok(ScmpAction::KillThread),
            "SCMP_ACT_KILL_PROCESS" => Ok(ScmpAction::KillProcess),
            "SCMP_ACT_ALLOW" => Ok(ScmpAction::Allow),
            "SCMP_ACT_TRAP" => Ok(ScmpAction::Trap),
            "SCMP_ACT_TRACE" => Ok(ScmpAction::Trace(0)),
            _ => anyhow::bail!("Invalid action: {}", action),
        }
    }

    // this is helful to validate config - instead of exiting when there's 0 arch
    fn validate_config(&self) -> Result<()> {
        if self.config.architectures.is_empty() {
            anyhow::bail!("At least one architecture must be specified");
        }
        if self.config.syscalls.is_empty() {
            println!("No syscall rules specified - this might be insecure");
        }
        // Validate that we have some allowed syscalls for basic ops
        let has_exit = self.config.syscalls.iter().any(|rule| {
            rule.action == "SCMP_ACT_ALLOW"
                && (rule.names.contains(&"exit".to_string())
                    || rule.names.contains(&"exit_group".to_string()))
        });
        if !has_exit {
            println!("No exit syscalls allowed - process may not be able to terminate cleanly");
        }
        println!("Seccomp Config Validation successful.");
        Ok(())
    }

    fn add_architectures(&self, ctx: &mut ScmpFilterContext) -> Result<()> {
        for arch in &self.config.architectures {
            let scmp_arch = match arch.as_str() {
                "SCMP_ARCH_X86_64" => ScmpArch::X8664,
                "SCMP_ARCH_X86" => ScmpArch::X86,
                "SCMP_ARCH_X32" => ScmpArch::X32,
                "SCMP_ARCH_AARCH64" => ScmpArch::Aarch64,
                "SCMP_ARCH_ARM" => ScmpArch::Arm,
                _ => anyhow::bail!("Invalid architecture: {arch}"),
            };
            ctx.add_arch(scmp_arch)
                .with_context(|| format!("Failed to add architecture {arch}"))?;
            println!("Added architecture successfully : {arch}");
        }
        Ok(())
    }

    fn add_syscall_rules(&self, ctx: &mut ScmpFilterContext) -> Result<()> {
        for rule in &self.config.syscalls {
            let action = self
                .parse_action(&rule.action)
                .with_context(|| format!("Invalid action in rule: {}", rule.action))?;

            for syscall_name in &rule.names {
                match ScmpSyscall::from_name(syscall_name.as_str()) {
                    Ok(syscall) => {
                        ctx.add_rule(action, syscall)
                            .with_context(|| format!("Failed to add rule for {syscall_name}"))?;
                        println!("Added rule successfully : {syscall_name} -> {:?}", action);
                    }
                    Err(_) => {
                        println!("Unknown syscall '{syscall_name}' - skipping");
                    }
                }
            }
        }
        Ok(())
    }
}
