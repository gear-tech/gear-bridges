use super::super::{Config, Error};
use gstd::msg;
use sails_rs::{calls::ActionIo, prelude::*};
use vft_client::{vft::io::TransferFrom, vft_admin::io::Mint};

trait Reply {
    fn check(&self);
}

impl Reply for () {
    fn check(&self) {}
}

impl Reply for bool {
    fn check(&self) {
        if !*self {
            panic!("Request to transfer tokens failed");
        }
    }
}

async fn send<Action>(
    slot: u64,
    transaction_index: u64,
    token_id: ActorId,
    params: &Action::Params,
    config: &Config,
) -> Result<(), Error>
where
    Action: ActionIo,
    Action::Reply: Reply,
{
    let payload = Action::encode_call(params);

    // We don't need to send the message with the fixed limit of gas.
    // If there is not enough gas for execution then the VFT-program will exit because of
    // the out of gas and hence its state will be reverted. That means that no tokens will be
    // minted/transferred and moreover our reply hook will not get ever executed.
    gstd::msg::send_bytes_for_reply(token_id, payload, 0, config.gas_for_reply_deposit)
        .map_err(|_| Error::SendFailure)?
        .up_to(Some(config.reply_timeout))
        .map_err(|_| Error::ReplyTimeout)?
        .handle_reply(move || handle_reply::<Action>(slot, transaction_index))
        .map_err(|_| Error::ReplyHook)?
        .await
        .map_err(|_| Error::ReplyFailure)?;

    Ok(())
}

fn handle_reply<Action>(slot: u64, transaction_index: u64)
where
    Action: ActionIo,
    Action::Reply: Reply,
{
    let reply_bytes = msg::load_bytes()
        .expect("May fail because of the insufficient gas only but the limit was specified by the caller; qed");
    let reply = Action::decode_reply(&reply_bytes)
        .expect("May fail only if there is no VFT-program at the specified address; qed");

    reply.check();

    // To that point we have a successful response from the VFT and enough gas to save
    // the information about processed Ethereum transaction.

    let transactions = super::transactions_mut();
    if super::TX_HISTORY_DEPTH <= transactions.len() {
        transactions.pop_first();
    }

    transactions.insert((slot, transaction_index));
}

/// Mint `amount` tokens into the `receiver` address.
///
/// It will send `Mint` call to the corresponding `VFT` program and
/// asyncronously wait for the reply.
pub async fn mint(
    slot: u64,
    transaction_index: u64,
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    send::<Mint>(
        slot,
        transaction_index,
        token_id,
        &(receiver, amount),
        config,
    )
    .await
}

/// Transfer `amount` tokens from the current program address to the `receiver` address,
/// effectively unlocking them.
///
/// It will send `TransferFrom` call to the corresponding `VFT` program and
/// asyncronously wait for the reply.
pub async fn unlock(
    slot: u64,
    transaction_index: u64,
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let sender = gstd::exec::program_id();

    send::<TransferFrom>(
        slot,
        transaction_index,
        token_id,
        &(sender, receiver, amount),
        config,
    )
    .await
}
