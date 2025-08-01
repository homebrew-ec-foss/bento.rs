// crates/bento-cli/src/main.rs

use clap::{Parser, Subcommand, ValueHint};
use libbento::process::{Config, RootfsPopulationMethod, create_container, start_container};
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
            default_value = "busybox",
            help = "Method to populate container rootfs: 'busybox' or 'manual'"
        )]
        population_method: String,
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
        } => {
            println!(
                "Creating container '{}' with bundle '{}' using {} method",
                container_id,
                bundle.display(),
                population_method
            );

            let config = Config {
                container_id: container_id.clone(),
                bundle_path: bundle.to_string_lossy().to_string(),
                population_method: match population_method.as_str() {
                    "manual" => RootfsPopulationMethod::Manual,
                    _ => RootfsPopulationMethod::BusyBox, // Clear default handling
                },
                ..Config::default() // Use default for remaining fields
            };
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
            todo!("Load and display container status from state file");
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
            todo!("Send termination signal to container process");
        }
        Commands::Delete { container_id } => {
            println!("Deleting container '{container_id}'");
            todo!("Clean up container resources");
        }
    }
}
