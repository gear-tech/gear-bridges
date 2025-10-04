use gear_rpc_client::{dto, GearApi};
use num::BigUint;
use primitive_types::H256;
use prometheus::{core::Collector, HistogramOpts, HistogramVec};
use prover::proving::{
    self, BlockFinality, BranchNodeData, GenesisConfig, PreCommit, ProofWithCircuitData,
    StorageInclusion,
};
use serde::{Deserialize, Serialize};
use std::{str::FromStr, thread, time::Instant};
use utils_prometheus::MeteredService;

pub struct Metrics;

impl MeteredService for Metrics {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        let proving_time: Box<dyn Collector> = Box::from(PROVING_TIME.clone());
        [proving_time]
    }
}

lazy_static::lazy_static!(
    static ref PROVING_TIME: HistogramVec = HistogramVec::new(
            HistogramOpts::new("proving_time", "ZK circuits proving time"),
            &["circuit"],
        ).unwrap();
);

pub async fn prove_genesis(
    gear_api: &GearApi,
    genesis_config: GenesisConfig,

    count_thread: Option<usize>,
) -> anyhow::Result<ProofWithCircuitData> {
    log::info!(
        "Proving genesis authority set change {} -> {}",
        genesis_config.authority_set_id,
        genesis_config.authority_set_id + 1
    );

    let (block, current_epoch_block_finality) = gear_api
        .fetch_finality_proof_for_session(genesis_config.authority_set_id)
        .await?;

    let next_validator_set_inclusion_proof = gear_api
        .fetch_next_session_keys_inclusion_proof(block)
        .await?;
    let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
    let next_validator_set_inclusion_proof =
        parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

    let now = Instant::now();
    let timer = PROVING_TIME.with_label_values(&["genesis"]).start_timer();

    let handler = thread::spawn(move || {
        proving::prove_genesis(
            parse_rpc_block_finality_proof(current_epoch_block_finality, count_thread),
            genesis_config,
            next_validator_set_inclusion_proof,
            next_validator_set_storage_data,
        )
    });

    let proof = handler
        .join()
        .expect("prover::proving::prove_genesis handle should be joined");

    timer.stop_and_record();
    log::info!("Genesis prove time: {}ms", now.elapsed().as_millis());

    Ok(proof)
}

pub async fn prove_validator_set_change(
    gear_api: &GearApi,
    previous_proof: ProofWithCircuitData,
    previous_authority_set_id: u64,

    count_thread: Option<usize>,
) -> anyhow::Result<ProofWithCircuitData> {
    log::info!(
        "Proving authority set change {} -> {}",
        previous_authority_set_id,
        previous_authority_set_id + 1
    );

    let (block, current_epoch_block_finality) = gear_api
        .fetch_finality_proof_for_session(previous_authority_set_id)
        .await?;

    let next_validator_set_inclusion_proof = gear_api
        .fetch_next_session_keys_inclusion_proof(block)
        .await?;
    let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
    let next_validator_set_inclusion_proof =
        parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

    let now = Instant::now();
    let timer = PROVING_TIME
        .with_label_values(&["validator_set_change"])
        .start_timer();

    let handler = thread::spawn(move || {
        proving::prove_validator_set_change(
            previous_proof,
            parse_rpc_block_finality_proof(current_epoch_block_finality, count_thread),
            next_validator_set_inclusion_proof,
            next_validator_set_storage_data,
        )
    });

    let proof = handler
        .join()
        .expect("proving::prove_validator_set_change handle should be joined");

    timer.stop_and_record();
    log::info!("Recursive prove time: {}ms", now.elapsed().as_millis());

    Ok(proof)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalProof {
    pub proof: Vec<u8>,
    pub block_number: u32,
    pub merkle_root: [u8; 32],
}

impl FinalProof {
    pub fn from_proof_and_public_inputs(proof: String, public_inputs: [BigUint; 2]) -> Self {
        // data layout:
        // root[0] root[1] root[2] root[3] root[4] root[5]
        // root[6] root[7] block_n   pad     pad     pad

        fn pad_pi(pi: &BigUint) -> Vec<u8> {
            const TARGET_LEN: usize = 24;

            let mut pi = pi.to_bytes_be();
            assert!(pi.len() <= TARGET_LEN);
            let mut padding = vec![0; TARGET_LEN - pi.len()];
            padding.append(&mut pi);
            padding
        }

        let pi_0 = pad_pi(&public_inputs[0]);
        let pi_1 = pad_pi(&public_inputs[1]);

        let first_root_part = &pi_0[..];
        let second_root_part = &pi_1[..8];

        let root_bytes = [first_root_part, second_root_part].concat();
        let block_number = &pi_1[8..12];

        assert_eq!(&proof[..2], "0x");

        Self {
            proof: hex::decode(&proof[2..]).expect("Got invalid proof string from gnark prover"),
            block_number: u32::from_be_bytes(
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
    genesis_config: GenesisConfig,
    at_block: H256,

    count_thread: Option<usize>,
    finality: Option<(H256, dto::BlockFinalityProof)>,
) -> anyhow::Result<FinalProof> {
    let (block, block_finality) = match finality {
        Some(finality) => finality,
        None => gear_api.fetch_finality_proof(at_block).await?,
    };
    prove_final_with_block_finality(
        gear_api,
        previous_proof,
        genesis_config,
        (block, block_finality),
        count_thread,
    )
    .await
}

pub async fn prove_final_with_block_finality(
    gear_api: &GearApi,
    previous_proof: ProofWithCircuitData,
    genesis_config: GenesisConfig,
    (block, block_finality): (H256, dto::BlockFinalityProof),

    count_thread: Option<usize>,
) -> anyhow::Result<FinalProof> {
    let sent_message_inclusion_proof = gear_api.fetch_sent_message_inclusion_proof(block).await?;

    let message_contents = sent_message_inclusion_proof.stored_data.clone();
    let sent_message_inclusion_proof = parse_rpc_inclusion_proof(sent_message_inclusion_proof);

    let now = Instant::now();
    let timer = PROVING_TIME.with_label_values(&["final"]).start_timer();

    let handler = thread::spawn(move || {
        let proof = proving::prove_message_sent(
            previous_proof,
            parse_rpc_block_finality_proof(block_finality, count_thread),
            genesis_config,
            sent_message_inclusion_proof,
            message_contents,
        );

        gnark::prove_circuit(&proof)
    });

    let proof = handler
        .join()
        .expect("proving::prove_message_sent & gnark handle should be joined");

    timer.stop_and_record();
    log::info!("Final prove time: {}ms", now.elapsed().as_millis());

    let public_inputs: [_; 2] = proof
        .public_inputs
        .try_into()
        .expect("Got wrong public input count from gnark prover");

    let public_inputs = public_inputs
        .map(|s| BigUint::from_str(&s).expect("Got wrong public input format from ganrk"));

    Ok(FinalProof::from_proof_and_public_inputs(
        proof.proof,
        public_inputs,
    ))
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

fn parse_rpc_block_finality_proof(
    proof: dto::BlockFinalityProof,
    count_thread: Option<usize>,
) -> BlockFinality {
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
        count_thread,
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
        fn prove(circuit_data: *const c_char) -> *const c_char;
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
