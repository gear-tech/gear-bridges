use super::{error::Error, MSG_TRACKER};
use gstd::prelude::collections::HashMap;
use sails_rs::prelude::*;

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
pub struct TransactionDetails {
    pub sender: ActorId,
    pub vara_token_id: ActorId,
    pub amount: U256,
    pub receiver: H160,
    pub attached_value: u128,
}

#[derive(Debug, PartialEq, Encode, Decode, TypeInfo, Clone)]
pub enum MessageStatus {
    SendingMessageToTreasury,
    WaitingReplyFromTreasury,
    ProcessRefund,
    TreasuryMessageProcessingCompleted,
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

    pub fn check_vft_treasury_reply(&mut self, msg_id: &MessageId) -> Result<(), Error> {
        if let Some(info) = self.message_info.get(msg_id) {
            match info.status {
                MessageStatus::TreasuryMessageProcessingCompleted => {
                    self.remove_message_info(msg_id);
                    Ok(())
                }
                MessageStatus::ProcessRefund | MessageStatus::SendingMessageToTreasury => {
                    Err(Error::TreasuryMessageProcessingFailed)
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
