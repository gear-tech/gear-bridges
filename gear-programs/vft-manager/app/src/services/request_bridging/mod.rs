//! Gear -> ethereum bridging request entrypoint of `VFTManager` service.

use sails_rs::{gstd::ExecContext, prelude::*};

use super::{error::Error, Event, TokenSupply, VftManager};

mod bridge_builtin_operations;
mod msg_tracker;
mod token_operations;

use bridge_builtin_operations::Payload;
use msg_tracker::{msg_tracker_mut, MessageStatus, TxDetails};

pub use msg_tracker::{msg_tracker_state, MessageInfo as MsgTrackerMessageInfo};

/// Initialize state that's used by this entrypoint.
pub fn seed() {
    msg_tracker::init();
}

/// Lock/burn `vft` tokens (specific operation depends on the token supply type) and send
/// request to the bridge built-in actor. If request is failed then tokens will be refunded back
/// to the sender.
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
        token_supply: supply_type,
    };

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToDepositTokens,
        transaction_details,
    );

    match supply_type {
        TokenSupply::Ethereum => {
            token_operations::burn(vara_token_id, sender, amount, config, msg_id)
                .await
                .expect("Failed to burn tokens");
        }
        TokenSupply::Gear => {
            token_operations::lock(vara_token_id, sender, amount, config, msg_id)
                .await
                .expect("Failed to lock tokens");
        }
    }

    let payload = Payload {
        receiver,
        token_id: eth_token_id,
        amount,
    };

    msg_tracker_mut().update_message_status(msg_id, MessageStatus::SendingMessageToBridgeBuiltin);

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
            msg_tracker_mut()
                .update_message_status(msg_id, MessageStatus::SendingMessageToReturnTokens);

            match supply_type {
                TokenSupply::Ethereum => {
                    token_operations::mint(vara_token_id, sender, amount, config, msg_id)
                        .await
                        .expect("Failed to mint tokens");
                }
                TokenSupply::Gear => {
                    token_operations::unlock(vara_token_id, sender, amount, config, msg_id)
                        .await
                        .expect("Failed to unlock tokens");
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

/// Try to execute failed request again. It can be used to return funds back to the user when
/// the [request_bridging] execution unexpectedly finished (due to the insufficient gas amount
/// or some other temporary error) but funds have already been locked/burnt.
///
/// This function can return funds back to the user in the following scenarios:
/// - Token lock/burn is complete but message to the built-in actor haven't been sent yet. It can happen if
///     user haven't attached gas enough to process the message further after the first `wake` or if network
///     is loaded and timeout we've set to the reply is expired.
/// - Message to the built-in actor have returned error but token refund message haven't been sent yet. It
///     can happen if user haven't attached gas enough to process the message further after the second `wake`
///     or if network is loaded and timeout we've set to the reply is expired.
/// - Token refund message have been sent but it have failed. This case should be practically impossible
///     due to the invariants that `vft-manager` provides but left just in case.
pub async fn handle_interrupted_transfer<T: ExecContext>(
    service: &mut VftManager<T>,
    msg_id: MessageId,
) -> Result<(), Error> {
    let config = service.config();

    let msg_info = msg_tracker_mut()
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
        | MessageStatus::BridgeResponseReceived(None)
        | MessageStatus::TokensReturnComplete(false) => {
            msg_tracker_mut()
                .update_message_status(msg_id, MessageStatus::SendingMessageToReturnTokens);

            match token_supply {
                TokenSupply::Ethereum => {
                    token_operations::mint(vara_token_id, sender, amount, config, msg_id).await?;
                }
                TokenSupply::Gear => {
                    token_operations::unlock(vara_token_id, sender, amount, config, msg_id).await?;
                }
            }

            Ok(())
        }
        _ => {
            panic!("Unexpected status or transaction completed.")
        }
    }
}
