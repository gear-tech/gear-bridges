use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use message_relayer::{all_token_transfers, paid_token_transfers};

use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use proof_storage::{FileSystemProofStorage, GearProofStorage, ProofStorage};
use prover::proving::GenesisConfig;
use relay_merkle_roots::MerkleRootRelayer;
use utils_prometheus::MetricsBuilder;

mod erc20;
mod ethereum_checkpoints;
mod message_relayer;
mod proof_storage;
mod prover_interface;
mod relay_merkle_roots;

const DEFAULT_VARA_RPC: &str = "ws://localhost:8989";
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
    genesis_config_args: GenesisConfigArgs,
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

#[derive(Args)]
struct GenesisConfigArgs {
    /// Authority set hash used in genesis config
    #[arg(long = "authority-set-hash", env = "GENESIS_CONFIG_AUTHORITY_SET_HASH")]
    authority_set_hash: String,
    /// Authority set id used in genesis config
    #[arg(long = "authority-set-id", env = "GENESIS_CONFIG_AUTHORITY_SET_ID")]
    authority_set_id: u64,
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
    /// Specify ProgramId of the program
    #[arg(long, env = "ADDRESS")]
    program_id: String,

    /// Specify an endpoint providing Beacon API
    #[arg(long, env = "BEACON_ENDPOINT")]
    beacon_endpoint: String,

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

    /// Address of the ethereum endpoint
    #[arg(
        long = "ethereum-endpoint",
        default_value = DEFAULT_ETH_RPC,
        env = "ETH_RPC"
    )]
    eth_endpoint: String,

    /// Specify the hash of the ERC20-transaction to relay
    #[arg(long, env = "TX_HASH")]
    tx_hash: String,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    pretty_env_logger::formatted_timed_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .filter(Some("prover"), log::LevelFilter::Info)
        .filter(Some("relayer"), log::LevelFilter::Info)
        .filter(Some("ethereum-client"), log::LevelFilter::Info)
        .filter(Some("metrics"), log::LevelFilter::Info)
        .format_timestamp_secs()
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

            let authority_set_hash = hex::decode(&args.genesis_config_args.authority_set_hash)
                .expect("Incorrect format for authority set hash: hex-encoded hash is expected");
            let authority_set_hash = authority_set_hash
                .try_into()
                .expect("Incorrect format for authority set hash: wrong length");

            let genesis_config = GenesisConfig {
                authority_set_id: args.genesis_config_args.authority_set_id,
                authority_set_hash,
            };

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
                let bridging_payment_address = if &bridging_payment_address[..2] == "0x" {
                    &bridging_payment_address[2..]
                } else {
                    &bridging_payment_address
                };

                let bridging_payment_address: [u8; 32] = hex::decode(bridging_payment_address)
                    .expect("Wrong format of bridging-payment-address")
                    .try_into()
                    .expect("Wrong format of bridging-payment-address");

                let bridging_payment_address = bridging_payment_address.into();

                let relayer = paid_token_transfers::Relayer::new(
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
                let relayer = all_token_transfers::Relayer::new(gear_api, eth_api, args.from_block)
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
        CliCommands::RelayErc20(args) => erc20::relay(args).await,
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
