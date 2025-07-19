use clap::{Parser, Subcommand, ValueHint};
use log::info;
use std::path::PathBuf;
use libbento::process::{test_fork_clone};


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
    Start {},
    State {},
    List {},
    Kill {},
    Delete {},
}

fn main() {
    info!("Starting Bento CLI");

    let args = Cli::parse();

    match args.command {
        Commands::Spec {} => {
            println!("Spec");
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
		//let config = Config::default();

            	// Call your fork/clone workflow
            	if let Err(e) = test_fork_clone() {
                	eprintln!("Container creation failed: {e}");
            }
        }
        Commands::Start {} => {
            println!("Start");
        }
        Commands::State {} => {
            println!("State");
        }
        Commands::List {} => {
            println!("List");
        }
        Commands::Kill {} => {
            println!("Kill");
        }
        Commands::Delete {} => {
            println!("Delete");
        }
    }
}
