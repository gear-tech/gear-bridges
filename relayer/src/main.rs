extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};
use std::{path::PathBuf, time::Instant};

use circom_verifier::CircomVerifierFilePaths;
use gear_rpc_client::GearApi;
use prover::{
    common::targets::TargetSet, message_sent::MessageSent, next_validator_set::NextValidatorSet,
    ProofWithCircuitData,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: CliCommands,
    /// Address of the VARA RPC endpoint
    #[arg(
        long = "vara-endpoint",
        default_value = "wss://testnet-archive.vara-network.io:443"
    )]
    vara_endpoint: String,
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
enum CliCommands {
    /// Prove validator set change
    ValidatorSetChange(ProveValidatorSetChangeCommand),
    /// Prove that message was sent
    MessageSent(ProveMessageSentCommand),
}

#[derive(Args)]
struct ProveValidatorSetChangeCommand {}

#[derive(Args)]
struct ProveMessageSentCommand {}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let cli = Cli::parse();

    let api = GearApi::new(&cli.vara_endpoint).await;
    let block = api.latest_finalized_block().await;

    match &cli.command {
        CliCommands::ValidatorSetChange(_) => {
            let (block, current_epoch_block_finality) = api.fetch_finality_proof(block).await;
            let circuit = NextValidatorSet {
                current_epoch_block_finality,
                next_validator_set_inclusion_proof: api
                    .fetch_next_session_keys_merkle_proof(block)
                    .await,
            };

            process_command(&circuit, NextValidatorSet::prove, &cli);
        }
        CliCommands::MessageSent(_) => {
            let (block, block_finality) = api.fetch_finality_proof(block).await;
            let circuit = MessageSent {
                block_finality,
                inclusion_proof: api.fetch_sent_message_merkle_proof(block).await,
            };

            process_command(&circuit, MessageSent::prove, &cli);
        }
    };
}

fn process_command<C, P, TS>(circuit: &C, prove: P, cli: &Cli)
where
    TS: TargetSet,
    P: Fn(&C) -> ProofWithCircuitData<TS>,
{
    let now = Instant::now();
    let proof = prove(circuit);
    log::info!("Proven in {}ms", now.elapsed().as_millis());

    let _ = proof.verify();

    proof.generate_circom_verifier(CircomVerifierFilePaths {
        constants: cli.circom_constants_path.clone(),
        gates: cli.circom_gates_path.clone(),
        proof: cli.proof_path.clone(),
        config: cli.config_path.clone(),
    });
}
