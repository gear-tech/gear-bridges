use super::super::TokenSupply;
use gstd::{prelude::collections::HashMap, MessageId};
use sails_rs::prelude::*;

static mut MSG_TRACKER: Option<MessageTracker> = None;

// TODO: Remove completed messages from tracker.
/// State machine which tracks state of each message that was submitted into
/// `request_bridging` method.
#[derive(Default, Debug)]
pub struct MessageTracker {
    /// Message states.
    pub message_info: HashMap<MessageId, MessageInfo>,
}

/// Entry for a single message in [MessageTracker].
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct MessageInfo {
    /// State of the message.
    pub status: MessageStatus,
    /// Message details.
    pub details: TxDetails,
}

/// Details about a message stored in [MessageTracker].
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct TxDetails {
    /// Address of the `VFT` token which is being bridged.
    pub vara_token_id: ActorId,
    /// Original `VFT` token owner.
    pub sender: ActorId,
    /// Bridged token amount.
    pub amount: U256,
    /// `ERC20` token receiver on Ethereum.
    pub receiver: H160,
    /// [TokenSupply] type of the token being bridged.
    pub token_supply: TokenSupply,
}

/// State in which message processing can be.
#[derive(Debug, Clone, PartialEq, Encode, Decode, TypeInfo)]
pub enum MessageStatus {
    /// Message to deposit tokens is awaiting to be sent.
    SendingMessageToDepositTokens,
    /// Message to deposit tokens sent and reply is received.
    TokenDepositCompleted(bool),

    /// Message to the `pallet-gear-eth-bridge` is awaiting to be sent.
    SendingMessageToBridgeBuiltin,
    /// Message to the `pallet-gear-eth-bridge` is sent and reply is received.
    BridgeResponseReceived(Option<U256>),

    /// Message to refund tokens is awaiting to be sent.
    SendingMessageToReturnTokens,
    /// Message to refund tokens is sent and reply is received.
    TokensReturnComplete(bool),
}

/// Initialize global state of the message tracker.
pub fn init() {
    unsafe { MSG_TRACKER = Some(MessageTracker::default()) }
}

/// Fetch state of this message tracker.
pub fn msg_tracker_state() -> Vec<(MessageId, MessageInfo)> {
    unsafe {
        MSG_TRACKER
            .as_mut()
            .expect("VftManager::seed() should be called")
    }
    .message_info
    .clone()
    .into_iter()
    .collect()
}

/// Get mutable reference to a global message tracker.
pub fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_mut()
            .expect("VftManager::seed() should be called")
    }
}

impl MessageTracker {
    /// Start tracking state of the message.
    pub fn insert_message_info(
        &mut self,
        msg_id: MessageId,
        status: MessageStatus,
        details: TxDetails,
    ) {
        self.message_info
            .insert(msg_id, MessageInfo { status, details });
    }

    /// Drive state machine further for a given `msg_id`.
    pub fn update_message_status(&mut self, msg_id: MessageId, status: MessageStatus) {
        if let Some(info) = self.message_info.get_mut(&msg_id) {
            info.status = status;
        }
    }

    /// Get current state of the tracked message. Will return `None` if message isn't found.
    pub fn get_message_info(&self, msg_id: &MessageId) -> Option<&MessageInfo> {
        self.message_info.get(msg_id)
    }

    /// Stop tracking message state. It will return current state of the target message.
    pub fn remove_message_info(&mut self, msg_id: &MessageId) -> Option<MessageInfo> {
        self.message_info.remove(msg_id)
    }
}
