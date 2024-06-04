use std::{str::FromStr, time::Instant};

use super::GENESIS_CONFIG;
use gear_rpc_client::{dto, GearApi};
use num::BigUint;
use prover::proving::{
    self, BlockFinality, BranchNodeData, PreCommit, ProofWithCircuitData, StorageInclusion,
};

pub async fn prove_genesis(gear_api: &GearApi) -> ProofWithCircuitData {
    let (block, current_epoch_block_finality) = gear_api
        .fetch_finality_proof_for_session(GENESIS_CONFIG.authority_set_id)
        .await
        .unwrap();

    let next_validator_set_inclusion_proof = gear_api
        .fetch_next_session_keys_inclusion_proof(block)
        .await
        .unwrap();
    let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
    let next_validator_set_inclusion_proof =
        parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

    let now = Instant::now();

    let proof = prover::proving::prove_genesis(
        parse_rpc_block_finality_proof(current_epoch_block_finality),
        GENESIS_CONFIG,
        next_validator_set_inclusion_proof,
        next_validator_set_storage_data,
    );

    log::info!("Genesis prove time: {}ms", now.elapsed().as_millis());

    proof
}

pub async fn prove_validator_set_change(
    gear_api: &GearApi,
    previous_proof: ProofWithCircuitData,
    previous_authority_set_id: u64,
) -> ProofWithCircuitData {
    log::info!(
        "Proving authority set change {} -> {}",
        previous_authority_set_id,
        previous_authority_set_id + 1
    );

    let (block, current_epoch_block_finality) = gear_api
        .fetch_finality_proof_for_session(previous_authority_set_id)
        .await
        .unwrap();

    let next_validator_set_inclusion_proof = gear_api
        .fetch_next_session_keys_inclusion_proof(block)
        .await
        .unwrap();
    let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
    let next_validator_set_inclusion_proof =
        parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

    let now = Instant::now();

    let proof = proving::prove_validator_set_change(
        previous_proof,
        parse_rpc_block_finality_proof(current_epoch_block_finality),
        next_validator_set_inclusion_proof,
        next_validator_set_storage_data,
    );

    log::info!("Recursive prove time: {}ms", now.elapsed().as_millis());

    proof
}

pub struct FinalProof {
    pub proof: Vec<u8>,
    pub block_number: u32,
    pub merkle_root: [u8; 32],
}

impl FinalProof {
    pub fn from_proof_and_public_inputs(proof: String, public_inputs: [BigUint; 2]) -> Self {
        // data layout:
        // pad pad root[0] root[1] root[2] root[3] root[4] root[5]
        // pad pad root[6] root[7] block_n   pad     pad     pad

        fn pad_pi(pi: &BigUint) -> Vec<u8> {
            const TARGET_LEN: usize = 256 / 8;

            let mut pi = pi.to_bytes_le();
            assert!(pi.len() <= TARGET_LEN);
            pi.append(&mut vec![0; TARGET_LEN - pi.len()]);
            pi
        }

        let pi_0 = pad_pi(&public_inputs[0]);
        let pi_1 = pad_pi(&public_inputs[0]);

        let first_root_part = &pi_0[pi_0.len() - 24..];
        let second_root_part = &pi_1[pi_1.len() - 24..pi_1.len() - 16];

        let root_bytes = [first_root_part, second_root_part].concat();
        let block_number = &pi_1[pi_1.len() - 16..pi_1.len() - 12];

        assert_eq!(&proof[..2], "0x");

        Self {
            proof: hex::decode(&proof[2..]).expect("Got invalid proof string from gnark prover"),
            block_number: u32::from_le_bytes(
                block_number
                    .try_into()
                    .expect("Wrong amount of bytes to build block number"),
            ),
            merkle_root: root_bytes
                .try_into()
                .expect("Wrong amount of bytes to build merkle tree root"),
        }
    }
}

pub async fn prove_final(
    gear_api: &GearApi,
    previous_proof: ProofWithCircuitData,
    previous_validator_set_id: u64,
) -> FinalProof {
    let block = gear_api
        .search_for_authority_set_block(previous_validator_set_id)
        .await
        .unwrap();
    let (block, block_finality) = gear_api.fetch_finality_proof(block).await.unwrap();

    let sent_message_inclusion_proof = gear_api
        .fetch_sent_message_inclusion_proof(block)
        .await
        .unwrap();

    let message_contents = sent_message_inclusion_proof.stored_data.clone();
    let sent_message_inclusion_proof = parse_rpc_inclusion_proof(sent_message_inclusion_proof);

    let proof = proving::prove_message_sent(
        previous_proof,
        parse_rpc_block_finality_proof(block_finality),
        GENESIS_CONFIG,
        sent_message_inclusion_proof,
        message_contents,
    );

    // TODO: Compile only when not initialized #54
    //gnark::compile_circuit(&proof);

    let proof = gnark::prove_circuit(&proof);

    let public_inputs: [_; 2] = proof
        .public_inputs
        .try_into()
        .expect("Got wrong public input count from gnark prover");

    let public_inputs = public_inputs
        .map(|s| BigUint::from_str(&s).expect("Got wrong public input format from ganrk"));

    FinalProof::from_proof_and_public_inputs(proof.proof, public_inputs)
}

fn parse_rpc_inclusion_proof(proof: dto::StorageInclusionProof) -> StorageInclusion {
    let address_nibbles = proof
        .address
        .into_iter()
        .flat_map(|byte| [(byte & 0b11110000) >> 4, byte & 0b00001111])
        .collect();

    StorageInclusion {
        block_header_data: proof.block_header,
        branch_node_data: proof
            .branch_nodes_data
            .into_iter()
            .rev()
            .map(|d| BranchNodeData {
                data: d.data,
                child_nibble: d.target_child,
            })
            .collect(),
        leaf_node_data: proof.leaf_node_data,
        address_nibbles,
    }
}

fn parse_rpc_block_finality_proof(proof: dto::BlockFinalityProof) -> BlockFinality {
    BlockFinality {
        validator_set: proof.validator_set,
        pre_commits: proof
            .pre_commits
            .into_iter()
            .map(|pc| PreCommit {
                public_key: pc.public_key,
                signature: pc.signature,
            })
            .collect(),
        message: proof
            .message
            .try_into()
            .expect("Unexpected GRANDPA message length"),
    }
}

pub mod gnark {
    use core::ffi::c_char;
    use std::ffi::{CStr, CString};

    use prover::proving::ExportedProofWithCircuitData;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize)]
    pub struct ProveResult {
        pub proof: String,
        pub public_inputs: Vec<String>,
    }

    extern "C" {
        fn compile(circuit_data: *const c_char);
        fn prove(circuit_data: *const c_char) -> *const c_char;
    }

    pub fn compile_circuit(s: &ExportedProofWithCircuitData) {
        let serialized = serde_json::to_string(s).expect("Failed to serialize data");
        let c_string = CString::new(serialized).expect("CString::new failed");
        unsafe {
            compile(c_string.as_ptr());
        }
    }

    pub fn prove_circuit(s: &ExportedProofWithCircuitData) -> ProveResult {
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

        serde_json::from_str(&result).expect("Got wrong output from gnark prover")
    }
}
