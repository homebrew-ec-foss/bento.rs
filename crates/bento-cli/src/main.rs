use clap::{Parser, Subcommand, ValueHint};
use libbento::{config::Config, process::create_container};
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
        } => {
            println!(
                "Creating container '{}' with bundle '{}'",
                container_id,
                bundle.display()
            );

            //let config = Config::default(); // TODO: Load from bundle/config.json

            match Config::new_config() {
                Ok(config) => {
                    if let Err(e) = create_container(&config) {
                        eprintln!("Container creation failed: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("Container creation failed: {e}");
                }
            }
        }
        Commands::Start { container_id } => {
            println!("Starting container '{container_id}'");
            todo!("Generate OCI spec template");
        }
        Commands::State { container_id } => {
            println!("State of container '{container_id}'");
            todo!("Load and display container status from state file");
        }
        Commands::List {} => {
            println!("Listing containers");
            todo!("Enumerate all container state files");
        }
        Commands::Kill { container_id } => {
            println!("Killing container '{container_id}'");
            todo!("Send termination signal to container process");
        }
        Commands::Delete { container_id } => {
            println!("Deleting container '{container_id}'");
            todo!("Clean up container state and resources");
        }
    }
}
