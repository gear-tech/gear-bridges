#![no_std]

use checkpoint_light_client_client::{traits::ServiceCheckpointFor as _, ServiceCheckpointFor};
use ethereum_common::{
    beacon::BlockHeader as BeaconBlockHeader,
    hash_db, memory_db,
    patricia_trie::TrieDB,
    tree_hash::TreeHash,
    trie_db::{HashDB, Trie},
    utils::{self as eth_utils, ReceiptEnvelope},
    H256,
};
use ops::ControlFlow::*;
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
            slot,
            block_root,
            receipts_root,
            block_number,
            mut headers,
            proof,
            transaction_index,
            receipt_rlp,
        } = self;

        let receipt = decode_and_check_receipt(&receipt_rlp)?;

        // verify the proof of block inclusion
        let checkpoint = request_checkpoint(checkpoint_light_client_address, slot).await?;

        headers.sort_unstable_by(|a, b| a.slot.cmp(&b.slot));
        let Continue(block_root_parent) =
            headers
                .iter()
                .rev()
                .try_fold(checkpoint, |block_root_parent, header| {
                    let block_root = header.tree_hash_root();
                    match block_root == block_root_parent {
                        true => Continue(header.parent_root),
                        false => Break(()),
                    }
                })
        else {
            return Err(Error::InvalidBlockProof);
        };

        if block_root != block_root_parent {
            return Err(Error::InvalidBlockProof);
        }

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

    let receipt = ReceiptEnvelope::decode(&mut &receipt_rlp[..])
        .map_err(|_| Error::DecodeReceiptEnvelopeFailure)?;

    if !receipt.is_success() {
        return Err(Error::FailedEthTransaction);
    }

    Ok(receipt)
}

async fn request_checkpoint(
    checkpoint_light_client_address: ActorId,
    slot: u64,
) -> Result<H256, Error> {
    let service = ServiceCheckpointFor::new(GStdRemoting);
    let result = service
        .get(slot)
        .recv(checkpoint_light_client_address)
        .await
        .map_err(|_| Error::SendFailure)?;

    match result {
        Ok((_slot, hash)) => Ok(hash),
        Err(_) => Err(Error::MissingCheckpoint),
    }
}
