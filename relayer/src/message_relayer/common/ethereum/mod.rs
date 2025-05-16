use std::{
    cmp::Ordering,
    time::Duration,
    vec::Drain,
};

use alloy::providers::Provider;
use alloy_eips::BlockNumberOrTag;
use anyhow::anyhow;
use ethereum_client::EthApi;

use ethereum_beacon_client::BeaconClient;
use ethereum_common::beacon::electra::Block;

use super::{EthereumBlockNumber, EthereumSlotNumber, RelayedMerkleRoot, MessageInBlock, AuthoritySetId, GearBlockNumber};

pub mod accumulator;
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
        .get_block_by_hash::<Block>(&beacon_root_parent.0)
        .await?;

    // TODO: It's a temporary solution of a problem that we're connecting to a different
    // nodes, so if we're observing finalized block on one node, the finalized slot might still be not
    // available on other.
    for _ in 0..30 {
        let beacon_block_result = beacon_client
            .find_beacon_block(block.0, beacon_block_parent.clone())
            .await;

        match beacon_block_result {
            Ok(beacon_block) => {
                return Ok(EthereumSlotNumber(beacon_block.slot));
            }

            Err(e) => {
                let delay = Duration::from_secs(15);
                log::warn!(
                    "Failed to find beacon block for ethereum block #{}: {e}. Waiting for {delay:?} before next attempt...",
                    block.0,
                );

                tokio::time::sleep(delay).await;
            }
        }
    }

    anyhow::bail!(
        "Failed to find beacon block for Ethereum block #{} after 10 attempts",
        block.0
    );
}
