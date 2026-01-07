use clap::{Args, Parser, Subcommand};
use gear_rpc_client::GearApi;
use cli_utils::GearConnectionArgs;

const GEAR_RPC_RETRIES: u8 = 3;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: CliCommands,
}

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
enum CliCommands {
    /// Fetch genesis config from chain
    #[clap(visible_alias("f"))]
    Fetch(FetchArgs),
}

#[derive(Args)]
struct FetchArgs {
    #[clap(flatten)]
    gear_connection: GearConnectionArgs,

    /// Block number to fetch the genesis config for. If not specified, the latest block will be used
    #[arg(long = "block")]
    block: Option<u32>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let CliCommands::Fetch(args) = cli.command;

    let gear_api = GearApi::new(
        &args.gear_connection.get_endpoint().expect("Invalid URL"),
        GEAR_RPC_RETRIES
    )
    .await
    .expect("Failed to create Gear API");

    let block = match args.block {
        Some(block) => Some(
            gear_api
                .block_number_to_hash(block)
                .await
                .expect("Failed to fetch block hash by number"),
        ),
        None => None,
    };

    let state = gear_api
        .authority_set_state(block)
        .await
        .expect("Failed to fetch authority set state");

    println!("Authority set id: {}", state.authority_set_id);
    println!(
        "Authority set hash: {}",
        hex::encode(state.authority_set_hash)
    );
}
