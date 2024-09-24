//! # Message Tracker
//!
//! This module tracks lifetime of a message: handle reply,
//! execute next step, handles an error appropriatly.

use super::Error;
use gstd::{prelude::collections::HashMap, MessageId};
use sails_rs::prelude::*;

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
pub enum TxDetails {
    DepositToTreasury {
        vara_token_id: ActorId,
        eth_token_id: H160,
        sender: ActorId,
        amount: U256,
        receiver: H160,
    },

    WithdrawFromTreasury {
        vara_token_id: ActorId,
        eth_token_id: H160,
        recepient: ActorId,
        amount: U256,
    },
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

    pub fn update_message_status(
        &mut self,
        msg_id: MessageId,
        status: MessageStatus,
    ) -> Result<(), Error> {
        self.message_info
            .get_mut(&msg_id)
            .ok_or(Error::MessageNotFound)?
            .status = status;
        Ok(())
    }

    pub fn get_message_info(&self, msg_id: &MessageId) -> Option<&MessageInfo> {
        self.message_info.get(msg_id)
    }

    pub fn remove_message_info(&mut self, msg_id: &MessageId) -> Option<MessageInfo> {
        self.message_info.remove(msg_id)
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

    pub fn check_transfer_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenTransferCompleted(true) => Ok(()),
                MessageStatus::TokenTransferCompleted(false) => {
                    self.message_info.remove(msg_id);
                    Err(Error::TransferFailed)
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
    // Transfer tokens statuses
    SendingMessageToTransferTokens,
    TokenTransferCompleted(bool),
    WaitingReplyFromTransfer,

    // Send message to bridge builtin
    SendingMessageToBridgeBuiltin,
    BridgeResponseReceived(Option<U256>),
    WaitingReplyFromBuiltin,
    BridgeBuiltinStep,

    MessageProcessedWithSuccess(U256),
}
