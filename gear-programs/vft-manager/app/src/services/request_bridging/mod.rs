use sails_rs::{gstd::ExecContext, prelude::*};

use super::{error::Error, Event, TokenSupply, VftManager};

mod bridge_builtin_operations;
mod msg_tracker;
mod token_operations;
mod utils;

use bridge_builtin_operations::Payload;

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

    if gstd::exec::gas_available()
        < config.gas_for_token_ops
            + config.gas_to_send_request_to_builtin
            + config.gas_for_request_bridging
            + 3 * config.gas_for_reply_deposit
    {
        panic!("Please attach more gas");
    }

    match supply_type {
        TokenSupply::Ethereum => {
            token_operations::burn(vara_token_id, sender, receiver, amount, config, msg_id).await?;
        }
        TokenSupply::Gear => {
            token_operations::lock(vara_token_id, sender, amount, receiver, config, msg_id).await?;
        }
    }

    let payload = Payload {
        receiver,
        token_id: eth_token_id,
        amount,
    };
    let nonce = match bridge_builtin_operations::send_message_to_bridge_builtin(
        state.gear_bridge_builtin,
        state.erc20_manager_address,
        payload,
        config,
        msg_id,
    )
    .await
    {
        Ok(nonce) => nonce,
        Err(e) => {
            // In case of failure, mint tokens back to the sender
            // TODO: Or unlock.
            //token_operations::mint(vara_token_id, sender, amount, config, msg_id).await?;
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
