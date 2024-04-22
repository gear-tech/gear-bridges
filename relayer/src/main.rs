extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};

use pretty_env_logger::env_logger::fmt::TimestampPrecision;

use gear_rpc_client::GearApi;

mod proof_storage;
mod prover_interface;

const DEFAULT_VARA_RPC: &str = "wss://testnet-archive.vara-network.io:443";
const DEFAULT_SERVE_ENDPOINT: &str = "localhost:1723";

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
        .filter_level(log::LevelFilter::Info)
        .format_target(false)
        .format_timestamp(Some(TimestampPrecision::Seconds))
        .init();

    let cli = Cli::parse();

    //let proof_storage = Box::new(FileSystemProofStorage::new("./proof_storage".into()));

    match cli.command {
        CliCommands::Prove(prove_command) => match prove_command {
            ProveCommands::Genesis { args } => {
                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();
                // ProverInterface::new(proof_storage, gear_api)
                //     .prove_genesis()
                //     .await;
            }
            ProveCommands::ValidatorSetChange { args } => {
                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();
                // ProverInterface::new(proof_storage, gear_api)
                //     .prove_validator_set_change()
                //     .await;
            }
            ProveCommands::Wrapped { args } => {
                let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
                    .await
                    .unwrap();
                // ProverInterface::new(proof_storage, gear_api)
                //     .prove_final()
                //     .await;
            }
        },
        CliCommands::Serve(ServeArgs {
            endpoint,
            genesis_block,
            prove_args: ProveArgs { vara_endpoint },
        }) => {
            let gear_api = GearApi::new(&vara_endpoint.vara_endpoint).await.unwrap();
            //let mut prover = ProverInterface::new(proof_storage, gear_api);

            //prover.prove_genesis().await;
        }
    };
}
