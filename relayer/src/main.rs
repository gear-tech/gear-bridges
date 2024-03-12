extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};
use eth_client::ContractVerifiers;
use std::{path::PathBuf, time::Instant};

use gear_rpc_client::GearApi;
use prover::{
    final_proof::FinalProof, latest_validator_set::LatestValidatorSet, message_sent::MessageSent,
    next_validator_set::NextValidatorSet,
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
    /// Generate test data
    #[clap(visible_alias("t"))]
    TestCase {
        #[clap(flatten)]
        args: ProveArgs,
    },
}

#[derive(Args)]
struct ProveArgs {
    #[clap(flatten)]
    vara_endpoint: VaraEndpointArg,
    /// Where to write proof with public inputs
    #[arg(
        long = "circom-const-path",
        default_value = "./gnark-wrapper/data/proof_with_public_inputs.json"
    )]
    proof_with_public_inputs_path: PathBuf,
    /// Where to write common circuit data
    #[arg(
        long = "circom-gates-path",
        default_value = "./gnark-wrapper/data/common_circuit_data.json"
    )]
    common_circuit_data_path: PathBuf,
    /// Where to write verifier only circuit data
    #[arg(
        long = "verifier-only-circuit-data",
        default_value = "./gnark-wrapper/data/verifier_only_circuit_data.json"
    )]
    verifier_only_circuit_data_path: PathBuf,
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

                let now = Instant::now();
                let proof = circuit.prove();
                log::info!("Proven in {}ms", now.elapsed().as_millis());
                let verified = proof.verify();
                log::info!("Verified with result {}", verified);
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

                let now = Instant::now();
                let proof = circuit.prove();
                log::info!("Proven in {}ms", now.elapsed().as_millis());
                let verified = proof.verify();
                log::info!("Verified with result {}", verified);
            }
            ProveCommands::TestCase { args } => {
                const GENESIS_VS_ID: u64 = 272;

                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;

                let (block, current_epoch_block_finality) =
                    api.fetch_finality_proof_for_session(GENESIS_VS_ID).await;
                let change_from_genesis = NextValidatorSet {
                    current_epoch_block_finality,
                    next_validator_set_inclusion_proof: api
                        .fetch_next_session_keys_merkle_proof(block)
                        .await,
                };

                let latest_vs = LatestValidatorSet {
                    change_proof: change_from_genesis,
                }
                .prove_genesis();

                let (block, current_epoch_block_finality) = api
                    .fetch_finality_proof_for_session(GENESIS_VS_ID + 1)
                    .await;
                let next_change = NextValidatorSet {
                    current_epoch_block_finality,
                    next_validator_set_inclusion_proof: api
                        .fetch_next_session_keys_merkle_proof(block)
                        .await,
                };

                let latest_vs = LatestValidatorSet {
                    change_proof: next_change,
                }
                .prove_recursive(latest_vs.proof());

                let final_serialized = latest_vs.export();
                std::fs::write("./pwpi_2.json", final_serialized.proof_with_public_inputs).unwrap();
                std::fs::write("ccd_2.json", final_serialized.common_circuit_data).unwrap();
                std::fs::write("vocd_2.json", final_serialized.verifier_only_circuit_data).unwrap();

                panic!("DONE");

                let (block, current_epoch_block_finality) = api
                    .fetch_finality_proof_for_session(GENESIS_VS_ID + 2)
                    .await;
                let next_change = NextValidatorSet {
                    current_epoch_block_finality,
                    next_validator_set_inclusion_proof: api
                        .fetch_next_session_keys_merkle_proof(block)
                        .await,
                };

                let latest_vs = LatestValidatorSet {
                    change_proof: next_change,
                }
                .prove_recursive(latest_vs.proof());

                let block = api.search_for_validator_set_block(GENESIS_VS_ID + 3).await;
                let (block, block_finality) = api.fetch_finality_proof(block).await;
                let message_sent = MessageSent {
                    block_finality,
                    inclusion_proof: api.fetch_sent_message_merkle_proof(block).await,
                };

                let final_proof = FinalProof {
                    current_validator_set: latest_vs,
                    message_sent,
                }
                .prove();

                let final_serialized = final_proof.export_wrapped();

                std::fs::write(
                    args.proof_with_public_inputs_path,
                    final_serialized.proof_with_public_inputs,
                )
                .unwrap();
                std::fs::write(
                    args.common_circuit_data_path,
                    final_serialized.common_circuit_data,
                )
                .unwrap();
                std::fs::write(
                    args.verifier_only_circuit_data_path,
                    final_serialized.verifier_only_circuit_data,
                )
                .unwrap();
            }
        },
        CliCommands::Query(args) => {}
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
