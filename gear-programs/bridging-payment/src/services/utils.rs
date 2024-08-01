use super::{
    error::Error,
    handle_refund,
    msg_tracker::{msg_tracker_mut, MessageStatus, TransactionDetails},
};
use sails_rs::prelude::*;

#[macro_export]
macro_rules! event_or_panic_async {
    ($self:expr, $expr:expr) => {{
        let result: Result<BridgingPaymentEvents, Error> = $expr().await;
        match result {
            Ok(value) => {
                if let Err(e) = $self.notify_on(value) {
                    panic!("Error in depositing events: {:?}", e);
                }
            }
            Err(e) => {
                gstd::debug!("Error {:?}", e);
                panic!("Message processing failed with error: {:?}", e);
            }
        }
    }};
}

pub fn set_critical_hook(msg_id: MessageId) {
    gstd::critical::set_hook(move || {
        let msg_tracker = msg_tracker_mut();
        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg status does not exist");

        match msg_info.status {
            MessageStatus::SendingMessageToTransferTokens => {
                // If still sending, transition to `WaitingReplyFromTokenTransfer`.
                msg_tracker
                    .update_message_status(msg_id, MessageStatus::WaitingReplyFromTokenTransfer);
            }
            MessageStatus::TokenTransferCompleted(true) => {
                // If the token transfer is successful, continue to the gateway step.
                msg_tracker.update_message_status(msg_id, MessageStatus::MessageToGatewayStep);
            }
            MessageStatus::TokenTransferCompleted(false) => {
                // If the token transfer fails, cancel the transaction.
                msg_tracker.remove_message_info(&msg_id);
            }
            MessageStatus::SendingMessageToGateway => {
                // If still sending, transition to `WaitingReplyFromGateway`.
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromGateway);
            }
            MessageStatus::GatewayMessageProcessingCompleted(None) => {
                let (sender, attached_value) = if let TransactionDetails::SendMessageToGateway {
                    sender,
                    attached_value,
                    ..
                } = msg_info.details
                {
                    (sender, attached_value)
                } else {
                    panic!("Unexpected tx details")
                };
                handle_refund(sender, attached_value);
            }
            _ => {}
        };
    });
}

pub async fn send_message_with_gas_for_reply(
    destination: ActorId,
    message: Vec<u8>,
    gas_to_send: u64,
    gas_deposit: u64,
) -> Result<Vec<u8>, Error> {
    gstd::msg::send_bytes_with_gas_for_reply(
        destination.into(),
        message,
        gas_to_send,
        0,
        gas_deposit,
    )
    .map_err(|_| Error::SendError)?
    .await
    .map_err(|_| Error::ReplyError)
}

fn process_reply<T>(
    msg_id: MessageId,
    reply: T,
    completed_status: fn(T) -> MessageStatus,
) -> Result<(), Error> {
    msg_tracker_mut().update_message_status(msg_id, completed_status(reply));
    Ok(())
}
