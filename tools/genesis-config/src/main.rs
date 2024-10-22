use clap::{Args, Parser, Subcommand};
use gear_rpc_client::GearApi;
use serde::{Deserialize, Serialize};

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
    /// Address of the Gear RPC endpoint
    #[arg(
        long = "gear-endpoint",
        default_value = "wss://testnet.vara.network",
        env = "GEAR_RPC"
    )]
    gear_endpoint: String,
    /// Port of the Gear RPC endpoint
    #[arg(long = "gear-port", default_value = "443", env = "GEAR_PORT")]
    gear_port: u16,
    /// Block number to fetch the genesis config for. If not specified, the latest block will be used
    #[arg(long = "block")]
    block: Option<u32>,
    /// Whether to write result to a file
    #[arg(long, action)]
    write_to_file: bool,
}

#[derive(Deserialize, Serialize)]
struct GenesisConfigToml {
    authority_set_id: u64,
    authority_set_hash: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let CliCommands::Fetch(args) = cli.command;

    let gear_api = GearApi::new(&format!("{}:{}", args.gear_endpoint, args.gear_port))
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

    if args.write_to_file {
        let config = GenesisConfigToml {
            authority_set_id: state.authority_set_id,
            authority_set_hash: hex::encode(state.authority_set_hash),
        };

        let data = toml::to_string(&config).expect("Failed to serialize config");

        std::fs::write("./GenesisConfig.toml", data)
            .expect("Failed to write genesis config to file");
    }
}
