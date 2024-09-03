#![no_std]

pub mod abi;
pub mod error;

use abi::ERC20_TREASURY;
use alloy_sol_types::SolEvent;
use cell::RefCell;
use checkpoint_light_client_io::{Handle, HandleResult};
use cmp::Ordering;
use error::Error;
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

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    Relayed {
        fungible_token: ActorId,
        to: ActorId,
        amount: u128,
    },
}

struct Erc20RelayService<'a>(&'a RefCell<State>);

#[sails_rs::service(events = Event)]
impl<'a> Erc20RelayService<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self(state)
    }

    pub async fn relay(&mut self, message: EthToVaraEvent) -> Result<(), Error> {
        use alloy_rlp::Decodable;

        let receipt = ReceiptEnvelope::decode(&mut &message.receipt_rlp[..])
            .map_err(|_| Error::DecodeReceiptEnvelopeFailure)?;

        if !receipt.is_success() {
            return Err(Error::FailedEthTransaction);
        }

        let slot = message.proof_block.block.slot;
        let (fungible_token, event) = {
            let mut state = self.0.borrow_mut();
            // decode log and pick the corresponding fungible token address if any
            let (fungible_token, event) = receipt
                .logs()
                .iter()
                .find_map(|log| {
                    let Ok(event) = ERC20_TREASURY::Deposit::decode_log_data(log, true) else {
                        return None;
                    };

                    state
                        .map
                        .iter()
                        .find_map(|(address, fungible_token)| {
                            (address.0 == event.token.0).then_some(fungible_token)
                        })
                        .map(|fungible_token| (*fungible_token, event))
                })
                .ok_or(Error::NotSupportedEvent)?;

            // check for double spending
            let index = state
                .transactions
                .binary_search_by(
                    |(slot_old, transaction_index_old)| match slot.cmp(slot_old) {
                        Ordering::Equal => message.transaction_index.cmp(transaction_index_old),
                        ordering => ordering,
                    },
                )
                .err()
                .ok_or(Error::AlreadyProcessed)?;

            if state.transactions.capacity() <= state.transactions.len() {
                if index == state.transactions.len() - 1 {
                    return Err(Error::TooOldTransaction);
                }

                state.transactions.pop();
            }

            (fungible_token, event)
        };

        let amount = u128::try_from(event.amount).map_err(|_| Error::InvalidAmount)?;

        // verify the proof of block inclusion
        let checkpoints = self.0.borrow().checkpoints;
        let checkpoint = Self::request_checkpoint(checkpoints, slot).await?;

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
            return Err(Error::InvalidBlockProof);
        };

        let block_root = message.proof_block.block.tree_hash_root();
        if block_root != block_root_parent {
            return Err(Error::InvalidBlockProof);
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

        let trie = TrieDB::new(&memory_db, &receipts_root).map_err(|_| Error::TrieDbFailure)?;

        let (key_db, value_db) =
            eth_utils::rlp_encode_index_and_receipt(&message.transaction_index, &receipt);
        match trie.get(&key_db) {
            Ok(Some(found_value)) if found_value == value_db => {
                // TODO
                debug!("Proofs are valid. Mint the tokens");
                // TODO: save slot and index of the processed transaction

                self.notify_on(Event::Relayed {
                    fungible_token,
                    to: ActorId::from(event.to.0),
                    amount,
                })
                .unwrap();

                Ok(())
            }

            _ => Err(Error::InvalidReceiptProof),
        }
    }

    pub async fn request_checkpoint(checkpoints: ActorId, slot: u64) -> Result<H256, Error> {
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
