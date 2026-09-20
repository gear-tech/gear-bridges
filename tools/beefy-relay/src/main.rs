use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod ethereum;
mod rehearsal;
mod source;

#[derive(Parser)]
#[command(about = "Bounded, prover-free local Vara BEEFY bridge rehearsal")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Rehearse {
        #[arg(long)]
        gear_node: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Rehearse {
            gear_node,
            output_dir,
        } => rehearsal::run(&gear_node, &output_dir).await,
    }
}
