#![no_std]

use checkpoint_light_client_client::{
    traits::{ServiceCheckpointFor as _, ServiceState as _},
    ServiceCheckpointFor, ServiceState,
};
use ethereum_common::{
    beacon::BlockHeader as BeaconBlockHeader,
    hash_db, memory_db,
    patricia_trie::TrieDB,
    tree_hash::TreeHash,
    trie_db::{HashDB, Trie},
    utils::{self as eth_utils, ReceiptEnvelope},
    H256,
};
use sails_rs::{calls::*, gstd::calls::GStdRemoting, prelude::*};

#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum Error {
    DecodeReceiptEnvelopeFailure,
    FailedEthTransaction,
    SendFailure,
    ReplyFailure,
    HandleResultDecodeFailure,
    MissingCheckpoint,
    InvalidBlockProof,
    TrieDbFailure,
    InvalidReceiptProof,
    UnsupportedFork,
}

pub struct State {
    pub checkpoint_light_client_address: ActorId,
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct CheckedProofs {
    pub receipt_rlp: Vec<u8>,
    pub transaction_index: u64,
    pub block_number: u64,
    pub slot: u64,
}

#[derive(Clone, Debug)]
pub struct Proofs {
    pub checkpoint_light_client_address: ActorId,
    pub electra: bool,
    pub slot: u64,
    pub block_root: H256,
    pub receipts_root: H256,
    pub block_number: u64,
    pub headers: Vec<BeaconBlockHeader>,
    pub proof: Vec<Vec<u8>>,
    pub transaction_index: u64,
    pub receipt_rlp: Vec<u8>,
}

impl Proofs {
    /// Check proofs and return `CheckedProofs` if successfull, error otherwise.
    pub async fn check(self) -> Result<CheckedProofs, Error> {
        let Proofs {
            checkpoint_light_client_address,
            electra,
            slot,
            block_root,
            receipts_root,
            block_number,
            headers,
            proof,
            transaction_index,
            receipt_rlp,
        } = self;

        let receipt = decode_and_check_receipt(&receipt_rlp)?;

        let network = ServiceState::new(GStdRemoting)
            .network()
            .recv(checkpoint_light_client_address)
            .await
            .map_err(|_| Error::ReplyFailure)?;
        let epoch = eth_utils::calculate_epoch(slot);
        if epoch < network.epoch_deneb() || electra != (epoch >= network.epoch_electra()) {
            return Err(Error::UnsupportedFork);
        }
        let (checkpoint_slot, checkpoint_root) =
            request_checkpoint(checkpoint_light_client_address, slot).await?;
        check_ancestry(slot, block_root, &headers, checkpoint_slot, checkpoint_root)?;

        // verify Merkle-PATRICIA proof
        let mut memory_db = memory_db::new();
        for proof_node in &proof {
            memory_db.insert(hash_db::EMPTY_PREFIX, proof_node);
        }

        let trie = TrieDB::new(&memory_db, &receipts_root).map_err(|_| Error::TrieDbFailure)?;

        let (key_db, value_db) =
            eth_utils::rlp_encode_index_and_receipt(&transaction_index, &receipt);
        match trie.get(&key_db) {
            Ok(Some(found_value)) if found_value == value_db => Ok(CheckedProofs {
                receipt_rlp,
                transaction_index,
                block_number,
                slot,
            }),
            _ => Err(Error::InvalidReceiptProof),
        }
    }
}

fn decode_and_check_receipt(receipt_rlp: &[u8]) -> Result<ReceiptEnvelope, Error> {
    use alloy_rlp::Decodable;

    let mut input = receipt_rlp;
    let receipt =
        ReceiptEnvelope::decode(&mut input).map_err(|_| Error::DecodeReceiptEnvelopeFailure)?;
    if !input.is_empty() {
        return Err(Error::DecodeReceiptEnvelopeFailure);
    }
    if receipt
        .as_receipt()
        .and_then(|receipt| receipt.status.as_eip658())
        != Some(true)
    {
        return Err(Error::FailedEthTransaction);
    }

    Ok(receipt)
}

async fn request_checkpoint(
    checkpoint_light_client_address: ActorId,
    slot: u64,
) -> Result<(u64, H256), Error> {
    let service = ServiceCheckpointFor::new(GStdRemoting);
    let result = service
        .get(slot)
        .recv(checkpoint_light_client_address)
        .await
        .map_err(|_| Error::SendFailure)?;

    match result {
        Ok(checkpoint) => Ok(checkpoint),
        Err(_) => Err(Error::MissingCheckpoint),
    }
}

fn check_ancestry(
    slot: u64,
    block_root: H256,
    headers: &[BeaconBlockHeader],
    checkpoint_slot: u64,
    checkpoint_root: H256,
) -> Result<(), Error> {
    if slot > checkpoint_slot {
        return Err(Error::InvalidBlockProof);
    }
    let mut previous_slot = slot;
    let mut previous_root = block_root;
    for header in headers {
        if header.slot <= previous_slot
            || header.slot > checkpoint_slot
            || header.parent_root != previous_root
        {
            return Err(Error::InvalidBlockProof);
        }
        previous_slot = header.slot;
        previous_root = header.tree_hash_root();
    }
    if previous_slot != checkpoint_slot || previous_root != checkpoint_root {
        return Err(Error::InvalidBlockProof);
    }
    Ok(())
}
