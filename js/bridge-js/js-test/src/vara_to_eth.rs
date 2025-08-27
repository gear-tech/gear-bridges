use alloy::{
    primitives::{Address, Bytes, FixedBytes, U256},
    sol,
    sol_types::SolCall,
};
use gear_rpc_client::{dto::Message, GearApi};
use keccak_hash::keccak_256;
use std::path::PathBuf;

sol!(
    #![sol(rpc, extra_derives(Debug))]
    IMessageQueue,
    "../../../api/ethereum/MessageQueue.json"
);

fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_le.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_256(&data, &mut hash);

    hash
}

pub async fn vara_to_eth(gear_api: GearApi, message_nonce: primitive_types::U256, gear_block: u32) {
    let gear_block_hash = gear_api
        .block_number_to_hash(gear_block)
        .await
        .expect("Failed to fetch block hash by number");

    let message_queued_events = gear_api
        .message_queued_events(gear_block_hash)
        .await
        .expect("Failed to fetch MessageQueued events from gear block");

    let message = message_queued_events
        .into_iter()
        .find(|m| primitive_types::U256::from_little_endian(&m.nonce_le) == message_nonce)
        .unwrap_or_else(|| {
            panic!("Message with nonce {message_nonce} is not found in gear block {gear_block}")
        });

    let msg_hash = message_hash(&message);

    let proof = gear_api
        .fetch_message_inclusion_merkle_proof(gear_block_hash, msg_hash.into())
        .await
        .expect("Unable to fetch message inclusion proof");

    let vara_message = IMessageQueue::VaraMessage {
        nonce: U256::from_be_bytes(message.nonce_le),
        source: FixedBytes::from_slice(&message.source),
        destination: Address::from_slice(&message.destination),
        payload: Bytes::from(message.payload.clone()),
    };

    let proof_hashes: Vec<FixedBytes<32>> = proof
        .proof
        .iter()
        .map(|hash| FixedBytes::from_slice(hash.as_ref()))
        .collect();

    let process_message_call = IMessageQueue::processMessageCall {
        blockNumber: U256::from(gear_block),
        totalLeaves: U256::from(proof.num_leaves),
        leafIndex: U256::from(proof.leaf_index),
        message: vara_message,
        proof: proof_hashes,
    }
    .abi_encode();

    let target_dir = PathBuf::from("test/tmp");
    std::fs::write(
        target_dir.join("process_message_calldata"),
        hex::encode(process_message_call),
    )
    .expect("Failed to write encoded call data");
    std::fs::write(
        target_dir.join("vara_to_eth_message_hash"),
        hex::encode(msg_hash),
    )
    .expect("Failed to write message_hash");

    std::fs::write(target_dir.join("vara_to_eth_root"), hex::encode(proof.root))
        .expect("Failed to write root");

    std::fs::write(
        target_dir.join("vara_to_eth_proof"),
        proof
            .proof
            .iter()
            .map(hex::encode)
            .collect::<Vec<_>>()
            .join(""),
    )
    .expect("Failed to write proof");

    std::fs::write(
        target_dir.join("vara_to_eth_num_leaves"),
        proof.num_leaves.to_string(),
    )
    .expect("Failed to write num_leaves");

    std::fs::write(
        target_dir.join("vara_to_eth_leaf_index"),
        proof.leaf_index.to_string(),
    )
    .expect("Failed to write leaf_index")
}
