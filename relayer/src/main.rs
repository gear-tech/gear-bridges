use gear_rpc_client::GearApi;

#[tokio::main]
async fn main() {
    let api = GearApi::new().await;

    // epoch: 2_400 blocks
    // 2500th epoch ends at 6_000_000th block.

    const EPOCH_DURATION: u32 = 2_400;

    let genesis_epoch = 2500;
    let genesis_block_no = EPOCH_DURATION * genesis_epoch + 1;
    let genesis_block_hash = api.block_number_to_hash(genesis_block_no).await;
    let genesis_authorities = api.fetch_babe_authorities(genesis_block_hash).await;

    let message_sent_block_no = genesis_block_no + 8000;
    let message_sent_block_hash = api.block_number_to_hash(message_sent_block_no).await;
    let message_sent_from_epoch_start = (message_sent_block_no - 1) % EPOCH_DURATION;
    let message_sent_epoch =
        (message_sent_block_no - message_sent_from_epoch_start - 1) / EPOCH_DURATION;

    let mut last_known_authorities = genesis_authorities;
    let mut last_proven_epoch = genesis_epoch;

    while last_proven_epoch != message_sent_epoch {
        let last_proven_epoch_first_block = last_proven_epoch * EPOCH_DURATION + 1;
        let last_proven_epoch_first_block = api
            .block_number_to_hash(last_proven_epoch_first_block)
            .await;
        let finality = api
            .fetch_finality_proof(last_proven_epoch_first_block)
            .await;

        finality.prove(); // 1st proof.

        let next_authorities = api
            .fetch_next_authorities_merkle_proof(last_proven_epoch_first_block)
            .await;
        next_authorities.prove(); // 2nd proof.

        // TODO: decode
        // last_known_authorities = next_authorities.leaf_data;

        last_proven_epoch += 1;
    }

    // Here current validator set proof can be extracted.

    let message_block_finality = api.fetch_finality_proof(message_sent_block_hash).await;
    message_block_finality.prove(); // 3rd proof.

    // TODO: prove message inclusion into merkle trie.
}
