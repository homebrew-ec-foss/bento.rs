use clap::{Parser, Subcommand, ValueHint};
use log::info;
use std::path::PathBuf;
use libbento::process::{create_container, Config};

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
            println!("Spec command not implemented yet");
        }
        Commands::Create {
            container_id,
            bundle,
        } => {
            println!(
                "Creating container '{}' with bundle '{}'",
                container_id,
                bundle.display()
            );

            let config = Config::default();  // TODO: Load from bundle/config.json

            if let Err(e) = create_container(&config) {
                eprintln!("Container creation failed: {e}");
            }
        }
        Commands::Start { container_id } => {
            println!("Starting container '{}'", container_id);
            // TODO: Implement start logic (e.g., resume from saved state/PID)
            // Example: Load PID from state, signal to start
        }
        Commands::State { container_id } => {
            println!("State of container '{}'", container_id);
            // TODO: Load and print container status (e.g., running, PID)
        }
        Commands::List {} => {
            println!("Listing containers");
            // TODO: List all created container IDs
        }
        Commands::Kill { container_id } => {
            println!("Killing container '{}'", container_id);
            // TODO: Send signal to container PID
        }
        Commands::Delete { container_id } => {
            println!("Deleting container '{}'", container_id);
            // TODO: Cleanup state/rootfs
        }
    }
}

