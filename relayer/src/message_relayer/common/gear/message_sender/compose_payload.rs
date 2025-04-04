use ethereum_beacon_client::BeaconClient;

use alloy::{network::primitives::BlockTransactionsKind, primitives::TxHash, providers::Provider};
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_rlp::Encodable;
use anyhow::{anyhow, Result as AnyResult};
use prometheus::IntCounter;
use sails_rs::prelude::*;

use checkpoint_light_client_io::ethereum_common::{
    beacon,
    utils::{self as eth_utils, MerkleProof},
    SLOTS_PER_EPOCH,
};
use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent};
use ethereum_client::EthApi;

pub async fn compose(
    beacon_client: &BeaconClient,
    eth_client: &EthApi,
    tx_hash: TxHash,
    eth_errors: Option<&IntCounter>,
    beacon_errors: Option<&IntCounter>,
) -> AnyResult<EthToVaraEvent> {
    let provider = eth_client.raw_provider();

    let receipt = provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|e| {
            eth_errors.map(|e| e.inc());
            anyhow!("Failed to get transaction receipt: {e:?}")
        })?
        .ok_or(anyhow!("Transaction receipt is missing"))?;

    let block = match receipt.block_hash {
        Some(hash) => provider
            .get_block_by_hash(hash, BlockTransactionsKind::Hashes)
            .await
            .map_err(|e| {
                eth_errors.map(|e| e.inc());
                anyhow!("Failed to get block by hash: {e:?}")
            })?
            .ok_or(anyhow!("Ethereum block (hash) is missing"))?,
        None => match receipt.block_number {
            Some(number) => provider
                .get_block_by_number(BlockNumberOrTag::Number(number), false)
                .await
                .map_err(|e| {
                    eth_errors.map(|e| e.inc());
                    anyhow!("Failed to get block by number: {e:?}")
                })?
                .ok_or(anyhow!("Ethereum block (number) is missing"))?,
            None => return Err(anyhow!("Unable to get Ethereum block")),
        },
    };

    let beacon_root_parent = block
        .header
        .parent_beacon_block_root
        .ok_or(anyhow!("Unable to determine root of parent beacon block"))?;
    let block_number = block.header.number;

    let proof_block = build_inclusion_proof(
        beacon_client,
        &beacon_root_parent,
        block_number,
        beacon_errors,
    )
    .await?;

    // receipt Merkle-proof
    let tx_index = receipt
        .transaction_index
        .ok_or(anyhow!("Unable to determine transaction index"))?;
    let receipts = provider
        .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(block_number)))
        .await
        .map_err(|e| {
            eth_errors.map(|e| e.inc());
            anyhow!("Failed to get block receipts: {e:?}")
        })?
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
    beacon_client: &BeaconClient,
    beacon_root_parent: &[u8; 32],
    block_number: u64,
    beacon_errors: Option<&IntCounter>,
) -> AnyResult<BlockInclusionProof> {
    let beacon_block_parent = beacon_client
        .get_block_by_hash::<beacon::electra::Block>(beacon_root_parent)
        .await?;

    let beacon_block = beacon_client
        .find_beacon_block(block_number, beacon_block_parent)
        .await
        .map_err(|e| {
            beacon_errors.map(|e| e.inc());
            e
        })?;
    let beacon_block = beacon_client
        .get_block::<beacon::electra::Block>(beacon_block.slot)
        .await
        .map_err(|e| {
            beacon_errors.map(|e| e.inc());
            e
        })?;

    let slot = beacon_block.slot;
    let block = BlockGenericForBlockBody {
        slot,
        proposer_index: beacon_block.proposer_index,
        parent_root: beacon_block.parent_root,
        state_root: beacon_block.state_root,
        body: beacon_block.body.into(),
    };
    if slot % SLOTS_PER_EPOCH == 0 {
        return Ok(BlockInclusionProof {
            block,
            headers: vec![],
        });
    }

    let epoch_next = 1 + eth_utils::calculate_epoch(slot);
    let slot_checkpoint = epoch_next * SLOTS_PER_EPOCH;

    Ok(BlockInclusionProof {
        block,
        headers: beacon_client
            .request_headers(slot + 1, slot_checkpoint + 1)
            .await
            .map_err(|e| {
                beacon_errors.map(|e| e.inc());
                e
            })?
            .into_iter()
            .collect(),
    })
}
