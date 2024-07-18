use clap::Parser;
use tokio::sync::mpsc;

#[cfg(test)]
mod tests;

const SIZE_CHANNEL: usize = 100_000;

#[derive(Debug, Parser)]
struct Args {
    /// Specify ProgramId of the Checkpoint-light-client program
    #[arg(long)]
    program_id: String,

    /// Specify an endpoint providing Beacon API
    #[arg(long)]
    beacon_endpoint: String,

    /// Address of the VARA RPC endpoint
    #[arg(
        long,
        env = "VARA_RPC"
    )]
    vara_endpoint: String,
}

#[tokio::main]
async fn main() {
    let Args {
        program_id,
        beacon_endpoint,
        vara_endpoint,
    } = Args::parse();

    let (sender, receiver) = mpsc::channel::<()>(SIZE_CHANNEL);
}
