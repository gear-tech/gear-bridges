use super::{
    error::Error,
    msg_tracker::{msg_tracker_mut, MessageStatus},
    vft_treasury::vft_treasury::io as vft_treasury_io,
};
use sails_rs::calls::ActionIo;
use sails_rs::prelude::*;

macro_rules! maybe_event_or_panic_async {
    ($self:expr, $expr:expr) => {{
        let result: Result<Option<BridgingPaymentEvents>, Error> = $expr().await;
        match result {
            Ok(Some(value)) => {
                if let Err(e) = $self.notify_on(value) {
                    panic!("Error in depositing events: {:?}", e);
                }
            }
            Ok(None) => {}
            Err(e) => {
                panic!("Message processing failed with error: {:?}", e);
            }
        }
    }};
}

pub(crate) use maybe_event_or_panic_async;

pub fn set_critical_hook(msg_id: MessageId) {
    gstd::critical::set_hook(move || {
        let msg_tracker = msg_tracker_mut();
        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg info does not exist");

        #[allow(clippy::single_match)]
        match &msg_info.status {
            MessageStatus::SendingMessageToTreasury => {
                // If still sending, transition to `WaitingReplyFromGateway`.
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromTreasury);
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
        MessageStatus::SendingMessageToTreasury | MessageStatus::WaitingReplyFromTreasury => {
            let reply = decode_vft_treasury_reply(&reply_bytes);
            match reply {
                Ok(()) => {
                    msg_tracker.update_message_status(
                        msg_id,
                        MessageStatus::TreasuryMessageProcessingCompleted,
                    );
                }
                Err(_) => {
                    msg_tracker.update_message_status(msg_id, MessageStatus::ProcessRefund);
                }
            };
        }
        _ => {}
    };
}

fn decode_vft_treasury_reply(bytes: &[u8]) -> Result<(), Error> {
    vft_treasury_io::Deposit::decode_reply(bytes).map_err(|_| Error::RequestToTreasuryDecode)
}
