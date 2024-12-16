use super::super::{Error, TokenSupply};
use gstd::{prelude::collections::HashMap, MessageId};
use sails_rs::prelude::*;

static mut MSG_TRACKER: Option<MessageTracker> = None;

#[derive(Default, Debug)]
pub struct MessageTracker {
    pub message_info: HashMap<MessageId, MessageInfo>,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct MessageInfo {
    pub status: MessageStatus,
    pub details: TxDetails,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct TxDetails {
    pub vara_token_id: ActorId,
    pub sender: ActorId,
    pub amount: U256,
    pub receiver: H160,
    pub token_supply: TokenSupply,
}

impl MessageTracker {
    pub fn insert_message_info(
        &mut self,
        msg_id: MessageId,
        status: MessageStatus,
        details: TxDetails,
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

    pub fn check_deposit_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenDepositCompleted(true) => Ok(()),
                MessageStatus::TokenDepositCompleted(false) => {
                    self.message_info.remove(msg_id);
                    Err(Error::BurnTokensFailed)
                }
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }

    pub fn check_bridge_reply(&mut self, msg_id: &MessageId) -> Result<U256, Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::BridgeResponseReceived(Some(nonce)) => {
                    self.remove_message_info(msg_id);
                    Ok(nonce)
                }
                MessageStatus::BridgeResponseReceived(None) => {
                    Err(Error::BridgeBuiltinMessageFailed)
                }
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Encode, Decode, TypeInfo)]
pub enum MessageStatus {
    // Send message to bridge builtin
    SendingMessageToBridgeBuiltin,
    BridgeResponseReceived(Option<U256>),
    WaitingReplyFromBuiltin,
    BridgeBuiltinStep,

    // Deposit tokens statuses
    SendingMessageToDepositTokens,
    TokenDepositCompleted(bool),
    WaitingReplyFromTokenDepositMessage,

    // Withdraw tokens statuses.
    SendingMessageToWithdrawTokens,
    TokenWithdrawCompleted,
    WaitingReplyFromTokenWithdrawMessage,
    WithdrawTokensStep,

    MessageProcessedWithSuccess(U256),
}

pub fn init() {
    unsafe { MSG_TRACKER = Some(MessageTracker::default()) }
}

pub fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_mut()
            .expect("VftManager::seed() should be called")
    }
}
