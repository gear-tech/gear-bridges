#![no_std]

use cell::RefCell;
use eth_events_common::{CheckedProofs, Error, Proofs, State};
use ethereum_common::{
    beacon::{light::Block as LightBeaconBlock, BlockHeader as BeaconBlockHeader},
    tree_hash::TreeHash,
    H256,
};
use sails_rs::{
    gstd::{ExecContext, GStdExecContext},
    prelude::*,
};

pub struct Program(RefCell<State>);

#[sails_rs::program]
impl Program {
    pub fn new(checkpoint_light_client_address: ActorId) -> Self {
        let exec_context = GStdExecContext::new();
        Self(RefCell::new(State {
            admin: exec_context.actor_id(),
            checkpoint_light_client_address,
        }))
    }

    pub fn ethereum_event_client(&self) -> Service {
        Service::new(&self.0)
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

pub struct Service<'a> {
    state: &'a RefCell<State>,
}

#[sails_rs::service]
impl<'a> Service<'a> {
    pub fn new(state: &'a RefCell<State>) -> Self {
        Self { state }
    }

    pub fn admin(&self) -> ActorId {
        self.state.borrow().admin
    }

    pub fn checkpoint_light_client_address(&self) -> ActorId {
        self.state.borrow().checkpoint_light_client_address
    }

    pub async fn check_proofs(&mut self, message: EthToVaraEvent) -> Result<CheckedProofs, Error> {
        let EthToVaraEvent {
            proof_block: BlockInclusionProof { block, headers },
            proof,
            transaction_index,
            receipt_rlp,
        } = message;

        Proofs {
            checkpoint_light_client_address: self.checkpoint_light_client_address(),
            slot: block.slot,
            block_root: block.tree_hash_root(),
            receipts_root: H256::from(block.body.execution_payload.receipts_root.0 .0),
            block_number: block.body.execution_payload.block_number,
            headers,
            proof,
            transaction_index,
            receipt_rlp,
        }
        .check()
        .await
    }
}
