use super::{error::Error, vft::vft::io as vft_io, Config, MSG_TRACKER};
use gstd::{
    debug, msg,
    prelude::collections::{BTreeMap, HashMap},
};
use sails_rs::{calls::ActionIo, prelude::*};

#[derive(Default)]
pub struct MessageTracker {
    pub message_info: HashMap<MessageId, MessageInfo>,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct MessageInfo {
    pub status: MessageStatus,
    pub details: TransactionDetails,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum TransactionDetails {
    Transfer {
        sender: ActorId,
        receiver: ActorId,
        amount: U256,
        token_id: ActorId,
    },
    SendMessageToGateway {
        sender: ActorId,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
        attached_value: u128,
    },
}

#[derive(Debug, Clone, PartialEq, Encode, Decode, TypeInfo)]
pub enum MessageStatus {
    // Transfer tokens statuses
    SendingMessageToTransferTokens,
    TokenTransferCompleted(bool),
    WaitingReplyFromTokenTransfer,

    // Send message to gateway statuses
    SendingMessageToGateway,
    GatewayMessageProcessingCompleted(Option<(U256, H160)>),
    WaitingReplyFromGateway,
    MessageToGatewayStep,
}

impl MessageTracker {
    pub fn insert_message_info(
        &mut self,
        msg_id: MessageId,
        status: MessageStatus,
        details: TransactionDetails,
    ) {
        self.message_info
            .insert(msg_id, MessageInfo { status, details });
    }

    pub fn update_message_status(&mut self, msg_id: MessageId, status: MessageStatus) {
        if let Some(info) = self.message_info.get_mut(&msg_id) {
            info.status = status;
        }
    }

    pub fn get_message_info(&self, msg_id: &MessageId) -> Option<&MessageInfo> {
        self.message_info.get(msg_id)
    }

    pub fn remove_message_info(&mut self, msg_id: &MessageId) -> Option<MessageInfo> {
        self.message_info.remove(msg_id)
    }
}

pub fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe { MSG_TRACKER.as_mut().expect("Pair::seed() should be called") }
}
