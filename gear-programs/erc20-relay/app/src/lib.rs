#![no_std]

pub mod abi;
pub mod error;
pub mod services;

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
    gstd::{debug, msg, ExecContext, GStdExecContext},
    prelude::*,
};
use services::{Erc20Relay as Erc20RelayService, FTManage as FTManageService};

const CAPACITY: usize = 500_000;
const CAPACITY_MAP: usize = 100;

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

pub struct State {
    admin: ActorId,
    map: Vec<(H160, ActorId)>,
    checkpoints: ActorId,
    // vft: ActorId,
    // (slot, transaction_index)
    transactions: Vec<(u64, u64)>,
}

pub struct Erc20RelayProgram(RefCell<State>);

#[sails_rs::program]
impl Erc20RelayProgram {
    pub fn new(checkpoints: ActorId, _vft: ActorId) -> Self {
        let exec_context = GStdExecContext::new();
        Self(RefCell::new(State {
            admin: exec_context.actor_id(),
            map: Vec::with_capacity(CAPACITY_MAP),
            checkpoints,
            // vft,
            transactions: Vec::with_capacity(CAPACITY),
        }))
    }

    pub fn erc20_relay(&self) -> Erc20RelayService {
        Erc20RelayService::new(&self.0)
    }

    pub fn ft_manage(&self) -> FTManageService<GStdExecContext> {
        FTManageService::new(&self.0, GStdExecContext::new())
    }
}
