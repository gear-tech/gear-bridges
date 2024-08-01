use gstd::prelude::collections::HashMap;
use sails_rs::prelude::*;

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
pub struct TransactionDetails {
    vara_token_id: ActorId,
    sender: ActorId,
    amount: U256,
    receiver: H160,
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

#[derive(Debug, Clone, PartialEq, Encode, Decode, TypeInfo)]
pub enum MessageStatus {
    // Burn tokens statuses
    SendingMessageToBurnTokens,
    TokenBurnCompleted(bool),
    WaitingReplyFromBurn,

    // Send message to bridge builtin
    SendingMessageToBridgeBuiltin,
    BridgeResponseReceived(Option<U256>),
    WaitingReplyFromBuiltin,
    BridgeBuiltinStep,

    // Mint tokens status
    SendingMessageToMintTokens,
    TokenMintCompleted(bool),
    WaitingReplyFromMint,
    MintTokensStep,
}

impl TransactionDetails {
    pub fn new(sender: ActorId, amount: U256, receiver: H160, vara_token_id: ActorId) -> Self {
        Self {
            vara_token_id,
            sender,
            amount,
            receiver,
        }
    }

    pub fn data(&self) -> (ActorId, U256, H160, ActorId) {
        (self.sender, self.amount, self.receiver, self.vara_token_id)
    }
}
