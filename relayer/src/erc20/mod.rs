use super::{ethereum_checkpoints::utils, *};
use alloy::{
    network::primitives::BlockTransactionsKind,
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

pub async fn relay(args: RelayErc20Args) {
    if let Err(e) = relay_inner(args).await {
        log::error!("{e:?}");
    }
}

async fn relay_inner(args: RelayErc20Args) -> AnyResult<()> {
    log::info!("Started");

    let RelayErc20Args {
        program_id,
        beacon_endpoint,
        vara_domain,
        vara_port,
        vara_suri,
        eth_endpoint,
        tx_hash,
    } = args;

    let program_id: [u8; 32] =
        utils::try_from_hex_encoded(&program_id).expect("Expecting correct ProgramId");
    let tx_hash: [u8; 32] =
        utils::try_from_hex_encoded(&tx_hash).expect("Expecting correct hash of a transaction");

    let rpc_url = eth_endpoint.parse()?;
    let provider = ProviderBuilder::new().on_http(rpc_url);

    let receipt = provider
        .get_transaction_receipt(tx_hash.into())
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
    let message = EthToVaraEvent {
        proof_block,
        proof,
        transaction_index: tx_index,
        receipt_rlp,
    };

    let client = GearApi::init_with(WSAddress::new(vara_domain, vara_port), vara_suri).await?;
    let gas_limit_block = client.block_gas_limit()?;
    // use 95% of block gas limit for all extrinsics
    let gas_limit = gas_limit_block / 100 * 95;

    let remoting = GClientRemoting::new(client);

    let mut erc20_service = Erc20Relay::new(remoting.clone());
    let mut listener = erc20_relay_client::erc_20_relay::events::listener(remoting.clone());
    let mut events = listener.listen().await.unwrap();

    let result = erc20_service
        .relay(message)
        .with_gas_limit(gas_limit)
        .send_recv(program_id.into())
        .await
        .unwrap();

    log::debug!("result = {result:?}");
    if result.is_ok() {
        let event = events.next().await.unwrap();

        log::debug!("event = {event:?}");
    }

    Ok(())
}

async fn build_inclusion_proof(
    client_http: &Client,
    rpc_url: &str,
    beacon_root_parent: &[u8; 32],
    block_number: u64,
) -> AnyResult<BlockInclusionProof> {
    let beacon_block_parent =
        utils::get_block_by_hash(client_http, rpc_url, beacon_root_parent).await?;

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
        headers: utils::request_headers(client_http, rpc_url, slot + 1, slot_checkpoint + 1)
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

    let block_finalized = utils::get_block_finalized(client_http, rpc_url).await?;

    let slot_start = block_start.slot + 1;
    for slot in slot_start..=block_finalized.slot {
        match utils::get_block(client_http, rpc_url, slot).await {
            Ok(block) if block.body.execution_payload.block_number == block_number => {
                return Ok(block)
            }
            Ok(_) => (),
            Err(e) if e.downcast_ref::<utils::ErrorNotFound>().is_some() => (),
            Err(e) => return Err(e),
        }
    }

    Err(anyhow!("Block was not found"))
}
