use crate::{ethereum_beacon_client, RelayErc20Args};

use alloy::{
    network::primitives::BlockTransactionsKind,
    primitives::TxHash,
    providers::{Provider, ProviderBuilder},
};
use alloy_eips::BlockNumberOrTag;
use alloy_rlp::Encodable;
use anyhow::{anyhow, Result as AnyResult};
use checkpoint_light_client_io::ethereum_common::{
    beacon::{light::Block as LightBeaconBlock, Block as BeaconBlock},
    utils::{self as eth_utils, MerkleProof},
    SLOTS_PER_EPOCH,
};
use erc20_relay_client::{
    traits::Erc20Relay as _, BlockInclusionProof, Erc20Relay, EthToVaraEvent,
};
use futures::StreamExt;
use gclient::{GearApi, WSAddress};
use reqwest::Client;
use sails_rs::{calls::*, events::*, gclient::calls::*, prelude::*};
use std::cmp::Ordering;

// TODO: Don't create ethereum clients inside.
pub async fn compose(
    beacon_endpoint: String,
    eth_endpoint: String,
    tx_hash: TxHash,
) -> AnyResult<EthToVaraEvent> {
    let rpc_url = eth_endpoint.parse()?;
    let provider = ProviderBuilder::new().on_http(rpc_url);

    let receipt = provider
        .get_transaction_receipt(tx_hash)
        .await?
        .ok_or(anyhow!("Transaction receipt is missing"))?;

    let block = match receipt.block_hash {
        Some(hash) => provider
            .get_block_by_hash(hash, BlockTransactionsKind::Hashes)
            .await?
            .ok_or(anyhow!("Ethereum block (hash) is missing"))?,
        None => match receipt.block_number {
            Some(number) => provider
                .get_block_by_number(BlockNumberOrTag::Number(number), false)
                .await?
                .ok_or(anyhow!("Ethereum block (number) is missing"))?,
            None => return Err(anyhow!("Unable to get Ethereum block")),
        },
    };

    let beacon_root_parent = block
        .header
        .parent_beacon_block_root
        .ok_or(anyhow!("Unable to determine root of parent beacon block"))?;
    let block_number = block
        .header
        .number
        .ok_or(anyhow!("Unable to determine Ethereum block number"))?;
    let client_http = Client::new();
    let proof_block = build_inclusion_proof(
        &client_http,
        &beacon_endpoint,
        &beacon_root_parent,
        block_number,
    )
    .await?;

    // receipt Merkle-proof
    let tx_index = receipt
        .transaction_index
        .ok_or(anyhow!("Unable to determine transaction index"))?;
    let receipts = provider
        .get_block_receipts(BlockNumberOrTag::Number(block_number))
        .await?
        .unwrap_or_default()
        .iter()
        .map(|tx_receipt| {
            let receipt = tx_receipt.as_ref();

            tx_receipt
                .transaction_index
                .map(|i| (i, eth_utils::map_receipt_envelope(receipt)))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();

    let MerkleProof { proof, receipt } = eth_utils::generate_merkle_proof(tx_index, &receipts[..])?;

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
    Encodable::encode(&receipt, &mut receipt_rlp);

    Ok(EthToVaraEvent {
        proof_block,
        proof,
        transaction_index: tx_index,
        receipt_rlp,
    })
}

async fn build_inclusion_proof(
    client_http: &Client,
    rpc_url: &str,
    beacon_root_parent: &[u8; 32],
    block_number: u64,
) -> AnyResult<BlockInclusionProof> {
    let beacon_block_parent =
        ethereum_beacon_client::get_block_by_hash(client_http, rpc_url, beacon_root_parent).await?;

    let beacon_block = LightBeaconBlock::from(
        find_beacon_block(client_http, rpc_url, block_number, &beacon_block_parent).await?,
    );

    let slot = beacon_block.slot;
    if slot % SLOTS_PER_EPOCH == 0 {
        return Ok(BlockInclusionProof {
            block: beacon_block,
            headers: vec![],
        });
    }

    let epoch_next = 1 + eth_utils::calculate_epoch(beacon_block.slot);
    let slot_checkpoint = epoch_next * SLOTS_PER_EPOCH;

    Ok(BlockInclusionProof {
        block: beacon_block,
        headers: ethereum_beacon_client::request_headers(
            client_http,
            rpc_url,
            slot + 1,
            slot_checkpoint + 1,
        )
        .await?
        .into_iter()
        .collect(),
    })
}

async fn find_beacon_block(
    client_http: &Client,
    rpc_url: &str,
    block_number: u64,
    block_start: &BeaconBlock,
) -> AnyResult<BeaconBlock> {
    match block_number.cmp(&block_start.body.execution_payload.block_number) {
        Ordering::Less => {
            return Err(anyhow!(
                "Requested block number is behind the start beacon block"
            ))
        }
        Ordering::Equal => return Ok(block_start.clone()),
        Ordering::Greater => (),
    }

    let block_finalized = ethereum_beacon_client::get_block_finalized(client_http, rpc_url).await?;

    let slot_start = block_start.slot + 1;
    for slot in slot_start..=block_finalized.slot {
        match ethereum_beacon_client::get_block(client_http, rpc_url, slot).await {
            Ok(block) if block.body.execution_payload.block_number == block_number => {
                return Ok(block)
            }
            Ok(_) => (),
            Err(e)
                if e.downcast_ref::<ethereum_beacon_client::ErrorNotFound>()
                    .is_some() =>
            {
                ()
            }
            Err(e) => return Err(e),
        }
    }

    Err(anyhow!("Block was not found"))
}
