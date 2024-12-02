use std::time::Duration;

use alloy::providers::Provider;
use alloy_eips::BlockNumberOrTag;
use anyhow::anyhow;
use ethereum_client::EthApi;

use ethereum_beacon_client::BeaconClient;

use super::{EthereumBlockNumber, EthereumSlotNumber};

pub mod block_listener;
pub mod deposit_event_extractor;
pub mod merkle_root_extractor;
pub mod message_paid_event_extractor;
pub mod message_sender;

async fn find_slot_by_block_number(
    eth_api: &EthApi,
    beacon_client: &BeaconClient,
    block: EthereumBlockNumber,
) -> anyhow::Result<EthereumSlotNumber> {
    let block_body = eth_api
        .raw_provider()
        .get_block_by_number(BlockNumberOrTag::Number(block.0), false)
        .await?
        .ok_or(anyhow!("Ethereum block #{} is missing", block.0))?;

    let beacon_root_parent = block_body.header.parent_beacon_block_root.ok_or(anyhow!(
        "Unable to determine root of parent beacon block for block #{}",
        block.0
    ))?;

    let beacon_block_parent = beacon_client
        .get_block_by_hash(&beacon_root_parent.0)
        .await?;

    // TODO: It's a temporary solution of a problem that we're connecting to a different
    // nodes, so if we're observing finalized block on one node, the finalized slot might still be not
    // available on other.
    for _ in 0..10 {
        let beacon_block_result = beacon_client
            .find_beacon_block(block.0, &beacon_block_parent)
            .await;

        match beacon_block_result {
            Ok(beacon_block) => {
                return Ok(EthereumSlotNumber(beacon_block.slot));
            }
            Err(err) => {
                log::warn!(
                    "Failed to find beacon block for ethereum block #{}: {}. Waiting for 2 seconds before next attempt...",
                    block.0,
                    err
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }

    anyhow::bail!(
        "Failed to find beacon block for Ethereum block #{} after 10 attempts",
        block.0
    );
}
