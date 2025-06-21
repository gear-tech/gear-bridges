use super::super::{Config, Error};
use gstd::{errors::Error as GStdError, msg};
use sails_rs::{
    calls::{ActionIo, Call},
    gstd::calls::GStdRemoting,
    prelude::*,
};
use vft_client::{vft::io::TransferFrom, vft_admin::io::Mint};
use vft_vara_client::traits::VftNativeExchangeAdmin;

trait Reply {
    fn check(&self) -> Result<(), Error>;
}

impl Reply for () {
    fn check(&self) -> Result<(), Error> {
        Ok(())
    }
}

impl Reply for bool {
    fn check(&self) -> Result<(), Error> {
        self.then_some(()).ok_or(Error::InvalidReply)
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
        .map_err(|e| match e {
            GStdError::Timeout(..) => Error::ReplyTimeout,
            _ => Error::Internal(format!("{e:?}").into_bytes()),
        })
        .and_then(|_| {
            let key = (slot, transaction_index);
            match super::reply_statuses_mut().remove(&key) {
                Some(status) => status.clone(),
                None => {
                    unreachable!("Status always set if a VFT invocation was successful");
                }
            }
        })
}

fn handle_reply<Action>(slot: u64, transaction_index: u64)
where
    Action: ActionIo,
    Action::Reply: Reply,
{
    let status = {
        || {
            let reply_code = msg::reply_code().map_err(|_| Error::ReplyCode)?;
            if let ReplyCode::Error(_reason) = reply_code {
                // An error reply received. The error description will be extracted by the sender.
                // However, we must set status to `Err` here to skip further processing.
                return Err(Error::ReplyFailure);
            }
            let reply_bytes = msg::load_bytes().map_err(|_| Error::GasForReplyTooLow)?;

            // At this point we assume the reply is a legit VFT reply and try to treat it as such.
            let reply =
                Action::decode_reply(&reply_bytes).map_err(|_| Error::Internal(reply_bytes))?;

            reply.check()
        }
    }();

    let reply_statuses = super::reply_statuses_mut();
    reply_statuses.insert((slot, transaction_index), status.clone());

    if status.is_err() {
        return;
    }

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
    let sender = Syscall::program_id();

    send::<TransferFrom>(
        slot,
        transaction_index,
        token_id,
        &(sender, receiver, amount),
        config,
    )
    .await?;

    let remoting = GStdRemoting;
    let mut service = vft_vara_client::VftNativeExchangeAdmin::new(remoting);
    service
        .burn_from(receiver, amount)
        .send_recv(token_id)
        .await
        .map_err(|e| Error::BurnFromFailed(format!("{e:?}")))
}
