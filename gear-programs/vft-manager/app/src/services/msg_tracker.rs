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
    RequestBridging {
        vara_token_id: ActorId,
        sender: ActorId,
        amount: U256,
        receiver: H160,
    },
    SubmitReceipt {
        vara_token_id: ActorId,
        receiver: ActorId,
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

    pub fn check_burn_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenBurnCompleted(true) => Ok(()),
                MessageStatus::TokenBurnCompleted(false) => {
                    self.message_info.remove(msg_id);
                    Err(Error::BurnTokensFailed)
                }
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }

    pub fn check_lock_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenLockCompleted(true) => Ok(()),
                MessageStatus::TokenLockCompleted(false) => {
                    self.message_info.remove(msg_id);
                    Err(Error::LockTokensFailed)
                }
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }

    pub fn check_mint_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenMintCompleted => Ok(()),
                MessageStatus::MintTokensStep => Err(Error::MessageFailed),
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }

    pub fn check_unlock_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenUnlockCompleted => Ok(()),
                MessageStatus::UnlockTokensStep => Err(Error::MessageFailed),
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

    // Burn tokens statuses
    SendingMessageToBurnTokens,
    TokenBurnCompleted(bool),
    WaitingReplyFromBurn,

    // Mint tokens status
    SendingMessageToMintTokens,
    TokenMintCompleted,
    WaitingReplyFromMint,
    MintTokensStep,

    // Lock tokens statuses
    SendingMessageToLockTokens,
    TokenLockCompleted(bool),
    WaitingReplyFromLock,

    // Unlock tokens status
    SendingMessageToUnlockTokens,
    TokenUnlockCompleted,
    WaitingReplyFromUnlock,
    UnlockTokensStep,

    MessageProcessedWithSuccess(U256),
}
