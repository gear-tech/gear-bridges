extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};
use std::{path::PathBuf, time::Instant};

use circom_verifier::CircomVerifierFilePaths;
use gear_rpc_client::GearApi;
use prover::{
    common::targets::TargetSet, message_sent::MessageSent, next_validator_set::NextValidatorSet,
    ProofWithCircuitData,
};

const DEFAULT_VARA_RPC: &str = "wss://testnet-archive.vara-network.io:443";
const DEFAULT_ETH_RPC: &str = "http://127.0.0.1:8545";

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
    #[command(subcommand)]
    #[clap(visible_alias("p"))]
    Prove(ProveCommands),
    /// Fetch data from RPC
    #[command(subcommand)]
    #[clap(visible_alias("q"))]
    Query(QueryCommands),
}

#[derive(Subcommand)]
enum ProveCommands {
    /// Prove validator set change
    #[clap(visible_alias("v"))]
    ValidatorSetChange(ProveArgs),
    /// Prove that message was sent
    #[clap(visible_alias("m"))]
    MessageSent(ProveArgs),
}

#[derive(Args)]
struct ProveArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    /// Path to the generated circom file containing constants
    #[arg(
        long = "circom-const-path",
        default_value = "./circom-verifier/circom/circuits/constants.circom"
    )]
    circom_constants_path: PathBuf,
    /// Path to the generated circom file containing gates
    #[arg(
        long = "circom-gates-path",
        default_value = "./circom-verifier/circom/circuits/gates.circom"
    )]
    circom_gates_path: PathBuf,
    /// Path to the generated proof
    #[arg(
        long = "proof-path",
        default_value = "./circom-verifier/plonky2_proof.json"
    )]
    proof_path: PathBuf,
    /// Path to the generated circuit config
    #[arg(
        long = "config-path",
        default_value = "./circom-verifier/circom/test/data/conf.json"
    )]
    config_path: PathBuf,
}

#[derive(Subcommand)]
enum QueryCommands {
    /// Query validator set stats
    #[clap(visible_alias("v"))]
    ValidatorSet {
        #[clap(flatten)]
        vara_endpoint: VaraEndpointArg,
        #[clap(flatten)]
        eth_endpoint: EthEndpointArg,
    },
    /// Query relayed message stats
    #[clap(visible_alias("m"))]
    Messages {
        #[clap(flatten)]
        eth_endpoint: EthEndpointArg,
    },
    /// Query all possible stats
    #[clap(visible_alias("a"))]
    All {
        #[clap(flatten)]
        vara_endpoint: VaraEndpointArg,
        #[clap(flatten)]
        eth_endpoint: EthEndpointArg,
    },
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
struct EthEndpointArg {
    /// Address of the Ethereum RPC endpoint
    #[arg(
        long = "eth-endpoint",
        default_value = DEFAULT_ETH_RPC
    )]
    eth_endpoint: String,
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::Prove(prove_command) => match prove_command {
            ProveCommands::ValidatorSetChange(args) => {
                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;
                let block = api.latest_finalized_block().await;

                let (block, current_epoch_block_finality) = api.fetch_finality_proof(block).await;
                let circuit = NextValidatorSet {
                    current_epoch_block_finality,
                    next_validator_set_inclusion_proof: api
                        .fetch_next_session_keys_merkle_proof(block)
                        .await,
                };

                process_prove(&circuit, NextValidatorSet::prove, &args);
            }
            ProveCommands::MessageSent(args) => {
                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;
                let block = api.latest_finalized_block().await;

                let (block, block_finality) = api.fetch_finality_proof(block).await;
                let circuit = MessageSent {
                    block_finality,
                    inclusion_proof: api.fetch_sent_message_merkle_proof(block).await,
                };

                process_prove(&circuit, MessageSent::prove, &args);
            }
        },
        CliCommands::Query(query_command) => match query_command {
            QueryCommands::Vara { vara_endpoint } => {
                todo!()
            }
            QueryCommands::Ethereum { eth_endpoint } => {
                todo!()
            }
        },
    };
}

fn process_prove<C, P, TS>(circuit: &C, prove: P, args: &ProveArgs)
where
    TS: TargetSet,
    P: Fn(&C) -> ProofWithCircuitData<TS>,
{
    let now = Instant::now();
    let proof = prove(circuit);
    log::info!("Proven in {}ms", now.elapsed().as_millis());

    let _ = proof.verify();

    proof.generate_circom_verifier(CircomVerifierFilePaths {
        constants: args.circom_constants_path.clone(),
        gates: args.circom_gates_path.clone(),
        proof: args.proof_path.clone(),
        config: args.config_path.clone(),
    });
}
