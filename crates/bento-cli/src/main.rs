// crates/bento-cli/src/main.rs

use clap::{Parser, Subcommand, ValueHint};
use libbento::{
    cgroups::CgroupsConfig,
    process::{
        Config, RootfsPopulationMethod, create_container, delete_container, load_container_state,
        start_container, stop_container,
    },
};
use log::info;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Spec {},
    Create {
        #[arg(required = true)]
        container_id: String,
        #[arg(short, long, required = true, value_hint = ValueHint::FilePath)]
        bundle: PathBuf,
        /// Rootfs population method: 'busybox' for static binary or 'manual' for host binary copying
        #[arg(
            long,
            default_value = "manual",
            help = "Method to populate container rootfs: 'busybox' or 'manual'"
        )]
        population_method: String,

        #[arg(long)]
        memory_limit: Option<String>,

        /// Soft memory limit; throttles before OOM (cgroup v2 memory.high)
        #[arg(long)]
        memory_high: Option<String>,

        #[arg(long)]
        cpu_limit: Option<String>,

        #[arg(long)]
        cpu_weight: Option<u32>,

        #[arg(long)]
        memory_swap_limit: Option<String>,

        #[arg(long)]
        pids_limit: Option<String>,

        #[arg(long, action = clap::ArgAction::SetTrue)]
        no_cgroups: bool,
    },
    Start {
        #[arg(required = true)]
        container_id: String,
    },
    State {
        #[arg(required = true)]
        container_id: String,
    },
    List {},
    Kill {
        #[arg(required = true)]
        container_id: String,
    },
    Delete {
        #[arg(required = true)]
        container_id: String,
    },
    Stats {
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        continuous: bool,
    },
}

fn main() {
    info!("Starting Bento CLI");

    let args = Cli::parse();

    match args.command {
        Commands::Spec {} => {
            todo!("Generate OCI spec template");
        }
        Commands::Create {
            container_id,
            bundle,
            population_method, // Add this parameter
            memory_limit,
            memory_high,
            cpu_limit,
            cpu_weight,
            memory_swap_limit,
            pids_limit,
            no_cgroups,
        } => {
            println!(
                "Creating container '{}' with bundle '{}' using {} method",
                container_id,
                bundle.display(),
                population_method
            );

            let mut config = Config {
                container_id: container_id.clone(),
                bundle_path: bundle.to_string_lossy().to_string(),
                population_method: match population_method.as_str() {
                    "manual" => RootfsPopulationMethod::Manual,
                    _ => RootfsPopulationMethod::BusyBox, // Clear default handling
                },
                ..Config::default() // Use default for remaining fields
            };

            if no_cgroups {
                config.cgroups = CgroupsConfig::default();
            } else {
                let mut cgroups_config = CgroupsConfig::new();

                if let Some(memory) = memory_limit {
                    cgroups_config.memory_max = Some(memory.clone());
                }

                if let Some(cpu) = cpu_limit {
                    cgroups_config.cpu_max = Some(cpu.clone());
                }

                if let Some(weight) = cpu_weight {
                    cgroups_config.cpu_weight = Some(weight);
                }

                if let Some(high) = memory_high {
                    cgroups_config.memory_high = Some(high.clone());
                }

                if let Some(mswap) = memory_swap_limit {
                    cgroups_config.memory_swap_max = Some(mswap.clone());
                }

                if let Some(pids) = pids_limit {
                    cgroups_config.pids_max = Some(pids.clone());
                }

                config.cgroups = cgroups_config;
            }

            match create_container(&config) {
                Ok(_) => println!("Container '{container_id}' created successfully"),
                Err(e) => {
                    eprintln!("Container creation failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Start { container_id } => {
            println!("Starting container '{container_id}'");
            match start_container(&container_id) {
                Ok(_) => println!("Container '{container_id}' started successfully"),
                Err(e) => {
                    eprintln!("Failed to start container '{container_id}': {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::State { container_id } => {
            println!("State of container '{container_id}'");
            match load_container_state(&container_id) {
                Ok(state) => {
                    println!("Container ID: {}", state.id);
                    println!("Status: {}", state.status);
                    println!("PID: {}", state.pid);
                    println!("Bundle Path: {}", state.bundle_path);
                    println!("Created At: {}", state.created_at);
                    println!("Cgroups Enabled: {}", state.cgroup_enabled);
                    if let Some(pipe_path) = &state.start_pipe_path {
                        println!("Start Pipe Path: {}", pipe_path);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to get state for container '{container_id}': {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::List {} => {
            println!("Listing containers");
            match libbento::process::list_containers() {
                Ok(containers) => {
                    if containers.is_empty() {
                        println!("No containers found");
                    } else {
                        // Display containers in a formatted table
                        println!(
                            "{:<20} {:<10} {:<15} {:<10} {:<30}",
                            "CONTAINER ID", "STATUS", "PID", "RUNTIME", "BUNDLE"
                        );
                        println!("{}", "-".repeat(95));

                        for container in containers {
                            println!(
                                "{:<20} {:<10} {:<15} {:<10} {:<30}",
                                container.id,
                                container.display_status(),
                                container.pid,
                                format!("{:?}", container.runtime_status).to_lowercase(),
                                container.bundle_path
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to list containers: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Kill { container_id } => {
            println!("Killing container '{container_id}'");
            match stop_container(&container_id) {
                Ok(_) => println!("Container '{container_id}' stopped successfully"),
                Err(e) => {
                    eprintln!("Failed to stop container '{container_id}': {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Delete { container_id } => {
            println!("Deleting container '{container_id}'");
            match delete_container(&container_id) {
                Ok(_) => println!("Container '{container_id}' deleted successfully"),
                Err(e) => {
                    eprintln!("Failed to delete container '{container_id}': {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Stats { continuous } => {
            loop {
                // Clear screen
                print!("\x1B[2J\x1B[1;1H");

                let containers = libbento::process::list_containers().unwrap();

                println!("CONTAINER RESOURCE USAGE");
                println!("{}", "=".repeat(80));
                println!(
                    "{:<15} {:<10} {:<15} {:<15} {:<10} {:<10}",
                    "CONTAINER ID", "STATUS", "MEMORY", "CPU TIME", "PIDS", "PID"
                );
                println!("{}", "-".repeat(80));

                for container in containers {
                    let pids_display = if let Some(stats) = &container.cgroup_stats {
                        match stats.pids_limit {
                            Some(limit) => format!("{}/{}", stats.pids_current, limit),
                            None => format!("{}/∞", stats.pids_current),
                        }
                    } else {
                        "N/A".to_string()
                    };

                    println!(
                        "{:<15} {:<10} {:<15} {:<15} {:<10} {:<10}",
                        container.id,
                        container.display_status(),
                        container.memory_usage_display(),
                        container.cpu_usage_display(),
                        pids_display,
                        container.pid,
                    );
                }

                if !continuous {
                    break;
                }

                println!("\nPress Ctrl+C to exit");
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }
}
