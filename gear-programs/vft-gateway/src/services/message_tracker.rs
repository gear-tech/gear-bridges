use super::Config;
use gstd::{msg, prelude::collections::HashMap, MessageId};
use sails_rtl::prelude::*;

#[derive(Default)]
pub struct MessageTracker {
    msg_id_to_status: HashMap<MessageId, MessageStatus>,
    waiting_reply_to_msg_id: HashMap<MessageId, MessageId>,
    pending_messages: HashMap<MessageId, (MessageStatus, MsgData)>,
}

impl MessageTracker {
    /// Saves the ID of the message that is waiting for a reply and the current message ID.
    /// `waiting_reply_to`: ID of the message waiting for a reply.
    /// `msg_id`: Current message ID.
    pub fn track_waiting_reply(&mut self, waiting_reply_to: MessageId, msg_id: MessageId) {
        self.waiting_reply_to_msg_id
            .insert(waiting_reply_to, msg_id);
    }

    /// Updates the status for a given message ID.
    /// `msg_id`: Message ID to update.
    /// `status`: New status to set for the message.
    pub fn update_message_status(&mut self, msg_id: MessageId, status: MessageStatus) {
        self.msg_id_to_status.insert(msg_id, status);
    }

    /// Removes a message status entry by message ID.
    /// `msg_id`: ID of the message to remove.
    pub fn remove_message_status(&mut self, msg_id: &MessageId) -> Option<MessageStatus> {
        self.msg_id_to_status.remove(msg_id)
    }

    /// Removes an entry from the waiting replies map.
    /// `waiting_reply_to`: ID of the message whose reply-to association needs to be removed.
    pub fn remove_waiting_reply(&mut self, waiting_reply_to: &MessageId) -> Option<MessageId> {
        self.waiting_reply_to_msg_id.remove(waiting_reply_to)
    }

    /// Retrieves a mutable reference to the status of a message by its ID.
    /// `msg_id`: ID of the message for which the status is queried.
    /// Returns: An option containing a mutable reference to the message status if found, or None if the message ID is not in the map.
    pub fn get_message_status_mut(&mut self, msg_id: &MessageId) -> Option<&mut MessageStatus> {
        self.msg_id_to_status.get_mut(msg_id)
    }

    /// Adds data for an interrupted message that needs to be completed later.
    pub fn track_pending_message(
        &mut self,
        msg_id: MessageId,
        msg_status: MessageStatus,
        msg_data: MsgData,
    ) {
        self.pending_messages.insert(msg_id, (msg_status, msg_data));
    }

    /// Removes data for an interrupted message, typically after it has been completed.
    /// `msg_id`: Identifier of the message to remove from the tracking.
    pub fn remove_pending_message(
        &mut self,
        msg_id: &MessageId,
    ) -> Option<(MessageStatus, MsgData)> {
        self.pending_messages.remove(msg_id)
    }
}
#[derive(Debug, Clone, PartialEq, Encode, Decode, TypeInfo)]
pub enum MessageStatus {
    SendingMessageToBurnTokens(MessageId),
    TokenBurnCompleted(bool),
    SendingMessageToBridgeBuiltin(MessageId),
    BridgeResponseReceived(bool, U256),
    SendingMessageToMintTokens(MessageId),
    TokenMintCompleted(bool),
}

#[derive(Clone)]
pub struct MsgData {
    sender: ActorId,
    amount: U256,
    receiver: H160,
    vara_token_id: ActorId,
}

impl MsgData {
    pub fn new(sender: ActorId, amount: U256, receiver: H160, vara_token_id: ActorId) -> Self {
        Self {
            sender,
            amount,
            receiver,
            vara_token_id,
        }
    }

    pub fn data(&self) -> (ActorId, U256, H160, ActorId) {
        (self.sender, self.amount, self.receiver, self.vara_token_id)
    }
}
