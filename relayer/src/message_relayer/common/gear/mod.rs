pub mod block_listener;
pub mod block_storage;
pub mod checkpoints_extractor;
pub mod merkle_proof_fetcher;
pub mod message_data_extractor;
pub mod message_paid_event_extractor;
pub mod message_queued_event_extractor;

/// Read QueueId only at the original canonical finalized hash. Missing storage is
/// unknown, not queue zero: historical ValueQuery defaults need runtime approval.
pub async fn queue_id_at(
    api: &gear_rpc_client::GearApi,
    number: u32,
    hash: primitive_types::H256,
) -> anyhow::Result<u64> {
    let finalized = api.latest_finalized_block().await?;
    anyhow::ensure!(
        api.block_hash_to_number(finalized).await? >= number
            && api.block_number_to_hash(number).await? == hash,
        "HOLD: queue evidence is not at the original finalized source hash"
    );
    let address = gsdk::gear::storage().gear_eth_bridge().queue_id();
    api.api
        .storage_fetch_at(&address, Some(hash))
        .await
        .map_err(|error| anyhow::anyhow!("HOLD: QueueId unavailable at {hash}: {error}"))
}
