use crate::binary_checker::BinaryChecker;
use anyhow::{Result, anyhow};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub port_mappings: Vec<PortMapping>,
    pub command: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: Protocol,
    pub bind_addr: String,
}

#[derive(Debug, Clone)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl NetworkConfig {
    pub fn new(command: Vec<String>) -> Self {
        Self {
            port_mappings: Vec::new(),
            command,
        }
    }

    pub fn with_ports(mut self, ports: Vec<PortMapping>) -> Self {
        self.port_mappings = ports;
        self
    }
}

pub fn setup_network(config: &NetworkConfig) -> Result<()> {
    println!("🚀 Setting up networking...");

    BinaryChecker::validate_required_binaries()?;

    let can_unshare = test_unshare_capability();
    if !can_unshare {
        println!("⚠️ Cannot create network namespaces (requires privileges or sysctl settings)");
        println!("💡 Running command in current namespace...");
        return run_in_current_namespace(config);
    }

    let mut cmd = Command::new("unshare");
    cmd.args(["--net", "--fork"]);
    cmd.arg("sh").arg("-c");

    let mut setup_script = String::new();
    setup_script.push_str(
        "(slirp4netns --configure --mtu=65520 --disable-host-loopback $$ tap0 &) && sleep 2\n",
    );

    if !config.port_mappings.is_empty() {
        println!("⚠️ Port forwarding not yet implemented in direct mode");
    }

    for (i, arg) in config.command.iter().enumerate() {
        if i > 0 {
            setup_script.push(' ');
        }
        if arg.contains(' ') || arg.contains('"') || arg.contains('\'') {
            setup_script.push_str(&format!("'{}'", arg.replace('\'', "'\\''")));
        } else {
            setup_script.push_str(arg);
        }
    }

    cmd.arg(setup_script);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    println!("🌐 Starting container...");
    let status = cmd
        .status()
        .map_err(|e| anyhow!("Failed to execute: {}", e))?;
    if !status.success() {
        return Err(anyhow!("Network setup failed: {}", status));
    }
    Ok(())
}

fn test_unshare_capability() -> bool {
    let test_result = Command::new("unshare").args(["--net", "true"]).output();

    match test_result {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

fn run_in_current_namespace(config: &NetworkConfig) -> Result<()> {
    println!("🚀 Executing in current namespace (no isolation)...");

    if !config.port_mappings.is_empty() {
        println!("⚠️ Port forwarding not available without network namespace");
    }

    if config.command.is_empty() {
        return Err(anyhow!("No command specified"));
    }

    let status = Command::new(&config.command[0])
        .args(&config.command[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow!("Failed to execute: {}", e))?;

    if !status.success() {
        return Err(anyhow!("Command failed: {}", status));
    }
    Ok(())
}

pub fn parse_port_mappings(port_str: &str) -> Vec<PortMapping> {
    port_str
        .split(',')
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return None;
            }

            let parts: Vec<&str> = if trimmed.contains(':') {
                trimmed.split(':').collect()
            } else {
                return None;
            };

            match parts.len() {
                2 => {
                    let host_port = parts[0].parse().ok()?;
                    let (container_port, protocol) = parse_port_and_protocol(parts[1]);
                    Some(PortMapping {
                        host_port,
                        container_port: container_port?,
                        protocol,
                        bind_addr: "127.0.0.1".to_string(),
                    })
                }
                3 => {
                    let bind_addr = parts[0].to_string();
                    let host_port = parts[1].parse().ok()?;
                    let (container_port, protocol) = parse_port_and_protocol(parts[2]);
                    Some(PortMapping {
                        host_port,
                        container_port: container_port?,
                        protocol,
                        bind_addr,
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn parse_port_and_protocol(port_str: &str) -> (Option<u16>, Protocol) {
    if let Some(slash_pos) = port_str.find('/') {
        let port = port_str[..slash_pos].parse().ok();
        let protocol = match &port_str[slash_pos + 1..] {
            "udp" => Protocol::Udp,
            _ => Protocol::Tcp,
        };
        (port, protocol)
    } else {
        (port_str.parse().ok(), Protocol::Tcp)
    }
}
