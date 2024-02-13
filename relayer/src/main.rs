extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};
use eth_client::ContractVerifiers;
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
    #[clap(visible_alias("p"))]
    #[command(subcommand)]
    Prove(ProveCommands),
    /// Fetch stats from RPC
    #[clap(visible_alias("q"))]
    Query(QueryArgs),
    /// Send zk-proof to Ethereum
    #[clap(visible_alias("s"))]
    #[command(subcommand)]
    SendProof(SendProofCommands),
}

#[derive(Subcommand)]
enum ProveCommands {
    /// Prove validator set change
    #[clap(visible_alias("v"))]
    ValidatorSetChange {
        #[clap(flatten)]
        args: ProveArgs,
        #[arg(long = "validator-set-id", short = 'v')]
        validator_set_id: u64,
    },
    /// Prove that message was sent
    #[clap(visible_alias("m"))]
    MessageSent {
        #[clap(flatten)]
        args: ProveArgs,
        #[arg(long = "validator-set-id", short = 'v')]
        validator_set_id: Option<u64>,
    },
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

#[derive(Args)]
struct QueryArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    #[clap(flatten)]
    eth_endpoint: EthEndpointArg,
    #[clap(flatten)]
    contract_addresses: ContractAddressesArgs,
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

#[derive(Subcommand)]
enum SendProofCommands {
    #[clap(visible_alias("v"))]
    ValidatorSetChange(SendProofArgs),
    #[clap(visible_alias("m"))]
    MessageSent(SendProofArgs),
}

#[derive(Args)]
struct SendProofArgs {
    #[arg(long = "proof-path", default_value = "./final_proof.json")]
    proof_path: PathBuf,
    #[arg(long = "pi-path", default_value = "./final_public.json")]
    public_inputs_path: PathBuf,
    #[arg(long, env = "ETHEREUM_PRIVATE_KEY")]
    ethereum_private_key: String,
    #[clap(flatten)]
    eth_endpoint: EthEndpointArg,
    #[clap(flatten)]
    contract_addresses: ContractAddressesArgs,
}

#[derive(Args)]
struct ContractAddressesArgs {
    #[arg(long, env = "MS_CONTRACT")]
    message_sent_contract: String,
    #[arg(long, env = "VS_CONTRACT")]
    validator_set_change_contract: String,
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::Prove(prove_command) => match prove_command {
            ProveCommands::ValidatorSetChange {
                args,
                validator_set_id,
            } => {
                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;
                let (block, current_epoch_block_finality) =
                    api.fetch_finality_proof_for_session(validator_set_id).await;
                let circuit = NextValidatorSet {
                    current_epoch_block_finality,
                    next_validator_set_inclusion_proof: api
                        .fetch_next_session_keys_merkle_proof(block)
                        .await,
                };

                process_prove(&circuit, NextValidatorSet::prove, &args);
            }
            ProveCommands::MessageSent {
                args,
                validator_set_id,
            } => {
                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;

                let block = if let Some(validator_set_id) = validator_set_id {
                    api.search_for_validator_set_block(validator_set_id).await
                } else {
                    api.latest_finalized_block().await
                };

                let (block, block_finality) = api.fetch_finality_proof(block).await;
                let circuit = MessageSent {
                    block_finality,
                    inclusion_proof: api.fetch_sent_message_merkle_proof(block).await,
                };

                process_prove(&circuit, MessageSent::prove, &args);
            }
        },
        CliCommands::Query(args) => {
            let eth_client = ContractVerifiers::new(
                &args.eth_endpoint.eth_endpoint,
                &args.contract_addresses.validator_set_change_contract,
                &args.contract_addresses.message_sent_contract,
            )
            .unwrap();

            let vara_client = GearApi::new(&args.vara_endpoint.vara_endpoint).await;

            let messages = eth_client
                .get_all_msg_hashes_from_msg_sent_vrf()
                .await
                .unwrap();
            println!("-----Messages-----");
            for msg in messages {
                println!("{:?}", msg);
            }

            let validator_sets = eth_client
                .get_all_validator_sets_from_vs_vrf()
                .await
                .unwrap();
            println!("-----Validator sets-----");
            for vs in validator_sets {
                println!("{:?}", vs);
            }

            let last_vs_id = eth_client.get_nonce_id_from_vs_vrf().await.unwrap();
            println!("-----Last relayed validator set ID-----");
            println!("{}", last_vs_id);

            let latest_block = vara_client.latest_finalized_block().await;
            let last_vs_id = vara_client.validator_set_id(latest_block).await;
            println!("-----Last validator set ID on VARA-----");
            println!("{}", last_vs_id);
        }
        CliCommands::SendProof(command) => match command {
            SendProofCommands::ValidatorSetChange(args) => {
                let eth_client = ContractVerifiers::new(
                    &args.eth_endpoint.eth_endpoint,
                    &args.contract_addresses.validator_set_change_contract,
                    &args.contract_addresses.message_sent_contract,
                )
                .unwrap();

                let verified = eth_client
                    .verify_vs_change(
                        args.ethereum_private_key,
                        args.proof_path.to_str().unwrap(),
                        args.public_inputs_path.to_str().unwrap(),
                    )
                    .await
                    .unwrap();

                println!("Verified with result: {}", verified);
            }
            SendProofCommands::MessageSent(args) => {
                let eth_client = ContractVerifiers::new(
                    &args.eth_endpoint.eth_endpoint,
                    &args.contract_addresses.validator_set_change_contract,
                    &args.contract_addresses.message_sent_contract,
                )
                .unwrap();

                let verified = eth_client
                    .verify_msg_sent(
                        args.ethereum_private_key,
                        args.proof_path.to_str().unwrap(),
                        args.public_inputs_path.to_str().unwrap(),
                    )
                    .await
                    .unwrap();

                println!("Verified with result: {}", verified);
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
