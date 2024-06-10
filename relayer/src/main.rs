extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};
use pretty_env_logger::env_logger::fmt::TimestampPrecision;

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;
use prover::proving::GenesisConfig;

mod proof_storage;
mod prover_interface;
mod relay_merkle_roots;
mod relay_messages;

const DEFAULT_VARA_RPC: &str = "ws://localhost:8989";
const DEFAULT_ETH_RPC: &str = "http://localhost:8545";

const GENESIS_CONFIG: GenesisConfig = GenesisConfig {
    authority_set_id: 0,
    // 0xb9853ab2fb585702dfd9040ee8bc9f94dc5b0abd8b0f809ec23fdc0265b21e24
    validator_set_hash: [
        0xb9853ab2, 0xfb585702, 0xdfd9040e, 0xe8bc9f94, 0xdc5b0abd, 0x8b0f809e, 0xc23fdc02,
        0x65b21e24,
    ],
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: CliCommands,
}

#[derive(Subcommand)]
enum CliCommands {
    /// Start service constantly relaying messages to ethereum
    #[clap(visible_alias("rr"))]
    RelayMerkleRoots(RelayMerkleRootsArgs),
    /// Relay message to ethereum
    #[clap(visible_alias("rm"))]
    RelayMessages(RelayMessagesArgs),
}

#[derive(Args)]
struct RelayMessagesArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    #[clap(flatten)]
    ethereum_args: EthereumArgs,
    /// Block number to start relaying from. If not specified equals to the latest finalized block
    #[arg(long = "from-block")]
    from_block: Option<u32>,
}

#[derive(Args)]
struct RelayMerkleRootsArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    #[clap(flatten)]
    ethereum_args: EthereumArgs,
}

#[derive(Args)]
struct VaraEndpointArg {
    /// Address of the VARA RPC endpoint
    #[arg(
        long = "vara-endpoint",
        default_value = DEFAULT_VARA_RPC
    )]
    vara_endpoint: String,
}

#[derive(Args)]
struct EthereumArgs {
    /// Address of the ethereum endpoint
    #[arg(
        long = "ethereum-endpoint",
        default_value = DEFAULT_ETH_RPC
    )]
    eth_endpoint: String,
    /// Private key for fee payer
    #[arg(long = "fee-payer")]
    fee_payer: Option<String>,
    /// Ethereum address of relayer contract
    #[arg(long = "relayer-address")]
    relayer_address: String,
    /// Ethereum address of message queue contract
    #[arg(long = "mq-address")]
    mq_address: String,
}

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .filter(Some("prover"), log::LevelFilter::Info)
        .filter(Some("relayer"), log::LevelFilter::Info)
        .filter(Some("ethereum-client"), log::LevelFilter::Info)
        .format_timestamp(Some(TimestampPrecision::Seconds))
        .init();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::RelayMerkleRoots(args) => {
            let gear_api = create_gear_client(&args.vara_endpoint).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            relay_merkle_roots::run(gear_api, eth_api).await.unwrap();
        }
        CliCommands::RelayMessages(args) => {
            let gear_api = create_gear_client(&args.vara_endpoint).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            relay_messages::run(gear_api, eth_api, args.from_block, [0; 32].into())
                .await
                .unwrap();
        }
    };
}

async fn create_gear_client(args: &VaraEndpointArg) -> GearApi {
    GearApi::new(&args.vara_endpoint)
        .await
        .unwrap_or_else(|err| panic!("Error while creating gear client: {}", err))
}

fn create_eth_client(args: &EthereumArgs) -> EthApi {
    let EthereumArgs {
        eth_endpoint,
        fee_payer,
        relayer_address,
        mq_address,
    } = args;

    EthApi::new(
        &eth_endpoint,
        &mq_address,
        &relayer_address,
        fee_payer.as_deref(),
    )
    .unwrap_or_else(|err| panic!("Error while creating ethereum client: {}", err))
}
