use checkpoint_light_client_client::{traits::ServiceState, Order};
use ethereum_client::PollingEthApi;
use gclient::GearApi;
use primitive_types::H256;
use sails_rs::{calls::Query, gclient::calls::GClientRemoting};
use std::{path::Path, sync::Arc};

use super::common::{ethereum::block_storage::JSONBlockStorage, EthereumSlotNumber};
use storage::{InboundRuntimeIdentity, JSONStorage};

pub mod all_token_transfers;
pub mod cutover;
pub mod manual;
pub mod paid_token_transfers;

pub mod message_sender;
pub mod proof_composer;
pub mod storage;
pub mod tx_manager;

pub async fn get_latest_checkpoint(
    checkpoint_light_client_address: H256,
    gear_api: GearApi,
) -> Option<EthereumSlotNumber> {
    let remoting = GClientRemoting::new(gear_api);
    checkpoint_light_client_client::ServiceState::new(remoting)
        .get(Order::Reverse, 0, 1)
        .recv(checkpoint_light_client_address.0.into())
        .await
        .ok()
        .map(|state| {
            state
                .checkpoints
                .last()
                .map(|(checkpoint, _)| EthereumSlotNumber(*checkpoint))
        })
        .unwrap_or(None)
}

#[allow(clippy::too_many_arguments)]
async fn bind_runtime(
    identity: InboundRuntimeIdentity,
    storage: &JSONStorage,
    storage_path: &str,
    discovery_path: String,
    journal_ownership: &Arc<cutover::JournalOwnership>,
    eth_api: &PollingEthApi,
    client: &GearApi,
    cutover_seal: Option<&Path>,
    approved_manager_code: Option<H256>,
) -> anyhow::Result<(Arc<JSONBlockStorage>, std::fs::File, u64, Option<u64>)> {
    let signer_ownership =
        gear_common::signer_ownership::lock_signer(identity.gear_genesis_hash, identity.gear_sender)?;
    eth_api
        .enable_finality_archive(
            &Path::new(storage_path).join("ethereum-finality"),
            alloy::primitives::B256::from(identity.ethereum_genesis_hash.0),
        )
        .await?;
    let predecessor = cutover::authenticate_startup(
        &identity,
        cutover_seal,
        approved_manager_code,
        client,
        eth_api,
    )
    .await?;
    log::info!(
        "Authenticated inbound runtime: {}; predecessor_cutover: {}",
        serde_json::to_string(&identity)?,
        serde_json::to_string(&predecessor)?
    );
    let (minimum_receipt_slot, last_receipt_slot) =
        cutover::receipt_domain(client, identity.vft_manager_address).await?;
    log::info!("Authenticated inbound receipt domain: minimum={minimum_receipt_slot}; last={last_receipt_slot:?}; independently approved successor CodeId={approved_manager_code:?}");
    let block_storage = Arc::new(
        JSONBlockStorage::new_bound(
            discovery_path.into(),
            identity.clone(),
            predecessor.clone(),
        )
        .await?
        .with_journal_ownership(journal_ownership.clone()),
    );
    if let Some(predecessor) = predecessor {
        storage.bind_successor_identity(identity, predecessor).await?;
    } else {
        storage.bind_runtime_identity(identity).await?;
    }
    Ok((
        block_storage,
        signer_ownership,
        minimum_receipt_slot,
        last_receipt_slot,
    ))
}
