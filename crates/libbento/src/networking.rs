use crate::binary_checker::BinaryChecker;
use anyhow::{Result, anyhow};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub port_mappings: Vec<String>,
    pub command: Vec<String>,
}

impl NetworkConfig {
    pub fn new(command: Vec<String>) -> Self {
        Self {
            port_mappings: Vec::new(),
            command,
        }
    }

    pub fn with_ports(mut self, ports: Vec<String>) -> Self {
        self.port_mappings = ports;
        self
    }
}

pub fn setup_network(config: &NetworkConfig) -> Result<()> {
    BinaryChecker::validate_required_binaries()?;

    let mut cmd = Command::new("rootlesskit");

    cmd.arg("--net=slirp4netns");
    cmd.arg("--disable-host-loopback");

    // Use builtin port driver for port forwarding if we have port mappings
    if !config.port_mappings.is_empty() {
        cmd.arg("--port-driver=builtin");
        for port_mapping in &config.port_mappings {
            cmd.arg(format!("--publish={}", port_mapping));
        }
    }

    cmd.arg("--");
    cmd.args(&config.command);

    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    println!("🚀 Starting container with slirp4netns networking...");
    println!("Command: {:?}", cmd);

    let status = cmd
        .status()
        .map_err(|e| anyhow!("Failed to execute rootlesskit: {}", e))?;

    if !status.success() {
        return Err(anyhow!("rootlesskit exited with status: {}", status));
    }

    Ok(())
}

pub fn parse_port_mappings(port_str: &str) -> Vec<String> {
    port_str
        .split(',')
        .map(|s| {
            let trimmed = s.trim();

            // Check if it already has a bind address
            if trimmed.starts_with("127.0.0.1:") || trimmed.starts_with("0.0.0.0:") {
                // Add /tcp if no protocol is specified
                if trimmed.contains('/') {
                    trimmed.to_string()
                } else {
                    format!("{}/tcp", trimmed)
                }
            } else {
                // Add bind address and protocol
                if trimmed.contains('/') {
                    format!("127.0.0.1:{}", trimmed)
                } else {
                    format!("127.0.0.1:{}/tcp", trimmed)
                }
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}
