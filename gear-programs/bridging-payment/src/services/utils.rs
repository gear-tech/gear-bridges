use super::{
    error::Error,
    msg_tracker::{msg_tracker_mut, MessageStatus},
    vft::vft::io as vft_io,
    vft_gateway::vft_gateway::io as vft_gateway_io,
    vft_gateway::Error as VftGatewayError,
};
use sails_rs::calls::ActionIo;
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
            .expect("Unexpected: msg info does not exist");

        match &msg_info.status {
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
            MessageStatus::SendingMessageToTransferTokensBack => {
                // If still sending, transition to `WaitingReplyFromTokenTransferBack`.
                msg_tracker.update_message_status(
                    msg_id,
                    MessageStatus::WaitingReplyFromTokenTransferBack,
                );
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
    reply_timeout: u32,
    msg_id: MessageId,
) -> Result<(), Error> {
    gstd::msg::send_bytes_with_gas_for_reply(destination, message, gas_to_send, 0, gas_deposit)
        .map_err(|_| Error::SendFailure)?
        .up_to(Some(reply_timeout))
        .map_err(|_| Error::ReplyTimeout)?
        .handle_reply(move || handle_reply_hook(msg_id))
        .map_err(|_| Error::ReplyHook)?
        .await
        .map_err(|_| Error::ReplyFailure)?;
    Ok(())
}

fn handle_reply_hook(msg_id: MessageId) {
    let msg_tracker = msg_tracker_mut();

    let msg_info = msg_tracker
        .get_message_info(&msg_id)
        .expect("Unexpected: msg info does not exist");
    let reply_bytes = gstd::msg::load_bytes().expect("Unable to load bytes");

    match msg_info.status {
        MessageStatus::SendingMessageToTransferTokens => {
            match decode_transfer_reply(&reply_bytes) {
                Ok(reply) => {
                    msg_tracker.update_message_status(
                        msg_id,
                        MessageStatus::TokenTransferCompleted(reply),
                    );
                }
                Err(_) => {
                    msg_tracker.remove_message_info(&msg_id);
                }
            };
        }
        MessageStatus::WaitingReplyFromTokenTransfer => {
            let reply = decode_transfer_reply(&reply_bytes).unwrap_or(false);
            if reply {
                msg_tracker.update_message_status(msg_id, MessageStatus::MessageToGatewayStep);
            } else {
                msg_tracker.remove_message_info(&msg_id);
            }
        }
        MessageStatus::SendingMessageToGateway => {
            let reply = decode_vft_gateway_reply(&reply_bytes);
            match reply {
                Ok(Ok((nonce, eth_token_id))) => {
                    msg_tracker.update_message_status(
                        msg_id,
                        MessageStatus::GatewayMessageProcessingCompleted((nonce, eth_token_id)),
                    );
                }
                Ok(Err(error)) => {
                    match error {
                        VftGatewayError::BurnTokensFailed | VftGatewayError::MessageFailed => {
                            msg_tracker
                                .update_message_status(msg_id, MessageStatus::ReturnTokensBackStep);
                        }
                        // retry tx
                        _ => msg_tracker
                            .update_message_status(msg_id, MessageStatus::MessageToGatewayStep),
                    }
                }
                Err(_) => {
                    msg_tracker.update_message_status(msg_id, MessageStatus::MessageToGatewayStep);
                }
            };
        }
        MessageStatus::WaitingReplyFromGateway => {
            let reply = decode_vft_gateway_reply(&reply_bytes);
            match reply {
                Ok(Ok((nonce, eth_token_id))) => {
                    msg_tracker.update_message_status(
                        msg_id,
                        MessageStatus::MessageProcessedWithSuccess((nonce, eth_token_id)),
                    );
                }
                _ => {
                    msg_tracker.update_message_status(msg_id, MessageStatus::ReturnTokensBackStep);
                }
            };
        }
        MessageStatus::SendingMessageToTransferTokensBack
        | MessageStatus::WaitingReplyFromTokenTransferBack => {
            match decode_transfer_reply(&reply_bytes) {
                Ok(true) => {
                    msg_tracker
                        .update_message_status(msg_id, MessageStatus::TokenTransferBackCompleted);
                }
                Err(_) | Ok(false) => {
                    // retry tx
                    msg_tracker.update_message_status(msg_id, MessageStatus::ReturnTokensBackStep);
                }
            };
        }
        _ => {}
    };
}

fn decode_transfer_reply(bytes: &[u8]) -> Result<bool, Error> {
    vft_io::TransferFrom::decode_reply(bytes).map_err(|_| Error::TransferTokensDecode)
}

fn decode_vft_gateway_reply(bytes: &[u8]) -> Result<Result<(U256, H160), VftGatewayError>, Error> {
    vft_gateway_io::TransferVaraToEth::decode_reply(bytes)
        .map_err(|_| Error::RequestToGateWayDecode)
}
