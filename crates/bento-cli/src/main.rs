use clap::{Parser, Subcommand, ValueHint};
use libbento::config::Config;           
use log::{error, info};
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
    Start {},
    State {},
    List {},
    Kill {},
    Delete {},
}

fn main() {
    env_logger::init();                   // convenience
    info!("Starting Bento CLI");

    let args = Cli::parse();

    match args.command {
        Commands::Spec {} => println!("Spec"),
        Commands::Create { container_id, bundle } => {
            let cfg_path = bundle.join("config.json");
            match Config::load(&cfg_path) {
                Ok(cfg) => {
                    println!(
                        "Container '{}' validated. rootfs = {}",
                        container_id,
                        cfg.root.path.display()
                    );
                    // → pass `cfg` into your `create()` implementation next
                }
                Err(e) => {
                    error!("Invalid bundle: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Start {} => println!("Start"),
        Commands::State {} => println!("State"),
        Commands::List {} => println!("List"),
        Commands::Kill {} => println!("Kill"),
        Commands::Delete {} => println!("Delete"),
    }
}
