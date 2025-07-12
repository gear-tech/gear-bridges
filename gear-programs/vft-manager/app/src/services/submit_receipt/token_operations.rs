use crate::services::{Config, Error};
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

trait Params {
    fn receiver(&self) -> ActorId;
    fn amount(&self) -> U256;
}

impl Params for (ActorId, U256) {
    fn receiver(&self) -> ActorId {
        self.0
    }
    fn amount(&self) -> U256 {
        self.1
    }
}

impl Params for (ActorId, ActorId, U256) {
    fn receiver(&self) -> ActorId {
        self.1
    }
    fn amount(&self) -> U256 {
        self.2
    }
}

struct TxDetails {
    slot: u64,
    transaction_index: u64,
    erc20_sender: H160,
    receiver: ActorId,
    token_id: ActorId,
    amount: U256,
}

async fn send<Action>(
    slot: u64,
    transaction_index: u64,
    erc20_sender: H160,
    token_id: ActorId,
    params: &Action::Params,
    config: &Config,
) -> Result<(), Error>
where
    Action: ActionIo,
    Action::Reply: Reply,
    Action::Params: Params,
{
    let payload = Action::encode_call(params);

    let tx_details = TxDetails {
        slot,
        transaction_index,
        erc20_sender,
        receiver: params.receiver(),
        token_id,
        amount: params.amount(),
    };

    // We don't need to send the message with the fixed limit of gas.
    // If there is not enough gas for execution then the VFT-program will exit because of
    // the out of gas and hence its state will be reverted. That means that no tokens will be
    // minted/transferred and moreover our reply hook will not get ever executed.
    gstd::msg::send_bytes_for_reply(token_id, payload, 0, config.gas_for_reply_deposit)
        .map_err(|_| Error::SendFailure)?
        .up_to(Some(config.reply_timeout))
        .map_err(|_| Error::ReplyTimeout)?
        .handle_reply(move || handle_reply::<Action>(tx_details))
        .map_err(|_| Error::ReplyHook)?
        .await
        .map_err(|e| match e {
            GStdError::Timeout(..) => Error::ReplyTimeout,
            _ => Error::Internal(format!("{e:?}")),
        })
        .and_then(|_| {
            // Careful: this might never run in case of OutOfGas error.
            // To guarantee atomicity, this logic is being moved inside the `handle_reply` hook.
            let key = (slot, transaction_index);
            match super::reply_statuses_mut().remove(&key) {
                Some(status) => status.clone(),
                None => {
                    unreachable!("Status always set if a VFT invocation was successful");
                }
            }
        })
}

fn handle_reply<Action>(data: TxDetails)
where
    Action: ActionIo,
    Action::Reply: Reply,
    Action::Params: Params,
{
    let TxDetails {
        slot,
        transaction_index,
        erc20_sender,
        receiver,
        token_id,
        amount,
    } = data;

    let status = {
        || {
            // If a message is a reply, the reply code is always present.
            let reply_code = msg::reply_code().map_err(|e| Error::NoReplyCode(format!("{e:?}")))?;

            if let ReplyCode::Error(_reason) = reply_code {
                // The actual error will be available in the original (awaiting) message.
                return Err(Error::ReplyFailure);
            }
            // Under normal circumstances (if the reply hook has been benchmarked correctly) this
            // should never result in an error.
            let reply_bytes =
                msg::load_bytes().map_err(|e| Error::GasForReplyTooLow(format!("{e:?}")))?;

            // At this point we assume the reply is a legit VFT reply and try to treat it as such.
            // An error can stem from a malformed reply payload. If occurs, we'll want to report it
            // to the calling message because it is not detectable otherwise.
            let reply = Action::decode_reply(&reply_bytes)
                .map_err(|e| Error::Internal(format!("{e:?}")))?;

            // Check if the reply is as expected.
            reply.check()
        }
    }();

    let reply_statuses = super::reply_statuses_mut();
    reply_statuses.insert((slot, transaction_index), status.clone());

    // An error means one of the two things:
    // - the VFT invocation failed
    // - we couldn't read the reply payload
    // In either case we can't proceed with the event emission and transaction history update.
    if status.is_err() {
        return;
    }

    let transactions = super::transactions_mut();
    if super::TX_HISTORY_DEPTH <= transactions.len() {
        transactions.pop_first();
    }

    transactions.insert((slot, transaction_index));

    emit_event(receiver, erc20_sender, amount, token_id);
}

/// Mint `amount` tokens into the `receiver` address.
///
/// It will send `Mint` call to the corresponding `VFT` program and
/// asynchronously wait for the reply.
pub async fn mint(
    slot: u64,
    transaction_index: u64,
    erc20_sender: H160,
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    send::<Mint>(
        slot,
        transaction_index,
        erc20_sender,
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
/// asynchronously wait for the reply.
pub async fn unlock(
    slot: u64,
    transaction_index: u64,
    erc20_sender: H160,
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    let sender = Syscall::program_id();

    send::<TransferFrom>(
        slot,
        transaction_index,
        erc20_sender,
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

#[allow(unused_variables)]
fn emit_event(to: ActorId, from: H160, amount: U256, token: ActorId) {
    #[cfg(target_arch = "wasm32")]
    {
        const ROUTE: [u8; 11usize] = [
            40u8, 86u8, 102u8, 116u8, 77u8, 97u8, 110u8, 97u8, 103u8, 101u8, 114u8,
        ];
        sails_rs::gstd::__emit_event_with_route(
            &ROUTE,
            crate::services::Event::BridgingAccepted {
                to,
                from,
                amount,
                token,
            },
        );
    }
}
