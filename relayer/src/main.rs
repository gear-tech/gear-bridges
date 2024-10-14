extern crate pretty_env_logger;

use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use ethereum_beacon_client::BeaconClient;
use gclient::{GearApi as GclientGearApi, WSAddress};
use pretty_env_logger::env_logger::fmt::TimestampPrecision;
use primitive_types::{H160, H256};

use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use message_relayer::{eth_to_gear, gear_to_eth};
use proof_storage::{FileSystemProofStorage, GearProofStorage, ProofStorage};
use relay_merkle_roots::MerkleRootRelayer;
use utils_prometheus::MetricsBuilder;

mod ethereum_beacon_client;
mod ethereum_checkpoints;
mod genesis_config;
mod message_relayer;
mod proof_storage;
mod prover_interface;
mod relay_merkle_roots;

const DEFAULT_VARA_RPC: &str = "ws://localhost:8989";
const DEFAULT_ETH_BEACON_RPC: &str = "http://localhost:50000";
const DEFAULT_ETH_RPC: &str = "http://localhost:8545";
const DEFAULT_PROMETHEUS_ENDPOINT: &str = "0.0.0.0:9090";

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
    /// Start service constantly relaying messages to ethereum
    #[clap(visible_alias("rr"))]
    RelayMerkleRoots(RelayMerkleRootsArgs),
    /// Relay message to ethereum
    #[clap(visible_alias("rm"))]
    RelayMessages(RelayMessagesArgs),
    /// Start service constantly relaying Ethereum checkpoints to the Vara program
    RelayCheckpoints(RelayCheckpointsArgs),
    /// Fetch authority set hash and id at specified block
    #[clap(visible_alias("fs"))]
    FetchAuthoritySetState(FetchAuthoritySetStateArgs),
    /// Relay the ERC20 tokens to the Vara network
    RelayErc20(RelayErc20Args),
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
struct FetchAuthoritySetStateArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    /// Block number which contains desired authority set
    block_number: Option<u32>,
    /// Wether to write fetched authority set state as genesis config
    #[clap(long, action)]
    write_to_file: bool,
}

// TODO: Separate domain and port.
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
struct BeaconRpcArgs {
    /// Address of the ethereum beacon RPC endpoint
    #[arg(
        long = "ethereum-beacon-rpc",
        default_value = DEFAULT_ETH_BEACON_RPC,
        env = "ETH_BEACON_RPC"
    )]
    endpoint: String,

    /// Timeout in seconds for requests to the ethereum beacon RPC
    #[arg(long = "ethereum-beacon-rpc-timeout", env = "ETH_BEACON_RPC_TIMEOUT")]
    timeout: Option<u64>,
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

#[derive(Args)]
struct RelayCheckpointsArgs {
    /// Specify ProgramId of the Checkpoint-light-client program
    #[arg(long, env = "CHECKPOINT_LIGHT_CLIENT_ADDRESS")]
    program_id: String,

    /// Specify the endpoint providing Beacon API
    #[arg(long, env = "BEACON_ENDPOINT")]
    beacon_endpoint: String,

    /// Specify the timeout in seconds for requests to the Beacon API endpoint
    #[arg(long, default_value = "120", env = "BEACON_TIMEOUT")]
    beacon_timeout: u64,

    /// Domain of the VARA RPC endpoint
    #[arg(long, default_value = "ws://127.0.0.1", env = "VARA_DOMAIN")]
    vara_domain: String,

    /// Port of the VARA RPC endpoint
    #[arg(long, default_value = "9944", env = "VARA_PORT")]
    vara_port: u16,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = "//Alice", env = "VARA_SURI")]
    vara_suri: String,

    #[clap(flatten)]
    prometheus_args: PrometheusArgs,
}

#[derive(Args)]
struct RelayErc20Args {
    /// Address of the ERC20Treasury contract on ethereum
    #[arg(long = "erc20-treasury-address", env = "ERC20_TREASURY_ADDRESS")]
    erc20_treasury_address: String,

    /// Address of the checkpoint-light-client program on gear
    #[arg(
        long = "checkpoint-light-client-address",
        env = "CHECKPOINT_LIGHT_CLIENT_ADDRESS"
    )]
    checkpoint_light_client_address: String,

    /// Address of the ethereum-event-client program on gear
    #[arg(
        long = "ethereum-event-client-address",
        env = "ETHEREUM_EVENT_CLIENT_ADDRESS"
    )]
    ethereum_event_client_address: String,

    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, env = "VARA_SURI")]
    vara_suri: String,

    #[clap(flatten)]
    ethereum_args: EthereumArgs,

    #[clap(flatten)]
    beacon_rpc: BeaconRpcArgs,

    #[clap(flatten)]
    prometheus_args: PrometheusArgs,
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
        .parse_default_env()
        .init();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::RelayMerkleRoots(args) => {
            let gear_api = create_gear_client(&args.vara_endpoint).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            let mut metrics = MetricsBuilder::new();

            let proof_storage: Box<dyn ProofStorage> =
                if let Some(fee_payer) = args.proof_storage_args.gear_fee_payer {
                    let proof_storage = GearProofStorage::new(
                        &args.vara_endpoint.vara_endpoint,
                        &fee_payer,
                        "./onchain_proof_storage_data".into(),
                    )
                    .await
                    .expect("Failed to initilize proof storage");

                    metrics = metrics.register_service(&proof_storage);

                    Box::from(proof_storage)
                } else {
                    log::warn!("Fee payer not present, falling back to FileSystemProofStorage");
                    Box::from(FileSystemProofStorage::new("./proof_storage".into()))
                };

            let genesis_config = genesis_config::load_from_file();

            let relayer =
                MerkleRootRelayer::new(gear_api, eth_api, genesis_config, proof_storage).await;

            metrics
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            relayer.run().await.expect("Merkle root relayer failed");
        }
        CliCommands::RelayMessages(args) => {
            let gear_api = create_gear_client(&args.vara_endpoint).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            if let Some(bridging_payment_address) = args.bridging_payment_address {
                let bridging_payment_address = decode_h256(&bridging_payment_address);

                let relayer = gear_to_eth::paid_token_transfers::Relayer::new(
                    gear_api,
                    eth_api,
                    args.from_block,
                    bridging_payment_address,
                )
                .await
                .unwrap();

                MetricsBuilder::new()
                    .register_service(&relayer)
                    .build()
                    .run(args.prometheus_args.endpoint)
                    .await;

                relayer.run();
            } else {
                let relayer = gear_to_eth::all_token_transfers::Relayer::new(
                    gear_api,
                    eth_api,
                    args.from_block,
                )
                .await
                .unwrap();

                MetricsBuilder::new()
                    .register_service(&relayer)
                    .build()
                    .run(args.prometheus_args.endpoint)
                    .await;

                relayer.run();
            }

            loop {
                // relayer.run() spawns thread and exits, so we need to add this loop after calling run.
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        CliCommands::RelayCheckpoints(args) => ethereum_checkpoints::relay(args).await,
        CliCommands::FetchAuthoritySetState(args) => {
            let gear_api = create_gear_client(&args.vara_endpoint).await;
            genesis_config::fetch_from_chain(gear_api, args.block_number, args.write_to_file)
                .await
                .expect("Failed to fetch authority set state");
        }
        CliCommands::RelayErc20(args) => {
            let eth_api = create_eth_client(&args.ethereum_args);
            let beacon_client = create_beacon_client(&args.beacon_rpc).await;

            let gear_api = create_gear_client(&args.vara_endpoint).await;
            let gclient_client = create_gclient_client(&args.vara_endpoint, &args.vara_suri).await;

            let erc20_treasury_address = decode_h160(&args.erc20_treasury_address);
            let checkpoint_light_client_address =
                decode_h256(&args.checkpoint_light_client_address);
            let ethereum_event_client_address = decode_h256(&args.ethereum_event_client_address);

            let relayer = eth_to_gear::all_token_transfers::Relayer::new(
                gear_api,
                gclient_client,
                eth_api,
                beacon_client,
                erc20_treasury_address,
                checkpoint_light_client_address,
                ethereum_event_client_address,
            )
            .await
            .expect("Failed to create relayer");

            MetricsBuilder::new()
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            relayer.run();

            loop {
                // relayer.run() spawns thread and exits, so we need to add this loop after calling run.
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
}

async fn create_gclient_client(args: &VaraEndpointArg, vara_suri: &str) -> GclientGearApi {
    let endpoint_parts: Vec<_> = args.vara_endpoint.split(':').collect();
    let [domain_1, domain_2, port] = endpoint_parts
        .try_into()
        .expect("Invalid gear endpoint provided");

    let domain = [domain_1, domain_2].join(":");
    let port: u16 = port.parse().expect("Invalid gear endpoint provided");

    GclientGearApi::init_with(WSAddress::new(domain, Some(port)), vara_suri)
        .await
        .expect("Failed to create gclient client")
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

async fn create_beacon_client(args: &BeaconRpcArgs) -> BeaconClient {
    let timeout = args.timeout.map(Duration::from_secs);

    BeaconClient::connect(args.endpoint.clone(), timeout)
        .await
        .expect("Failed to create beacon client")
}

fn decode_h256(hex: &str) -> H256 {
    let data: [u8; 32] = decode_byte_array(hex);
    data.into()
}

fn decode_h160(hex: &str) -> H160 {
    let data: [u8; 20] = decode_byte_array(hex);
    data.into()
}

fn decode_byte_array<const LEN: usize>(hex: &str) -> [u8; LEN] {
    let address = if &hex[..2] == "0x" { &hex[2..] } else { hex };

    hex::decode(address)
        .unwrap_or_else(|_| panic!("Wrong address format: {}", hex))
        .try_into()
        .unwrap_or_else(|_| panic!("Wrong address length: {}", hex))
}
