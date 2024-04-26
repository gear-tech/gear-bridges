extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};

use pretty_env_logger::env_logger::fmt::TimestampPrecision;

use gear_rpc_client::GearApi;
use proof_storage::{FileSystemProofStorage, ProofStorage};
use prover::proving::GenesisConfig;

mod proof_storage;
mod prover_interface;

const DEFAULT_VARA_RPC: &str = "ws://localhost:9944";
const DEFAULT_SERVE_ENDPOINT: &str = "localhost:1723";

const GENESIS_CONFIG: GenesisConfig = GenesisConfig {
    validator_set_id: 1,
    // 0xb9853ab2fb585702dfd9040ee8bc9f94dc5b0abd8b0f809ec23fdc0265b21e24
    validator_set_hash: [
        0xb23a85b9, 0x025758fb, 0x0e04d9df, 0x949fbce8, 0xbd0a5bdc, 0x9e800f8b, 0x02dc3fc2,
        0x241eb265,
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
    /// Start HTTP server on specified endpoint
    #[clap(visible_alias("s"))]
    Serve(ServeArgs),
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
struct ServeArgs {
    /// Endpoint to expose API
    #[arg(
        long = "endpoint",
        default_value = DEFAULT_SERVE_ENDPOINT
    )]
    endpoint: String,
    /// Genesis block for bridge
    #[arg(long = "genesis-block", short = 'g')]
    genesis_block: Option<u32>,
    #[clap(flatten)]
    prove_args: ProveArgs,
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

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .filter(Some("prover"), log::LevelFilter::Debug)
        .format_timestamp(Some(TimestampPrecision::Seconds))
        .init();

    let cli = Cli::parse();

    let mut proof_storage = FileSystemProofStorage::new("./proof_storage".into());

    match cli.command {
        CliCommands::Prove(prove_command) => match prove_command {
            ProveCommands::Genesis { args } => {
                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();

                let proof = prover_interface::prove_genesis(&gear_api).await;
                proof_storage
                    .init(proof, GENESIS_CONFIG.validator_set_id)
                    .unwrap();
            }
            ProveCommands::ValidatorSetChange { args } => {
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
                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();

                let (previous_proof, previous_validator_set_id) =
                    proof_storage.get_latest_proof().unwrap();
                let proof = prover_interface::prove_final(
                    &gear_api,
                    previous_proof,
                    previous_validator_set_id,
                )
                .await;

                println!("{}", proof);
            }
        },
        CliCommands::Serve(ServeArgs {
            endpoint: _,
            genesis_block: _,
            prove_args: ProveArgs { vara_endpoint },
        }) => {
            let _gear_api = GearApi::new(&vara_endpoint.vara_endpoint).await.unwrap();
            todo!()
        }
    };
}
