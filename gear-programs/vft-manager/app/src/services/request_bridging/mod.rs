use sails_rs::{gstd::ExecContext, prelude::*};

use super::{error::Error, Event, TokenSupply, VftManager};

mod bridge_builtin_operations;
mod msg_tracker;
mod token_operations;
mod utils;

use bridge_builtin_operations::Payload;
use msg_tracker::{msg_tracker_mut, MessageStatus, TxDetails};

pub use msg_tracker::{msg_tracker_state, MessageInfo as MsgTrackerMessageInfo};

pub fn seed() {
    msg_tracker::init();
}

pub async fn request_bridging<T: ExecContext>(
    service: &mut VftManager<T>,
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
) -> Result<(U256, H160), Error> {
    let state = service.state();
    let msg_id = gstd::msg::id();
    let eth_token_id = service.state().token_map.get_eth_token_id(&vara_token_id)?;
    let supply_type = service.state().token_map.get_supply_type(&vara_token_id)?;
    let config = service.config();

    let transaction_details = TxDetails {
        vara_token_id,
        sender,
        amount,
        receiver,
        token_supply: TokenSupply::Ethereum,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToDepositTokens,
        transaction_details,
    );
    set_critical_hook(msg_id);

    match supply_type {
        TokenSupply::Ethereum => {
            token_operations::burn(vara_token_id, sender, amount, config, msg_id).await?;
        }
        TokenSupply::Gear => {
            token_operations::lock(vara_token_id, sender, amount, config, msg_id).await?;
        }
    }

    let payload = Payload {
        receiver,
        token_id: eth_token_id,
        amount,
    };

    let bridge_builtin_reply = bridge_builtin_operations::send_message_to_bridge_builtin(
        state.gear_bridge_builtin,
        state.erc20_manager_address,
        payload,
        config,
        msg_id,
    )
    .await;

    let nonce = match bridge_builtin_reply {
        Ok(nonce) => nonce,
        Err(e) => {
            match supply_type {
                TokenSupply::Ethereum => {
                    token_operations::mint(vara_token_id, sender, amount, config, msg_id).await?;
                }
                TokenSupply::Gear => {
                    token_operations::unlock(vara_token_id, sender, amount, config, msg_id).await?;
                }
            }

            return Err(e);
        }
    };

    service
        .notify_on(Event::BridgingRequested {
            nonce,
            vara_token_id,
            amount,
            sender,
            receiver,
        })
        .expect("Failed to emit event");

    Ok((nonce, eth_token_id))
}

pub async fn handle_interrupted_transfer<T: ExecContext>(
    service: &mut VftManager<T>,
    msg_id: MessageId,
) -> Result<(U256, H160), Error> {
    let config = service.config();
    let msg_tracker = msg_tracker::msg_tracker_mut();

    let msg_info = msg_tracker
        .get_message_info(&msg_id)
        .expect("Unexpected: msg status does not exist");

    let TxDetails {
        vara_token_id,
        sender,
        amount,
        receiver: _,
        token_supply,
    } = msg_info.details;

    match msg_info.status {
        MessageStatus::TokenDepositCompleted(true)
        | MessageStatus::SendingMessageToBridgeBuiltin
        | MessageStatus::SendingMessageToReturnTokens
        | MessageStatus::TokensReturnComplete(false)
        | MessageStatus::BridgeResponseReceived(None) => {
            set_critical_hook(msg_id);

            match token_supply {
                TokenSupply::Ethereum => {
                    token_operations::mint(vara_token_id, sender, amount, config, msg_id).await?;
                }
                TokenSupply::Gear => {
                    token_operations::unlock(vara_token_id, sender, amount, config, msg_id).await?;
                }
            }

            Err(Error::TokensRefunded)
        }
        MessageStatus::TokenDepositCompleted(false)
        | MessageStatus::SendingMessageToDepositTokens
        | MessageStatus::WaitingReplyFromTokenDepositMessage
        | MessageStatus::WaitingReplyFromBuiltin
        | MessageStatus::WaitingReplyFromTokenReturnMessage
        | MessageStatus::BridgeResponseReceived(Some(_))
        | MessageStatus::TokensReturnComplete(true) => {
            panic!("Unexpected status or transaction completed.")
        }
    }
}

fn set_critical_hook(msg_id: MessageId) {
    gstd::critical::set_hook(move || {
        let msg_tracker = msg_tracker_mut();
        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg info does not exist");

        match msg_info.status {
            MessageStatus::SendingMessageToDepositTokens => {
                msg_tracker.update_message_status(
                    msg_id,
                    MessageStatus::WaitingReplyFromTokenDepositMessage,
                );
            }
            MessageStatus::SendingMessageToBridgeBuiltin => {
                msg_tracker.update_message_status(msg_id, MessageStatus::WaitingReplyFromBuiltin);
            }
            MessageStatus::SendingMessageToReturnTokens => {
                msg_tracker.update_message_status(
                    msg_id,
                    MessageStatus::WaitingReplyFromTokenReturnMessage,
                );
            }
            _ => {}
        };
    });
}
