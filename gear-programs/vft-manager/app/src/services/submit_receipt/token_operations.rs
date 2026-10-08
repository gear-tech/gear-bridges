use crate::services::{Config, Error};
use gstd::{errors::Error as GStdError, msg};
use sails_rs::{calls::ActionIo, prelude::*};
use vft_client::{vft::io::TransferFrom, vft_admin::io::Mint};
use vft_vara_client::{
    native_escrow::io::{RedeemEscrow, Redemption as GetRedemption},
    PayoutStatus, Redemption,
};

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

#[derive(Clone)]
struct TxDetails {
    slot: u64,
    transaction_index: u64,
    erc20_sender: H160,
    receiver: ActorId,
    token_id: ActorId,
    amount: U256,
    log_index: Option<u64>,
    action: ChildAction,
    terminal: bool,
}

#[derive(Clone, Copy)]
enum ChildAction {
    Mint,
    Unlock,
    Native,
}
static mut CHILDREN: Option<collections::BTreeMap<MessageId, TxDetails>> = None;
fn children() -> &'static mut collections::BTreeMap<MessageId, TxDetails> {
    unsafe { gstd::static_mut!(CHILDREN).get_or_insert_with(collections::BTreeMap::new) }
}

async fn send<Action>(
    (slot, transaction_index): (u64, u64),
    erc20_sender: H160,
    token_id: ActorId,
    params: &Action::Params,
    config: &Config,
    log_index: Option<u64>,
    action: ChildAction,
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
        log_index,
        action,
        terminal: false,
    };

    let reply = gstd::msg::send_bytes_with_gas_for_reply(
        token_id,
        payload,
        config.gas_for_token_ops,
        0,
        config.gas_for_reply_deposit,
    )
    .expect("Dispatch/reply funding failed: roll back the entire child dispatch");
    let child = reply.waiting_reply_to;
    children().insert(child, tx_details);
    if let Some(log_index) = log_index {
        super::record_child((slot, transaction_index), log_index, child);
    }
    let reply = reply
        .up_to(Some(config.reply_timeout))
        .expect("Cannot install child timeout: roll back dispatch")
        .handle_reply(handle_persistent_reply)
        .expect("Cannot install child reply hook: roll back dispatch");
    let awaited = reply.await.map_err(|e| match e {
        GStdError::Timeout(..) => Error::ReplyTimeout(format!("{e:?}")),
        _ => Error::Internal(format!("{e:?}")),
    });

    if let Some(log_index) = log_index {
        return match super::deposit_status((slot, transaction_index), log_index) {
            Some(super::DepositStatus::Processed) => Ok(()),
            Some(super::DepositStatus::NativeQueued) => Err(Error::NativeSettlementPending),
            Some(
                super::DepositStatus::Retryable(error) | super::DepositStatus::Ambiguous(error),
            ) => Err(error),
            Some(super::DepositStatus::Pending | super::DepositStatus::InFlight) | None => {
                awaited.map(|_| ()).and_then(|_| {
                    Err(Error::Internal(
                        "VFT reply hook did not record receipt progress".into(),
                    ))
                })
            }
        };
    }

    awaited.map(|_| ()).and_then(|_| {
        let key = (slot, transaction_index);
        match super::reply_statuses_mut().remove(&key) {
            Some(status) => status.clone(),
            None => unreachable!("Status always set if a VFT invocation was successful"),
        }
    })
}

/// Persistent linkage survives a timed-out or dead originating continuation.
pub fn handle_persistent_reply() {
    let Ok(child) = msg::reply_to() else { return };
    let Some(data) = children().get(&child).cloned() else {
        return;
    };
    if data.terminal || msg::source() != data.token_id {
        return;
    }
    if let Some(log) = data.log_index {
        if super::original_child((data.slot, data.transaction_index), log) != Some(child) {
            return;
        }
    }
    let status = (|| {
        if matches!(msg::reply_code(), Ok(ReplyCode::Error(_))) {
            return Err(Error::ReplyFailure("Reply error code received".into()));
        }
        if !matches!(msg::reply_code(), Ok(ReplyCode::Success(_))) {
            return Err(Error::InvalidReply);
        }
        let bytes = msg::load_bytes().map_err(|e| Error::GasForReplyTooLow(format!("{e:?}")))?;
        match data.action {
            ChildAction::Mint => decode_complete::<Mint>(&bytes)?.check(),
            ChildAction::Unlock => decode_complete::<TransferFrom>(&bytes)?.check(),
            ChildAction::Native => {
                let r = decode_complete::<RedeemEscrow>(&bytes)?;
                if (r.from, r.to, r.amount) != (Syscall::program_id(), data.receiver, data.amount) {
                    return Err(Error::InvalidReply);
                }
                match r.status {
                    PayoutStatus::Delivered => Ok(()),
                    PayoutStatus::Queued => Err(Error::NativeSettlementPending),
                    PayoutStatus::Returned => Err(Error::NativeSettlementReturned),
                    PayoutStatus::Ambiguous => Err(Error::InvalidReply),
                }
            }
        }
    })();
    children().get_mut(&child).unwrap().terminal = true;
    let key = (data.slot, data.transaction_index);
    if let Some(log_index) = data.log_index {
        let progress = match &status {
            Ok(()) => super::DepositStatus::Processed,
            Err(Error::NativeSettlementPending) => super::DepositStatus::NativeQueued,
            Err(error @ Error::ReplyFailure(_)) => super::DepositStatus::Retryable(error.clone()),
            Err(error) => super::DepositStatus::Ambiguous(error.clone()),
        };
        super::set_deposit_status(key, log_index, progress);
        if status.is_ok() {
            emit_event(data.receiver, data.erc20_sender, data.amount, data.token_id);
            super::emit_settled(key, log_index);
        }
    } else {
        super::reply_statuses_mut().insert(key, status.clone());
        if status.is_ok() {
            super::complete_transaction(key);
            emit_event(data.receiver, data.erc20_sender, data.amount, data.token_id);
        } else if matches!(status, Err(Error::ReplyFailure(_) | Error::InvalidReply)) {
            super::reserved_transactions_mut().remove(&key);
        }
    }
}

fn decode_complete<Action: ActionIo>(bytes: &[u8]) -> Result<Action::Reply, Error>
where
    Action::Reply: Encode,
{
    let reply = Action::decode_reply(bytes).map_err(|e| Error::Internal(format!("{e:?}")))?;
    let route_len = if Action::is_empty_tuple::<Action::Reply>() {
        0
    } else {
        Action::ROUTE.len()
    };
    if bytes.len() != route_len + reply.encoded_size() {
        return Err(Error::InvalidReply);
    }
    Ok(reply)
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
        (slot, transaction_index),
        erc20_sender,
        token_id,
        &(receiver, amount),
        config,
        None,
        ChildAction::Mint,
    )
    .await
}

pub(super) async fn mint_log(
    key: (u64, u64),
    deposit: &super::ReceiptDeposit,
    config: &Config,
) -> Result<(), Error> {
    send::<Mint>(
        key,
        deposit.sender,
        deposit.token_id,
        &(deposit.receiver, deposit.amount),
        config,
        Some(deposit.log_index),
        ChildAction::Mint,
    )
    .await
}

/// Transfer `amount` tokens from the current program address to the `receiver` address,
/// effectively unlocking them.
///
/// It will send `TransferFrom` call to the corresponding `VFT` program and
/// asynchronously wait for the reply.
/// Transfer `amount` tokens from the current program address to the `receiver` address,
/// effectively unlocking them.
pub async fn unlock(
    slot: u64,
    transaction_index: u64,
    erc20_sender: H160,
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    config: &Config,
) -> Result<(), Error> {
    send::<TransferFrom>(
        (slot, transaction_index),
        erc20_sender,
        token_id,
        &(Syscall::program_id(), receiver, amount),
        config,
        None,
        ChildAction::Unlock,
    )
    .await
}

pub(super) async fn unlock_log(
    key: (u64, u64),
    deposit: &super::ReceiptDeposit,
    config: &Config,
) -> Result<(), Error> {
    send::<TransferFrom>(
        key,
        deposit.sender,
        deposit.token_id,
        &(Syscall::program_id(), deposit.receiver, deposit.amount),
        config,
        Some(deposit.log_index),
        ChildAction::Unlock,
    )
    .await
}

impl Params for (H256, ActorId, ActorId, U256) {
    fn receiver(&self) -> ActorId {
        self.2
    }
    fn amount(&self) -> U256 {
        self.3
    }
}
impl Reply for Redemption {
    fn check(&self) -> Result<(), Error> {
        Err(Error::NativeSettlementPending)
    }
}

pub(super) async fn redeem_native_log(
    key: (u64, u64),
    deposit: &super::ReceiptDeposit,
    config: &Config,
) -> Result<(), Error> {
    if matches!(
        super::deposit_status(key, deposit.log_index),
        Some(super::DepositStatus::NativeQueued)
    ) {
        reconcile_native_log(
            key,
            deposit.log_index,
            deposit.sender,
            deposit.token_id,
            deposit.receiver,
            deposit.amount,
            deposit.operation_id,
        )
        .await
    } else {
        send::<RedeemEscrow>(
            key,
            deposit.sender,
            deposit.token_id,
            &(
                deposit.operation_id,
                Syscall::program_id(),
                deposit.receiver,
                deposit.amount,
            ),
            config,
            Some(deposit.log_index),
            ChildAction::Native,
        )
        .await
    }
}

/// Query the immutable original payout only: this path cannot dispatch an economic effect.
pub(super) async fn reconcile_native_log(
    key: (u64, u64),
    log_index: u64,
    erc20_sender: H160,
    token_id: ActorId,
    receiver: ActorId,
    amount: U256,
    operation_id: H256,
) -> Result<(), Error> {
    if matches!(
        super::deposit_status(key, log_index),
        Some(super::DepositStatus::Processed)
    ) {
        return Ok(());
    }
    if !matches!(
        super::deposit_status(key, log_index),
        Some(super::DepositStatus::NativeQueued)
    ) {
        return Err(Error::NativeSettlementPending);
    }
    let bytes = msg::send_bytes_for_reply(token_id, GetRedemption::encode_call(operation_id), 0, 0)
        .map_err(|e| Error::Internal(format!("{e:?}")))?
        .await
        .map_err(|e| Error::Internal(format!("{e:?}")))?;
    let redemption = decode_complete::<GetRedemption>(&bytes)?.ok_or(Error::InvalidReply)?;
    if (redemption.from, redemption.to, redemption.amount)
        != (Syscall::program_id(), receiver, amount)
    {
        return Err(Error::InvalidReply);
    }
    match redemption.status {
        PayoutStatus::Delivered => {
            if matches!(
                super::deposit_status(key, log_index),
                Some(super::DepositStatus::Processed)
            ) {
                return Ok(());
            }
            if !matches!(
                super::deposit_status(key, log_index),
                Some(super::DepositStatus::NativeQueued)
            ) {
                return Err(Error::NativeSettlementPending);
            }
            super::set_deposit_status(key, log_index, super::DepositStatus::Processed);
            super::emit_settled(key, log_index);
            emit_event(receiver, erc20_sender, amount, token_id);
            Ok(())
        }
        PayoutStatus::Queued => Err(Error::NativeSettlementPending),
        PayoutStatus::Returned => {
            super::set_deposit_status(
                key,
                log_index,
                super::DepositStatus::Ambiguous(Error::NativeSettlementReturned),
            );
            Err(Error::NativeSettlementReturned)
        }
        PayoutStatus::Ambiguous => {
            super::set_deposit_status(
                key,
                log_index,
                super::DepositStatus::Ambiguous(Error::InvalidReply),
            );
            Err(Error::InvalidReply)
        }
    }
}

#[allow(unused_variables)]
fn emit_event(to: ActorId, from: H160, amount: U256, token: ActorId) {
    #[cfg(target_arch = "wasm32")]
    {
        const ROUTE: [u8; 11usize] = [
            40u8, 86u8, 102u8, 116u8, 77u8, 97u8, 110u8, 97u8, 103u8, 101u8, 114u8,
        ];
        EventEmitter::<crate::services::Event>::new(&ROUTE)
            .emit_event(crate::services::Event::BridgingAccepted {
                to,
                from,
                amount,
                token,
            })
            .expect("Failed to emit event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_reply_decoding_preserves_unit_wire_and_rejects_trailing_native_state() {
        assert_eq!(decode_complete::<Mint>(&[]), Ok(()));
        assert_eq!(
            decode_complete::<Mint>(Mint::ROUTE),
            Err(Error::InvalidReply)
        );
        let mut reply = GetRedemption::ROUTE.to_vec();
        None::<Redemption>.encode_to(&mut reply);
        assert_eq!(decode_complete::<GetRedemption>(&reply), Ok(None));
        reply.push(0);
        assert_eq!(
            decode_complete::<GetRedemption>(&reply),
            Err(Error::InvalidReply)
        );
    }
}
