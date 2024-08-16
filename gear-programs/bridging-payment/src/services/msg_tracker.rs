use super::{error::Error, MSG_TRACKER};
use gstd::prelude::collections::HashMap;
#[allow(unused_imports)]
use sails_rs::{calls::ActionIo, prelude::*};
#[derive(Default)]
pub struct MessageTracker {
    pub message_info: HashMap<MessageId, MessageInfo>,
}

#[derive(Debug, Encode, Decode, TypeInfo, Clone)]
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

#[derive(Debug, PartialEq, Encode, Decode, TypeInfo, Clone)]
pub enum MessageStatus {
    // Transfer tokens statuses
    SendingMessageToTransferTokens,
    TokenTransferCompleted(bool),
    WaitingReplyFromTokenTransfer,

    // Send message to gateway statuses
    SendingMessageToGateway,
    GatewayMessageProcessingCompleted((U256, H160)),
    WaitingReplyFromGateway,
    MessageToGatewayStep,

    ReturnTokensBackStep,
    SendingMessageToTransferTokensBack,
    WaitingReplyFromTokenTransferBack,
    TokenTransferBackCompleted,

    MessageProcessedWithSuccess((U256, H160)),
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

    pub fn check_transfer_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenTransferCompleted(true) => Ok(()),
                MessageStatus::TokenTransferCompleted(false) => {
                    self.message_info.remove(msg_id);
                    Err(Error::TransferTokensFailed)
                }
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }

    pub fn check_transfer_back_result(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TokenTransferBackCompleted => {
                    self.message_info.remove(msg_id);
                    Ok(())
                }
                MessageStatus::ReturnTokensBackStep => Err(Error::TransferTokensFailed),
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }

    pub fn check_vft_gateway_reply(&mut self, msg_id: &MessageId) -> Result<(U256, H160), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::GatewayMessageProcessingCompleted((nonce, eth_token_id)) => {
                    self.remove_message_info(msg_id);
                    Ok((nonce, eth_token_id))
                }
                MessageStatus::ReturnTokensBackStep | MessageStatus::MessageToGatewayStep => {
                    Err(Error::GatewayMessageProcessingFailed)
                }
                _ => Err(Error::InvalidMessageStatus),
            }
        } else {
            Err(Error::MessageNotFound)
        }
    }
}

pub fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_mut()
            .expect("BridgingPaymentData::seed() should be called")
    }
}

pub fn msg_tracker() -> &'static MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_ref()
            .expect("BridgingPaymentData::seed() should be called")
    }
}
