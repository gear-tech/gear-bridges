#![no_std]

use cell::RefCell;
use sails_rs::prelude::*;
use ethereum_common::
    beacon::{
        light::Block as LightBeaconBlock, BlockHeader as BeaconBlockHeader,
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
    checkpoints: ActorId,
    vft: ActorId,
    // (slot, transaction_index)
    transactions: Vec<(u64, u64)>,
}

struct Erc20RelayService<'a>(&'a RefCell<State>);

#[sails_rs::service]
impl<'a> Erc20RelayService<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self(state)
    }

    pub fn relay(&mut self, message: EthToVaraEvent) -> String {
        "Hello from Erc20Relay!".to_string()
    }
}

pub struct Erc20RelayProgram(RefCell<State>);

#[sails_rs::program]
impl Erc20RelayProgram {
    pub fn new(
        checkpoints: ActorId,
        vft: ActorId,
    ) -> Self {
        Self(RefCell::new(State {
            checkpoints,
            vft,
            transactions: Vec::with_capacity(CAPACITY),
        }))
    }

    pub fn erc20_relay(&self) -> Erc20RelayService {
        Erc20RelayService::new(&self.0)
    }
}
