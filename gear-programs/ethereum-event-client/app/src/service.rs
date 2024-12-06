// Incorporate code generated based on the IDL file

use super::{error::Error, RefCell, State};
use checkpoint_light_client_io::{Handle, HandleResult};
use ethereum_common::{
    beacon::{light::Block as LightBeaconBlock, BlockHeader as BeaconBlockHeader},
    hash_db, memory_db,
    patricia_trie::TrieDB,
    tree_hash::TreeHash,
    trie_db::{HashDB, Trie},
    utils::{self as eth_utils, ReceiptEnvelope},
    H256,
};
use ops::ControlFlow::*;
use sails_rs::{gstd::msg, prelude::*};

#[derive(Clone, Debug, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct BlockInclusionProof {
    pub block: LightBeaconBlock,
    pub headers: Vec<BeaconBlockHeader>,
}

#[derive(Clone, Debug, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct EthToVaraEvent {
    pub proof_block: BlockInclusionProof,
    pub proof: Vec<Vec<u8>>,
    pub transaction_index: u64,
    pub receipt_rlp: Vec<u8>,
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct CheckedProofs {
    pub receipt_rlp: Vec<u8>,
    pub transaction_index: u64,
    pub block_number: u64,
}

pub struct EthereumEventClient<'a> {
    state: &'a RefCell<State>,
}

#[sails_rs::service]
impl<'a> EthereumEventClient<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }

    pub fn admin(&self) -> ActorId {
        self.state.borrow().admin
    }

    pub fn checkpoint_light_client_address(&self) -> ActorId {
        self.state.borrow().checkpoint_light_client_address
    }

    /// Check proofs and return receipt if successfull, error otherwise.
    pub async fn check_proofs(&mut self, message: EthToVaraEvent) -> Result<CheckedProofs, Error> {
        let receipt = self.decode_and_check_receipt(&message)?;

        let EthToVaraEvent {
            proof_block: BlockInclusionProof { block, mut headers },
            proof,
            transaction_index,
            ..
        } = message;

        // verify the proof of block inclusion
        let checkpoints = self.state.borrow().checkpoint_light_client_address;
        let slot = block.slot;
        let checkpoint = Self::request_checkpoint(checkpoints, slot).await?;

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

        let block_root = block.tree_hash_root();
        if block_root != block_root_parent {
            return Err(Error::InvalidBlockProof);
        }

        // verify Merkle-PATRICIA proof
        let receipts_root = H256::from(block.body.execution_payload.receipts_root.0 .0);
        let mut memory_db = memory_db::new();
        for proof_node in &proof {
            memory_db.insert(hash_db::EMPTY_PREFIX, proof_node);
        }

        let trie = TrieDB::new(&memory_db, &receipts_root).map_err(|_| Error::TrieDbFailure)?;

        let (key_db, value_db) =
            eth_utils::rlp_encode_index_and_receipt(&transaction_index, &receipt);
        match trie.get(&key_db) {
            Ok(Some(found_value)) if found_value == value_db => Ok(CheckedProofs {
                receipt_rlp: message.receipt_rlp,
                transaction_index,
                block_number: block.body.execution_payload.block_number,
            }),
            _ => Err(Error::InvalidReceiptProof),
        }
    }

    fn decode_and_check_receipt(&self, message: &EthToVaraEvent) -> Result<ReceiptEnvelope, Error> {
        use alloy_rlp::Decodable;

        let receipt = ReceiptEnvelope::decode(&mut &message.receipt_rlp[..])
            .map_err(|_| Error::DecodeReceiptEnvelopeFailure)?;

        if !receipt.is_success() {
            return Err(Error::FailedEthTransaction);
        }

        Ok(receipt)
    }

    async fn request_checkpoint(checkpoints: ActorId, slot: u64) -> Result<H256, Error> {
        let request = Handle::GetCheckpointFor { slot }.encode();
        let reply = msg::send_bytes_for_reply(checkpoints, &request, 0, 0)
            .map_err(|_| Error::SendFailure)?
            .await
            .map_err(|_| Error::ReplyFailure)?;

        match HandleResult::decode(&mut reply.as_slice())
            .map_err(|_| Error::HandleResultDecodeFailure)?
        {
            HandleResult::Checkpoint(Ok((_slot, hash))) => Ok(hash),
            HandleResult::Checkpoint(Err(_)) => Err(Error::MissingCheckpoint),
            _ => panic!("Unexpected result to `GetCheckpointFor` request"),
        }
    }
}
