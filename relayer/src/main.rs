extern crate pretty_env_logger;

use clap::{Args, Parser, Subcommand};

use intermediate_proof_storage::{PersistentMockProofStorage, ProofStorage};
use pretty_env_logger::env_logger::fmt::TimestampPrecision;
use std::time::Instant;

use gear_rpc_client::{BlockInclusionProof, GearApi};
use prover::{
    common::targets::ParsableTargetSet,
    final_proof::{message_sent::MessageSent, FinalProof},
    latest_validator_set::{
        next_validator_set::NextValidatorSet, LatestValidatorSet, LatestValidatorSetTarget,
    },
    prelude::GENESIS_AUTHORITY_SET_ID,
    storage_inclusion::{BranchNodeData, StorageInclusion},
};

const DEFAULT_VARA_RPC: &str = "wss://testnet-archive.vara-network.io:443";

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

// TODO: Move to prover.
pub const NEXT_SESSION_KEYS_STORAGE_ADDRESS: [u8; 64] = [
    0xc, 0xe, 0xc, 0x5, 0x0, 0x7, 0x0, 0xd, 0x6, 0x0, 0x9, 0xd, 0xd, 0x3, 0x4, 0x9, 0x7, 0xf, 0x7,
    0x2, 0xb, 0xd, 0xe, 0x0, 0x7, 0xf, 0xc, 0x9, 0x6, 0xb, 0xa, 0x0, 0xe, 0x0, 0xc, 0xd, 0xd, 0x0,
    0x6, 0x2, 0xe, 0x6, 0xe, 0xa, 0xf, 0x2, 0x4, 0x2, 0x9, 0x5, 0xa, 0xd, 0x4, 0xc, 0xc, 0xf, 0xc,
    0x4, 0x1, 0xd, 0x4, 0x6, 0x0, 0x9,
];
pub const MESSAGE_STORAGE_ADDRESS: [u8; 64] = [
    0x1, 0xc, 0xb, 0x6, 0xf, 0x3, 0x6, 0xe, 0x0, 0x2, 0x7, 0xa, 0xb, 0xb, 0x2, 0x0, 0x9, 0x1, 0xc,
    0xf, 0xb, 0x5, 0x1, 0x1, 0x0, 0xa, 0xb, 0x5, 0x0, 0x8, 0x7, 0xf, 0x7, 0xa, 0x4, 0x1, 0x4, 0xc,
    0xb, 0x0, 0x0, 0x8, 0xe, 0x0, 0xe, 0x6, 0x1, 0xe, 0x4, 0x6, 0x7, 0x2, 0x2, 0xa, 0xa, 0x6, 0x0,
    0xa, 0xb, 0xd, 0xd, 0x6, 0x7, 0x2,
];

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Info)
        .format_target(false)
        .format_timestamp(Some(TimestampPrecision::Seconds))
        .init();

    let cli = Cli::parse();

    let mut proof_storage = PersistentMockProofStorage::new("./proof_storage".into());

    match cli.command {
        CliCommands::Prove(prove_command) => match prove_command {
            ProveCommands::Genesis { args } => {
                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;
                let (block, current_epoch_block_finality) = api
                    .fetch_finality_proof_for_session(GENESIS_AUTHORITY_SET_ID)
                    .await;

                let next_validator_set_inclusion_proof =
                    api.fetch_next_session_keys_inclusion_proof(block).await;
                let next_validator_set_storage_data = next_validator_set_inclusion_proof
                    .storage_inclusion_proof
                    .storage_data
                    .clone();
                let next_validator_set_inclusion_proof = parse_rpc_inclusion_proof(
                    next_validator_set_inclusion_proof,
                    NEXT_SESSION_KEYS_STORAGE_ADDRESS.to_vec(),
                );

                let now = Instant::now();

                let change_from_genesis = NextValidatorSet {
                    current_epoch_block_finality,
                    next_validator_set_inclusion_proof,
                    next_validator_set_storage_data,
                };

                let genesis_proof = LatestValidatorSet {
                    change_proof: change_from_genesis,
                }
                .prove_genesis();

                log::info!("Genesis prove time: {}ms", now.elapsed().as_millis());

                proof_storage
                    .init(genesis_proof.circuit_data().clone(), genesis_proof.proof())
                    .unwrap();
            }
            ProveCommands::ValidatorSetChange { args } => {
                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;

                let latest_proof = proof_storage
                    .get_latest_proof()
                    .expect("No latest proof found");
                let latest_proof_public_inputs = LatestValidatorSetTarget::parse_public_inputs(
                    &mut latest_proof.public_inputs.clone().into_iter(),
                );

                let validator_set_id = latest_proof_public_inputs.current_set_id;

                let (block, current_epoch_block_finality) =
                    api.fetch_finality_proof_for_session(validator_set_id).await;

                let next_validator_set_inclusion_proof =
                    api.fetch_next_session_keys_inclusion_proof(block).await;
                let next_validator_set_storage_data = next_validator_set_inclusion_proof
                    .storage_inclusion_proof
                    .storage_data
                    .clone();
                let next_validator_set_inclusion_proof = parse_rpc_inclusion_proof(
                    next_validator_set_inclusion_proof,
                    NEXT_SESSION_KEYS_STORAGE_ADDRESS.to_vec(),
                );

                let now = Instant::now();

                let next_change = NextValidatorSet {
                    current_epoch_block_finality,
                    next_validator_set_inclusion_proof,
                    next_validator_set_storage_data,
                };

                let validator_set_change_proof = LatestValidatorSet {
                    change_proof: next_change,
                }
                .prove_recursive(latest_proof);

                log::info!("Recursive prove time: {}ms", now.elapsed().as_millis());

                proof_storage
                    .update(validator_set_change_proof.proof())
                    .unwrap();
            }
            ProveCommands::Wrapped { args } => {
                let api = GearApi::new(&args.vara_endpoint.vara_endpoint).await;

                let latest_proof = proof_storage
                    .get_latest_proof()
                    .expect("No latest proof found");
                let latest_proof_public_inputs = LatestValidatorSetTarget::parse_public_inputs(
                    &mut latest_proof.public_inputs.clone().into_iter(),
                );

                let block = api
                    .search_for_validator_set_block(latest_proof_public_inputs.current_set_id)
                    .await;
                let (block, block_finality) = api.fetch_finality_proof(block).await;

                let sent_message_inclusion_proof =
                    api.fetch_sent_message_inclusion_proof(block).await;

                let sent_message_storage_data = sent_message_inclusion_proof
                    .storage_inclusion_proof
                    .storage_data
                    .clone();
                let sent_message_inclusion_proof = parse_rpc_inclusion_proof(
                    sent_message_inclusion_proof,
                    MESSAGE_STORAGE_ADDRESS.to_vec(),
                );

                let message_sent = MessageSent {
                    block_finality,
                    inclusion_proof: sent_message_inclusion_proof,
                    message_storage_data: sent_message_storage_data,
                };

                let current_validator_set_verifier_data =
                    proof_storage.get_verifier_circuit_data().unwrap();
                let final_proof = FinalProof {
                    message_sent,
                    current_validator_set_proof: latest_proof,
                    current_validator_set_verifier_data,
                }
                .prove();

                let final_serialized = final_proof.export_wrapped();

                gnark::compile_circuit(&final_serialized);

                let proof = gnark::prove_circuit(&final_serialized);
                dbg!(proof);
            }
        },
    };
}

fn parse_rpc_inclusion_proof(
    proof: BlockInclusionProof,
    address_nibbles: Vec<u8>,
) -> StorageInclusion {
    StorageInclusion {
        block_header_data: proof.encoded_header,
        branch_node_data: proof
            .storage_inclusion_proof
            .branch_nodes_data
            .into_iter()
            .rev()
            .map(|d| BranchNodeData {
                data: d.encoded_node,
                child_nibble: d.child_nibble,
            })
            .collect(),
        leaf_node_data: proof.storage_inclusion_proof.encoded_leaf_node,
        address_nibbles: address_nibbles,
    }
}

pub mod gnark {
    use core::ffi::c_char;
    use std::ffi::{CStr, CString};

    use prover::common::SerializedDataToVerify;

    extern "C" {
        fn compile(circuit_data: *const c_char);
        fn prove(circuit_data: *const c_char) -> *const c_char;
    }

    pub fn compile_circuit(s: &SerializedDataToVerify) {
        let serialized = serde_json::to_string(s).expect("Failed to serialize data");
        let c_string = CString::new(serialized).expect("CString::new failed");
        unsafe {
            compile(c_string.as_ptr());
        }
    }

    pub fn prove_circuit(s: &SerializedDataToVerify) -> String {
        let serialized = serde_json::to_string(s).expect("Failed to serialize data");
        let c_string = CString::new(serialized).expect("CString::new failed");
        let result = unsafe {
            let result_ptr = prove(c_string.as_ptr());
            if result_ptr.is_null() {
                panic!("prove returned null pointer");
            }
            // Convert the result pointer to a Rust string.
            let result_cstr = CStr::from_ptr(result_ptr);
            let result_str = result_cstr.to_str().expect("Invalid UTF-8 sequence");
            let owned = result_str.to_owned();
            libc::free(result_ptr as *mut libc::c_void);
            owned
        };
        result // todo decode
    }
}
