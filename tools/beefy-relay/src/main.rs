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

fn message_hash(message: &gear_rpc_client::dto::Message) -> beefy_relay::Hash32 {
    let mut preimage = Vec::with_capacity(84 + message.payload.len());
    preimage.extend_from_slice(&message.nonce_be);
    preimage.extend_from_slice(&message.source);
    preimage.extend_from_slice(&message.destination);
    preimage.extend_from_slice(&message.payload);
    beefy_relay::keccak256(&preimage)
}
