#![no_std]

pub mod abi;
pub mod error;
pub mod service;

use abi::ERC20_TREASURY;
use alloy_sol_types::SolEvent;
use cell::RefCell;
use checkpoint_light_client_io::{Handle, HandleResult};
use collections::BTreeSet;
use error::Error;
use ethereum_common::{
    beacon::{light::Block as LightBeaconBlock, BlockHeader as BeaconBlockHeader},
    hash_db, memory_db,
    patricia_trie::TrieDB,
    tree_hash::TreeHash,
    trie_db::{HashDB, Trie},
    utils as eth_utils,
    utils::ReceiptEnvelope,
    H160, H256, U256,
};
use sails_rs::{
    gstd::{msg, ExecContext, GStdExecContext},
    prelude::*,
};
use service::Erc20Relay as Erc20RelayService;

const CAPACITY: usize = 500_000;

#[cfg(feature = "gas_calculation")]
const CAPACITY_STEP_SIZE: usize = 50_000;

static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

fn transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe {
        TRANSACTIONS
            .as_mut()
            .expect("Program should be constructed")
    }
}

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
    checkpoints: ActorId,
    address: H160,
    token: H160,
    vft_gateway: Option<ActorId>,
    reply_timeout: u32,
    reply_deposit: u64,
}

pub struct Erc20RelayProgram(RefCell<State>);

#[sails_rs::program]
impl Erc20RelayProgram {
    pub fn new(
        checkpoints: ActorId,
        address: H160,
        token: H160,
        reply_timeout: u32,
        reply_deposit: u64,
    ) -> Self {
        unsafe {
            TRANSACTIONS = Some(BTreeSet::new());
        }

        let exec_context = GStdExecContext::new();
        Self(RefCell::new(State {
            admin: exec_context.actor_id(),
            checkpoints,
            address,
            token,
            vft_gateway: None,
            reply_timeout,
            reply_deposit,
        }))
    }

    pub fn gas_calculation(_reply_timeout: u32, _reply_deposit: u64) -> Self {
        #[cfg(feature = "gas_calculation")]
        {
            let self_ = Self::new(
                Default::default(),
                Default::default(),
                Default::default(),
                _reply_timeout,
                _reply_deposit,
            );

            let transactions = transactions_mut();
            for i in 0..CAPACITY_STEP_SIZE {
                transactions.insert((0, i as u64));
            }

            self_
        }

        #[cfg(not(feature = "gas_calculation"))]
        panic!("Please rebuild with enabled `gas_calculation` feature")
    }

    pub fn erc20_relay(&self) -> Erc20RelayService<GStdExecContext> {
        Erc20RelayService::new(&self.0, GStdExecContext::new())
    }
}
