// Incorporate code generated based on the IDL file
#[allow(dead_code)]
mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-gateway.rs"));
}

use super::{error::Error, BTreeSet, Config, ExecContext, RefCell, State};
use checkpoint_light_client_io::{Handle, HandleResult};
use collections::HashMap;
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
use ops::ControlFlow::*;
use sails_rs::{
    calls::ActionIo,
    gstd::{self, msg},
    prelude::*,
};
use vft::vft_gateway::io::MintTokens;

type MintTokensReplies = HashMap<MessageId, (H160, ActorId, U256)>;

pub(crate) const CAPACITY: usize = 500_000;

#[cfg(feature = "gas_calculation")]
pub(crate) const CAPACITY_STEP_SIZE: usize = 50_000;

pub(crate) static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;
pub(crate) static mut MINT_TOKENS_REPLIES: Option<MintTokensReplies> = None;

pub(crate) fn transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe {
        TRANSACTIONS
            .as_mut()
            .expect("TRANSACTIONS; Program should be constructed")
    }
}

pub(crate) fn mint_tokens_replies_mut() -> &'static mut MintTokensReplies {
    unsafe { MINT_TOKENS_REPLIES.as_mut() }
        .expect("MINT_TOKENS_REPLIES; Program should be constructed")
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

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    Relayed {
        slot: u64,
        block_number: u64,
        transaction_index: u32,
        fungible_token: H160,
        to: ActorId,
        amount: U256,
    },
}

pub struct Erc20Relay<'a, ExecContext> {
    state: &'a RefCell<State>,
    exec_context: ExecContext,
}

#[sails_rs::service(events = Event)]
impl<'a, T> Erc20Relay<'a, T>
where
    T: ExecContext,
{
    pub fn new(state: &'a RefCell<State>, exec_context: T) -> Self {
        Self {
            state,
            exec_context,
        }
    }

    pub fn vft_gateway(&self) -> ActorId {
        self.state.borrow().vft_gateway
    }

    pub fn set_vft_gateway(&mut self, vft_gateway: ActorId) {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        state.vft_gateway = vft_gateway;
    }

    pub fn config(&self) -> Config {
        self.state.borrow().config
    }

    pub fn update_config(&mut self, config_new: Config) {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        state.config = config_new;
    }

    pub fn admin(&self) -> ActorId {
        self.state.borrow().admin
    }

    pub fn checkpoint_light_client_address(&self) -> ActorId {
        self.state.borrow().checkpoint_light_client_address
    }

    pub async fn relay(&mut self, message: EthToVaraEvent) -> Result<(), Error> {
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
            Ok(Some(found_value)) if found_value == value_db => (),
            _ => return Err(Error::InvalidReceiptProof),
        }

        let call_payload = MintTokens::encode_call(message.receipt_rlp);
        let (vft_gateway_id, reply_timeout, reply_deposit) = {
            let state = self.state.borrow();

            (
                state.vft_gateway,
                state.config.reply_timeout,
                state.config.reply_deposit,
            )
        };

        let msg_id = gstd::msg::id();
        gstd::msg::send_bytes_for_reply(vft_gateway_id, call_payload, 0, reply_deposit)
            .map_err(|_| Error::SendFailure)?
            .up_to(Some(reply_timeout))
            .map_err(|_| Error::ReplyTimeout)?
            .handle_reply(move || handle_reply(msg_id, slot, transaction_index))
            .map_err(|_| Error::ReplyHook)?
            .await
            .map_err(|_| Error::ReplyFailure)?;

        let (fungible_token, to, amount) = mint_tokens_replies_mut()
            .remove(&msg_id)
            .expect("handle_reply should insert the record");
        let _ = self.notify_on(Event::Relayed {
            slot,
            block_number: block.body.execution_payload.block_number,
            transaction_index: transaction_index as u32,
            fungible_token,
            to,
            amount,
        });

        Ok(())
    }

    fn decode_and_check_receipt(&self, message: &EthToVaraEvent) -> Result<ReceiptEnvelope, Error> {
        use alloy_rlp::Decodable;

        let receipt = ReceiptEnvelope::decode(&mut &message.receipt_rlp[..])
            .map_err(|_| Error::DecodeReceiptEnvelopeFailure)?;

        if !receipt.is_success() {
            return Err(Error::FailedEthTransaction);
        }

        // check for double spending
        let slot = message.proof_block.block.slot;
        let transactions = transactions_mut();
        let key = (slot, message.transaction_index);
        if transactions.contains(&key) {
            return Err(Error::AlreadyProcessed);
        }

        if CAPACITY <= transactions.len()
            && transactions
                .first()
                .map(|first| &key < first)
                .unwrap_or(false)
        {
            return Err(Error::TooOldTransaction);
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

    pub fn fill_transactions(&mut self) -> bool {
        #[cfg(feature = "gas_calculation")]
        {
            let transactions = transactions_mut();
            if CAPACITY == transactions.len() {
                return false;
            }

            let count = cmp::min(CAPACITY - transactions.len(), CAPACITY_STEP_SIZE);
            let (last, _) = transactions.last().copied().unwrap();
            for i in 0..count {
                transactions.insert((last + 1, i as u64));
            }

            true
        }

        #[cfg(not(feature = "gas_calculation"))]
        panic!("Please rebuild with enabled `gas_calculation` feature")
    }

    pub async fn calculate_gas_for_reply(
        &mut self,
        _slot: u64,
        _transaction_index: u64,
    ) -> Result<(), Error> {
        #[cfg(feature = "gas_calculation")]
        {
            let call_payload = MintTokens::encode_call(Default::default());
            let (reply_timeout, reply_deposit) = {
                let state = self.state.borrow();

                (state.config.reply_timeout, state.config.reply_deposit)
            };
            let source = self.exec_context.actor_id();
            let msg_id = gstd::msg::id();
            gstd::msg::send_bytes_for_reply(source, call_payload, 0, reply_deposit)
                .map_err(|_| Error::SendFailure)?
                .up_to(Some(reply_timeout))
                .map_err(|_| Error::ReplyTimeout)?
                .handle_reply(move || handle_reply(msg_id, _slot, _transaction_index))
                .map_err(|_| Error::ReplyHook)?
                .await
                .map_err(|_| Error::ReplyFailure)?;

            let _reply = mint_tokens_replies_mut()
                .remove(&msg_id)
                .expect("handle_reply should insert the record");

            Ok(())
        }

        #[cfg(not(feature = "gas_calculation"))]
        panic!("Please rebuild with enabled `gas_calculation` feature")
    }
}

fn handle_reply(msg_id: MessageId, slot: u64, transaction_index: u64) {
    let reply_bytes = msg::load_bytes().expect("Unable to load bytes");
    let reply = MintTokens::decode_reply(&reply_bytes)
        .expect("Unable to decode MintTokens reply")
        .unwrap_or_else(|e| panic!("Request to mint tokens failed: {e:?}"));

    let transactions = transactions_mut();
    if CAPACITY <= transactions.len() {
        transactions.pop_first();
    }

    transactions.insert((slot, transaction_index));
    mint_tokens_replies_mut().insert(msg_id, reply);
}
