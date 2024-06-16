extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};
use pretty_env_logger::env_logger::fmt::TimestampPrecision;

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;
use message_relayer::MessageRelayer;
use metrics::MetricsBuilder;
use proof_storage::{FileSystemProofStorage, GearProofStorage, ProofStorage};
use prover::proving::GenesisConfig;
use relay_merkle_roots::MerkleRootRelayer;

mod message_relayer;
mod metrics;
mod proof_storage;
mod prover_interface;
mod relay_merkle_roots;

const DEFAULT_VARA_RPC: &str = "ws://localhost:8989";
const DEFAULT_ETH_RPC: &str = "http://localhost:8545";
const DEFAULT_PROMETHEUS_ENDPOINT: &str = "0.0.0.0:9090";

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
    #[clap(flatten)]
    prometheus_args: PrometheusArgs,
    /// Block number to start relaying from. If not specified equals to the latest finalized block
    #[arg(long = "from-block")]
    from_block: Option<u32>,
    /// Address of bridging payment program (if not specified, relayer will relay all messages)
    #[arg(long = "bridging-payment-address", env = "BRIDGING_PAYMENT_ADDRESS")]
    bridging_payment_address: Option<String>,
}

#[derive(Args)]
struct RelayMerkleRootsArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    #[clap(flatten)]
    ethereum_args: EthereumArgs,
    #[clap(flatten)]
    prometheus_args: PrometheusArgs,
    #[clap(flatten)]
    proof_storage_args: ProofStorageArgs,
}

#[derive(Args)]
struct VaraEndpointArg {
    /// Address of the VARA RPC endpoint
    #[arg(
        long = "vara-endpoint",
        default_value = DEFAULT_VARA_RPC,
        env = "VARA_RPC"
    )]
    vara_endpoint: String,
}

#[derive(Args)]
struct EthereumArgs {
    /// Address of the ethereum endpoint
    #[arg(
        long = "ethereum-endpoint",
        default_value = DEFAULT_ETH_RPC,
        env = "ETH_RPC"
    )]
    eth_endpoint: String,
    /// Private key for fee payer
    #[arg(long = "eth-fee-payer", env = "ETH_FEE_PAYER")]
    fee_payer: Option<String>,
    /// Ethereum address of relayer contract
    #[arg(long = "relayer-address", env = "ETH_RELAYER_ADDRESS")]
    relayer_address: String,
    /// Ethereum address of message queue contract
    #[arg(long = "mq-address", env = "ETH_MESSAGE_QUEUE_ADDRESS")]
    mq_address: String,
}

#[derive(Args)]
struct PrometheusArgs {
    /// Address of the prometheus endpoint
    #[arg(
        long = "prometheus-endpoint",
        default_value = DEFAULT_PROMETHEUS_ENDPOINT,
        env = "PROMETHEUS_ENDPOINT"
    )]
    endpoint: String,
}

#[derive(Args)]
struct ProofStorageArgs {
    /// Gear fee payer. If not set, proofs are saved to file system
    #[arg(long = "gear-fee-payer", env = "GEAR_FEE_PAYER")]
    gear_fee_payer: Option<String>,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .filter(Some("prover"), log::LevelFilter::Info)
        .filter(Some("relayer"), log::LevelFilter::Info)
        .filter(Some("ethereum-client"), log::LevelFilter::Info)
        .filter(Some("metrics"), log::LevelFilter::Info)
        .format_timestamp(Some(TimestampPrecision::Seconds))
        .init();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::RelayMerkleRoots(args) => {
            let gear_api = create_gear_client(&args.vara_endpoint).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            let proof_storage: Box<dyn ProofStorage> =
                if let Some(fee_payer) = args.proof_storage_args.gear_fee_payer {
                    Box::from(
                        GearProofStorage::new(&args.vara_endpoint.vara_endpoint, &fee_payer)
                            .await
                            .expect("Failed to initilize proof storage"),
                    )
                } else {
                    log::warn!("Fee payer not present, falling back to FileSystemProofStorage");
                    Box::from(FileSystemProofStorage::new("./proof_storage".into()))
                };

            let relayer = MerkleRootRelayer::new(gear_api, eth_api, proof_storage).await;

            MetricsBuilder::new()
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            relayer.run().await.expect("Merkle root relayer failed");
        }
        CliCommands::RelayMessages(args) => {
            let gear_api = create_gear_client(&args.vara_endpoint).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            let bridging_payment_address = args.bridging_payment_address.map(|addr| {
                let arr: [u8; 32] = hex::decode(addr)
                    .expect("Wrong format of bridging-payment-address")
                    .try_into()
                    .expect("Wrong format of bridging-payment-address");

                arr.into()
            });

            let relayer =
                MessageRelayer::new(gear_api, eth_api, args.from_block, bridging_payment_address)
                    .await
                    .unwrap();

            MetricsBuilder::new()
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            relayer.run().await.unwrap();
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
        eth_endpoint,
        mq_address,
        relayer_address,
        fee_payer.as_deref(),
    )
    .unwrap_or_else(|err| panic!("Error while creating ethereum client: {}", err))
}
