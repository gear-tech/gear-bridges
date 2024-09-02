#![no_std]

pub mod abi;

use abi::ERC20_TREASURY;
use alloy_sol_types::SolEvent;
use cell::RefCell;
use checkpoint_light_client_io::{Handle, HandleResult};
use cmp::Ordering;
use ethereum_common::{
    beacon::{light::Block as LightBeaconBlock, BlockHeader as BeaconBlockHeader},
    hash_db, memory_db,
    patricia_trie::TrieDB,
    tree_hash::TreeHash,
    trie_db::{HashDB, Trie},
    utils as eth_utils,
    utils::ReceiptEnvelope,
    H160, H256,
};
use ops::ControlFlow;
use sails_rs::{
    gstd::{debug, msg},
    prelude::*,
};

const CAPACITY: usize = 500_000;

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

struct State {
    map: Vec<(H160, ActorId)>,
    checkpoints: ActorId,
    // vft: ActorId,
    // (slot, transaction_index)
    transactions: Vec<(u64, u64)>,
}

struct Erc20RelayService<'a>(&'a RefCell<State>);

#[sails_rs::service]
impl<'a> Erc20RelayService<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self(state)
    }

    pub async fn relay(&mut self, message: EthToVaraEvent) {
        use alloy_rlp::Decodable;

        let Ok(receipt) = ReceiptEnvelope::decode(&mut &message.receipt_rlp[..]) else {
            // TODO: event
            return;
        };

        if !receipt.is_success() {
            // TODO: event
            return;
        }

        let slot = message.proof_block.block.slot;
        {
            let state = self.0.borrow();
            // decode log and pick the corresponding fungible token address if any
            let Some((_fungible_token, _event)) = receipt.logs().iter().find_map(|log| {
                let Ok(event) = ERC20_TREASURY::Deposit::decode_log_data(log, true) else {
                    return None;
                };

                state
                    .map
                    .iter()
                    .find_map(|(address, fungible_token)| {
                        (address.0 == event.token.0).then_some(fungible_token)
                    })
                    .map(|fungible_token| (fungible_token, event))
            }) else {
                //TODO: event
                return;
            };

            // check for double spending
            let Err(index) =
                state
                    .transactions
                    .binary_search_by(|(slot_old, transaction_index_old)| {
                        match slot.cmp(slot_old) {
                            Ordering::Equal => message.transaction_index.cmp(transaction_index_old),
                            ordering => ordering,
                        }
                    })
            else {
                // TODO: event
                return;
            };

            if state.transactions.capacity() <= state.transactions.len()
                && index == state.transactions.len() - 1
            {
                // TODO: event
                return;
            }
        }

        // verify the proof of block inclusion
        let checkpoints = self.0.borrow().checkpoints;
        let Some(result) = Self::request_checkpoint(checkpoints, slot).await else {
            // TODO: event
            return;
        };
        let checkpoint = match result {
            HandleResult::Checkpoint(Ok(checkpoint)) => checkpoint.1,
            HandleResult::Checkpoint(Err(_)) => {
                // TODO: event
                return;
            }
            _ => panic!("Unexpected result to `GetCheckpointFor` request"),
        };

        // TODO: sort headers
        let ControlFlow::Continue(block_root_parent) =
            message
                .proof_block
                .headers
                .iter()
                .try_fold(checkpoint, |block_root_parent, header| {
                    let block_root = header.tree_hash_root();
                    if block_root == block_root_parent {
                        ControlFlow::Continue(block_root)
                    } else {
                        ControlFlow::Break(block_root_parent)
                    }
                })
        else {
            // TODO: event
            return;
        };

        let block_root = message.proof_block.block.tree_hash_root();
        if block_root != block_root_parent {
            // TODO: event
            return;
        }

        // verify Merkle-PATRICIA proof
        let receipts_root = H256::from(
            message
                .proof_block
                .block
                .body
                .execution_payload
                .receipts_root
                .0
                 .0,
        );
        let mut memory_db = memory_db::new();
        for proof_node in &message.proof {
            memory_db.insert(hash_db::EMPTY_PREFIX, proof_node);
        }

        let Ok(trie) = TrieDB::new(&memory_db, &receipts_root) else {
            //TODO: event
            return;
        };

        let (key_db, value_db) =
            eth_utils::rlp_encode_index_and_receipt(&message.transaction_index, &receipt);
        match trie.get(&key_db) {
            Ok(Some(found_value)) if found_value == value_db => {
                debug!("Proofs are valid. Mint the tokens");
                //TODO: event
            }
            _ => {
                //TODO: event
            }
        }
    }

    pub async fn request_checkpoint(checkpoints: ActorId, slot: u64) -> Option<HandleResult> {
        let request = Handle::GetCheckpointFor { slot }.encode();
        let reply = msg::send_bytes_for_reply(checkpoints, &request, 0, 0)
            .ok()?
            .await
            .ok()?;

        HandleResult::decode(&mut reply.as_slice()).ok()
    }
}

pub struct Erc20RelayProgram(RefCell<State>);

#[sails_rs::program]
impl Erc20RelayProgram {
    pub fn new(checkpoints: ActorId, _vft: ActorId) -> Self {
        Self(RefCell::new(State {
            map: vec![],
            checkpoints,
            // vft,
            transactions: Vec::with_capacity(CAPACITY),
        }))
    }

    pub fn erc20_relay(&self) -> Erc20RelayService {
        Erc20RelayService::new(&self.0)
    }
}
