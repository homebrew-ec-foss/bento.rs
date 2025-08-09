use clap::{Parser, Subcommand, ValueHint};
use libbento::process::{Config as ProcessConfig, create_container};
use libbento::config::Config as OciConfig;           
use log::{info, error};
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
    env_logger::init();
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
	    let cfg_path = bundle.join("config.json");
	    match OciConfig::load(&cfg_path) {
		Ok(cfg) => {
		    println!(
		        "Container '{}' validated. rootfs = {}",
		        container_id,
		        cfg.root.path.display()
		    );
		    println!(
		        "Creating container '{}' with bundle '{}'",
		        container_id,
		        bundle.display()
		    );

		    // Build ProcessConfig from OciConfig
		    let process_config = ProcessConfig {
		        root_path: bundle.join(&cfg.root.path).to_string_lossy().into_owned(),
		        args: cfg
		            .process
		            .as_ref()
		            .map(|p| p.args.clone())
		            .unwrap_or_default(),
		        hostname: cfg.hostname.clone().unwrap_or_else(|| "bento-container".to_string()),
		        rootless: false, // or detect from `cfg.linux` if needed
		        bundle_path: bundle.to_string_lossy().into_owned(),
		        container_id: container_id.clone(),
		    };

		    if let Err(e) = create_container(&process_config) {
		        eprintln!("Container creation failed: {e}");
		    }
		}
		Err(e) => {
		    error!("Invalid bundle: {}", e);
		    std::process::exit(1);
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
