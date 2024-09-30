use super::{ethereum_checkpoints::utils, *};
use alloy::{
    network::primitives::BlockTransactionsKind,
    providers::{Provider, ProviderBuilder},
    rpc::types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom},
};
use alloy_consensus::TxType;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::Log as PrimitiveLog;
use alloy_rlp::Encodable;
use anyhow::{anyhow, Result as AnyResult};
use checkpoint_light_client_io::ethereum_common::{
    beacon::{self, light::Block as LightBeaconBlock, Block as BeaconBlock},
    memory_db,
    patricia_trie::{TrieDB, TrieDBMut},
    trie_db::{Recorder, Trie, TrieMut},
    utils as eth_utils, H256, SLOTS_PER_EPOCH,
};
use erc20_relay_client::{
    traits::Erc20Relay as _, Block, BlockBody, BlockHeader, BlockInclusionProof, Erc20Relay,
    EthToVaraEvent, ExecutionPayload,
};
use futures::StreamExt;
use gclient::{GearApi, WSAddress};
use reqwest::Client;
use sails_rs::{calls::*, events::*, gclient::calls::*, prelude::*};
use std::cmp::Ordering;

fn into_idl_execution_payload(
    execution_payload: beacon::light::ExecutionPayload,
) -> ExecutionPayload {
    use erc20_relay_client::{
        ByteList, BytesFixed1, BytesFixed2, FixedArray1ForU8, FixedArray2ForU8, ListForU8,
    };

    ExecutionPayload {
        parent_hash: BytesFixed1(FixedArray1ForU8(execution_payload.parent_hash.0 .0)),
        fee_recipient: BytesFixed2(FixedArray2ForU8(execution_payload.fee_recipient.0 .0)),
        state_root: BytesFixed1(FixedArray1ForU8(execution_payload.state_root.0 .0)),
        receipts_root: BytesFixed1(FixedArray1ForU8(execution_payload.receipts_root.0 .0)),
        logs_bloom: execution_payload.logs_bloom,
        prev_randao: BytesFixed1(FixedArray1ForU8(execution_payload.prev_randao.0 .0)),
        block_number: execution_payload.block_number,
        gas_limit: execution_payload.gas_limit,
        gas_used: execution_payload.gas_used,
        timestamp: execution_payload.timestamp,
        extra_data: ByteList(ListForU8 {
            data: execution_payload.extra_data.0.as_ref().into(),
        }),
        base_fee_per_gas: execution_payload.base_fee_per_gas,
        block_hash: BytesFixed1(FixedArray1ForU8(execution_payload.block_hash.0 .0)),
        transactions: execution_payload.transactions,
        withdrawals: execution_payload.withdrawals,
        blob_gas_used: execution_payload.blob_gas_used,
        excess_blob_gas: execution_payload.excess_blob_gas,
    }
}

fn into_idl_block_body(block_body: beacon::light::BlockBody) -> BlockBody {
    use erc20_relay_client::{BytesFixed1, FixedArray1ForU8};

    BlockBody {
        randao_reveal: block_body.randao_reveal,
        eth1_data: block_body.eth1_data,
        graffiti: BytesFixed1(FixedArray1ForU8(block_body.graffiti.0 .0)),
        proposer_slashings: block_body.proposer_slashings,
        attester_slashings: block_body.attester_slashings,
        attestations: block_body.attestations,
        deposits: block_body.deposits,
        voluntary_exits: block_body.voluntary_exits,
        sync_aggregate: block_body.sync_aggregate,
        execution_payload: into_idl_execution_payload(block_body.execution_payload),
        bls_to_execution_changes: block_body.bls_to_execution_changes,
        blob_kzg_commitments: block_body.blob_kzg_commitments,
    }
}

fn into_idl_block(block: LightBeaconBlock) -> Block {
    Block {
        slot: block.slot,
        proposer_index: block.proposer_index,
        parent_root: block.parent_root,
        state_root: block.state_root,
        body: into_idl_block_body(block.body),
    }
}

fn into_idl_block_header(block_heaer: beacon::BlockHeader) -> BlockHeader {
    BlockHeader {
        slot: block_heaer.slot,
        proposer_index: block_heaer.proposer_index,
        parent_root: block_heaer.parent_root,
        state_root: block_heaer.state_root,
        body_root: block_heaer.body_root,
    }
}

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
                .map(|i| (i, map_receipt_envelope(receipt)))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();

    let mut memory_db = memory_db::new();
    let key_value_tuples = eth_utils::rlp_encode_receipts_and_nibble_tuples(&receipts[..]);
    let root = {
        let mut root = H256::zero();
        let mut triedbmut = TrieDBMut::new(&mut memory_db, &mut root);
        for (key, value) in &key_value_tuples {
            triedbmut.insert(key, value)?;
        }

        *triedbmut.root()
    };

    let (tx_index, receipt) = receipts
        .iter()
        .find(|(index, _)| index == &tx_index)
        .ok_or(anyhow!("Unable to find transaction's receipt"))?;

    let trie = TrieDB::new(&memory_db, &root)?;
    let (key, _expected_value) = eth_utils::rlp_encode_index_and_receipt(tx_index, receipt);

    let mut recorder = Recorder::new();
    let _value = trie.get_with(&key, &mut recorder);

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(receipt));
    Encodable::encode(receipt, &mut receipt_rlp);
    let message = EthToVaraEvent {
        proof_block,
        proof: recorder
            .drain()
            .into_iter()
            .map(|r| r.data)
            .collect::<Vec<_>>(),
        transaction_index: *tx_index,
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
            block: into_idl_block(beacon_block),
            headers: vec![],
        });
    }

    let epoch_next = 1 + eth_utils::calculate_epoch(beacon_block.slot);
    let slot_checkpoint = epoch_next * SLOTS_PER_EPOCH;

    Ok(BlockInclusionProof {
        block: into_idl_block(beacon_block),
        headers: utils::request_headers(client_http, rpc_url, slot + 1, slot_checkpoint + 1)
            .await?
            .into_iter()
            .map(into_idl_block_header)
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

fn map_receipt_envelope(receipt: &ReceiptEnvelope<Log>) -> ReceiptEnvelope<PrimitiveLog> {
    let logs = receipt
        .logs()
        .iter()
        .map(AsRef::as_ref)
        .cloned()
        .collect::<Vec<_>>();

    let result = ReceiptWithBloom::new(
        Receipt {
            status: receipt.status().into(),
            cumulative_gas_used: receipt.cumulative_gas_used(),
            logs,
        },
        *receipt.logs_bloom(),
    );

    match receipt.tx_type() {
        TxType::Legacy => ReceiptEnvelope::Legacy(result),
        TxType::Eip1559 => ReceiptEnvelope::Eip1559(result),
        TxType::Eip2930 => ReceiptEnvelope::Eip2930(result),
        TxType::Eip4844 => ReceiptEnvelope::Eip4844(result),
    }
}
