extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};

use pretty_env_logger::env_logger::fmt::TimestampPrecision;

use gear_rpc_client::GearApi;
use proof_storage::{FileSystemProofStorage, ProofStorage};
use prover::proving::GenesisConfig;

mod proof_storage;
mod prover_interface;
mod serve;

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
    /// Generate zk-proofs
    #[clap(visible_alias("p"))]
    #[command(subcommand)]
    Prove(ProveCommands),
    /// Start service constantly relaying messages to ethereum
    #[clap(visible_alias("s"))]
    Serve(ServeArgs),
    /// Relay message to ethereum
    #[clap(visible_alias("r"))]
    Relay(RelayArgs),
}

#[derive(Subcommand)]
enum ProveCommands {
    /// Generate genesis proof
    #[clap(visible_alias("g"))]
    Genesis {
        #[clap(flatten)]
        args: ProveArgs,
    },
    /// Prove that validator set has changed
    #[clap(visible_alias("v"))]
    ValidatorSetChange {
        #[clap(flatten)]
        args: ProveArgs,
    },
    /// Generate final proof
    #[clap(visible_alias("w"))]
    Wrapped {
        #[clap(flatten)]
        args: ProveArgs,
    },
}

#[derive(Args)]
struct RelayArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
}

#[derive(Args)]
struct ServeArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    #[clap(flatten)]
    ethereum_args: EthereumArgs,
}

#[derive(Args)]
struct ProveArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
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
        CliCommands::Prove(prove_command) => match prove_command {
            ProveCommands::Genesis { args } => {
                let mut proof_storage = FileSystemProofStorage::new("./proof_storage".into());

                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();

                let proof = prover_interface::prove_genesis(&gear_api).await;
                proof_storage
                    .init(proof, GENESIS_CONFIG.authority_set_id)
                    .unwrap();
            }
            ProveCommands::ValidatorSetChange { args } => {
                let mut proof_storage = FileSystemProofStorage::new("./proof_storage".into());

                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();

                let (previous_proof, previous_validator_set_id) =
                    proof_storage.get_latest_proof().unwrap();
                let proof = prover_interface::prove_validator_set_change(
                    &gear_api,
                    previous_proof,
                    previous_validator_set_id,
                )
                .await;
                proof_storage.update(proof.proof).unwrap();
            }
            ProveCommands::Wrapped { args } => {
                let proof_storage = FileSystemProofStorage::new("./proof_storage".into());

                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();

                let (previous_proof, previous_validator_set_id) =
                    proof_storage.get_latest_proof().unwrap();
                let _proof = prover_interface::prove_final(
                    &gear_api,
                    previous_proof,
                    previous_validator_set_id,
                )
                .await;
            }
        },
        CliCommands::Serve(args) => {
            let _ = serve::serve(args).await;
        }
        CliCommands::Relay(args) => {
            let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                .await
                .unwrap();

            // sender: ALICE
            // receiver: 0x000...00011
            // paylod: 0x11
            // nonce: 1
            // 0xbcf2aa76c36358f3913a1d701a2e9f9622d214348613f0059139b93e58edc6c2

            let block = gear_api.block_number_to_hash(715).await.unwrap();

            let message =
                hex::decode("bcf2aa76c36358f3913a1d701a2e9f9622d214348613f0059139b93e58edc6c2")
                    .unwrap();
            let message: [u8; 32] = message.try_into().unwrap();

            let message_hash = primitive_types::H256::from(message);

            gear_api
                .fetch_message_inclusion_merkle_proof(block, message_hash)
                .await
                .unwrap();
        }
    };
}
