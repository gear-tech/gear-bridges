use super::super::TokenSupply;
use gstd::{prelude::collections::HashMap, static_mut, static_ref, MessageId};
use sails_rs::prelude::*;

static mut MSG_TRACKER: Option<MessageTracker> = None;

/// State machine which tracks state of each message that was submitted into
/// `request_bridging` method.
#[derive(Default, Debug)]
pub struct MessageTracker {
    /// Message states.
    pub message_info: HashMap<MessageId, MessageInfo>,
    pub active_count: u64,
}

/// Entry for a single message in [MessageTracker].
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct MessageInfo {
    /// State of the message.
    pub status: MessageStatus,
    /// Request details.
    pub details: TxDetails,
}

/// Details about a request associated with a message stored in [MessageTracker].
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct TxDetails {
    /// Address of the `VFT` token which is being bridged.
    pub vara_token_id: ActorId,
    /// Original `VFT` token owner.
    pub sender: ActorId,
    /// Bridged tokens amount.
    pub amount: U256,
    /// `ERC20` token receiver on Ethereum.
    pub receiver: H160,
    /// [TokenSupply] type of the token being bridged.
    pub token_supply: TokenSupply,
}

/// State in which message processing can be.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub enum MessageStatus {
    /// Message to deposit tokens is sent.
    SendingMessageToDepositTokens,
    /// Reply is received for a token deposit message.
    TokenDepositCompleted(bool),

    /// Message to the `pallet-gear-eth-bridge` is sent. This status is also the
    /// fail-closed quarantine state when the outcome is ambiguous.
    SendingMessageToBridgeBuiltin,
    /// Reply is received for a message to the `pallet-gear-eth-bridge`.
    BridgeResponseReceived(Option<(U256, H256, u64)>),

    /// Message to refund tokens is sent.
    SendingMessageToReturnTokens,
    /// Reply is received for a token refund message.
    TokensReturnComplete(bool),
}

impl MessageStatus {
    pub(super) fn terminal(&self) -> bool {
        matches!(
            self,
            Self::TokenDepositCompleted(false)
                | Self::BridgeResponseReceived(Some(_))
                | Self::TokensReturnComplete(true)
        )
    }
}

/// Initialize global state of the message tracker.
pub fn init() {
    unsafe { MSG_TRACKER = Some(MessageTracker::default()) }
}

/// Get reference to a global message tracker.
pub fn msg_tracker_ref() -> &'static MessageTracker {
    unsafe { static_ref!(MSG_TRACKER).as_ref() }.expect("VftManager::seed() should be called")
}

/// Get mutable reference to a global message tracker.
pub fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe { static_mut!(MSG_TRACKER).as_mut() }.expect("VftManager::seed() should be called")
}

impl MessageTracker {
    /// Start tracking state of the message.
    pub fn insert_message_info(
        &mut self,
        msg_id: MessageId,
        status: MessageStatus,
        details: TxDetails,
    ) {
        assert!(
            !self.message_info.contains_key(&msg_id),
            "Request already tracked"
        );
        if !status.terminal() {
            self.active_count = self
                .active_count
                .checked_add(1)
                .expect("Tracker count overflow");
        }
        self.message_info
            .insert(msg_id, MessageInfo { status, details });
    }

    /// Drive state machine further for a given `msg_id`.
    pub fn update_message_status(&mut self, msg_id: MessageId, status: MessageStatus) {
        if let Some(info) = self.message_info.get_mut(&msg_id) {
            match (info.status.terminal(), status.terminal()) {
                (false, true) => {
                    self.active_count = self
                        .active_count
                        .checked_sub(1)
                        .expect("Tracker count underflow")
                }
                (true, false) => {
                    self.active_count = self
                        .active_count
                        .checked_add(1)
                        .expect("Tracker count overflow")
                }
                _ => {}
            }
            info.status = status;
        }
    }

    /// Get current state of the tracked message. Will return `None` if message isn't found.
    pub fn get_message_info(&self, msg_id: &MessageId) -> Option<&MessageInfo> {
        self.message_info.get(msg_id)
    }

    /// Stop tracking message state. It will return current state of the target message.
    pub fn remove_message_info(&mut self, msg_id: &MessageId) -> Option<MessageInfo> {
        let removed = self.message_info.remove(msg_id)?;
        if !removed.status.terminal() {
            self.active_count = self
                .active_count
                .checked_sub(1)
                .expect("Tracker count underflow");
        }
        Some(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reconstructed from supplied HTML state-bounds finding, audited 7fe1b032.
    #[test]
    fn active_count_tracks_transitions_not_lifetime_history() {
        let mut tracker = MessageTracker::default();
        let details = TxDetails {
            vara_token_id: ActorId::from([1; 32]),
            sender: ActorId::from([2; 32]),
            amount: U256::one(),
            receiver: H160([3; 20]),
            token_supply: TokenSupply::Ethereum,
        };
        for byte in 1..=128u8 {
            let id = MessageId::from([byte; 32]);
            tracker.insert_message_info(
                id,
                MessageStatus::SendingMessageToDepositTokens,
                details.clone(),
            );
            assert_eq!(tracker.active_count, 1);
            tracker.update_message_status(id, MessageStatus::TokenDepositCompleted(true));
            assert_eq!(tracker.active_count, 1);
            tracker.update_message_status(id, MessageStatus::SendingMessageToReturnTokens);
            tracker.update_message_status(id, MessageStatus::TokensReturnComplete(true));
            tracker.update_message_status(id, MessageStatus::TokensReturnComplete(true));
            assert_eq!(tracker.active_count, 0);
        }
        assert_eq!(tracker.message_info.len(), 128);
        for byte in 1..=128u8 {
            tracker.remove_message_info(&MessageId::from([byte; 32]));
            assert_eq!(tracker.active_count, 0);
        }
    }
}
