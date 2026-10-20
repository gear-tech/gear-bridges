use crate::services::{Config, Error};
use gstd::{errors::Error as GStdError, msg};
use sails_rs::{calls::ActionIo, prelude::*};
use vft_client::{vft::io::TransferFrom, vft_admin::io::Mint};
use vft_vara_client::{
    native_escrow::io::{RedeemEscrow, Redemption as GetRedemption},
    PayoutStatus,
};

#[derive(Clone)]
struct TxDetails {
    slot: u64,
    transaction_index: u64,
    erc20_sender: H160,
    receiver: ActorId,
    token_id: ActorId,
    amount: U256,
    log_index: u64,
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

pub(super) fn is_quiescent() -> bool {
    children().values().all(|child| child.terminal)
}

async fn send<Action>(
    (slot, transaction_index): (u64, u64),
    deposit: &super::ReceiptDeposit,
    params: &Action::Params,
    config: &Config,
    action: ChildAction,
) -> Result<(), Error>
where
    Action: ActionIo,
{
    let token_id = deposit.token_id;
    let log_index = deposit.log_index;
    let payload = Action::encode_call(params);
    let tx_details = TxDetails {
        slot,
        transaction_index,
        erc20_sender: deposit.sender,
        receiver: deposit.receiver,
        token_id,
        amount: deposit.amount,
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
    super::record_child((slot, transaction_index), log_index, child);
    let reply = reply
        .up_to(Some(config.reply_timeout))
        .expect("Cannot install child timeout: roll back dispatch")
        .handle_reply(handle_persistent_reply)
        .expect("Cannot install child reply hook: roll back dispatch");
    let awaited = reply.await.map_err(|e| match e {
        GStdError::Timeout(..) => Error::ReplyTimeout(format!("{e:?}")),
        _ => Error::Internal(format!("{e:?}")),
    });

    match super::deposit_status((slot, transaction_index), log_index) {
        Some(super::DepositStatus::Processed | super::DepositStatus::Refunded) => Ok(()),
        Some(super::DepositStatus::NativeQueued) => Err(Error::NativeSettlementPending),
        Some(super::DepositStatus::Retryable(error) | super::DepositStatus::Ambiguous(error)) => {
            Err(error)
        }
        Some(super::DepositStatus::Pending | super::DepositStatus::InFlight) | None => {
            awaited.map(|_| ()).and_then(|_| {
                Err(Error::Internal(
                    "VFT reply hook did not record receipt progress".into(),
                ))
            })
        }
    }
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
    if super::original_child((data.slot, data.transaction_index), data.log_index) != Some(child) {
        return;
    }
    let mut refund = None;
    let status = (|| {
        if matches!(msg::reply_code(), Ok(ReplyCode::Error(_))) {
            return Err(Error::ReplyFailure("Reply error code received".into()));
        }
        if !matches!(msg::reply_code(), Ok(ReplyCode::Success(_))) {
            return Err(Error::InvalidReply);
        }
        let bytes = msg::load_bytes().map_err(|e| Error::GasForReplyTooLow(format!("{e:?}")))?;
        match data.action {
            ChildAction::Mint => decode_complete::<Mint>(&bytes).map(|_| false),
            ChildAction::Unlock => decode_complete::<TransferFrom>(&bytes)?
                .then_some(false)
                .ok_or(Error::InvalidReply),
            ChildAction::Native => {
                let r = decode_complete::<RedeemEscrow>(&bytes)?;
                if (r.from, r.to, r.amount) != (Syscall::program_id(), data.receiver, data.amount)
                    || r.child.is_zero()
                {
                    return Err(Error::InvalidReply);
                }
                match r.status {
                    PayoutStatus::Delivered if r.returned_value == 0 => Ok(false),
                    PayoutStatus::Refunded if U256::from(r.returned_value) == r.amount => {
                        refund = Some((r.child, r.returned_value));
                        Ok(true)
                    }
                    PayoutStatus::Queued
                        if r.returned_value == 0 || U256::from(r.returned_value) == r.amount =>
                    {
                        Err(Error::NativeSettlementPending)
                    }
                    PayoutStatus::Returned => Err(Error::NativeSettlementReturned),
                    _ => Err(Error::InvalidReply),
                }
            }
        }
    })();
    if status.is_ok() {
        // Completed progress retains the original child and replay evidence.
        children().remove(&child);
    } else {
        children().get_mut(&child).unwrap().terminal = true;
    }
    let key = (data.slot, data.transaction_index);
    let log_index = data.log_index;
    let progress = match &status {
        Ok(false) => super::DepositStatus::Processed,
        Ok(true) => super::DepositStatus::Refunded,
        Err(Error::NativeSettlementPending) => super::DepositStatus::NativeQueued,
        Err(error @ Error::ReplyFailure(_)) => super::DepositStatus::Retryable(error.clone()),
        Err(error) => super::DepositStatus::Ambiguous(error.clone()),
    };
    super::set_deposit_status(key, log_index, progress);
    if matches!(status, Ok(false)) {
        emit_event(data.receiver, data.erc20_sender, data.amount, data.token_id);
        super::emit_settled(key, log_index);
    } else if let Some((payout_child, returned_value)) = refund {
        super::emit_refunded(key, log_index, payout_child, returned_value);
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

pub(super) async fn mint_log(
    key: (u64, u64),
    deposit: &super::ReceiptDeposit,
    config: &Config,
) -> Result<(), Error> {
    send::<Mint>(
        key,
        deposit,
        &(deposit.receiver, deposit.amount),
        config,
        ChildAction::Mint,
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
        deposit,
        &(Syscall::program_id(), deposit.receiver, deposit.amount),
        config,
        ChildAction::Unlock,
    )
    .await
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
        reconcile_native_log(key, deposit, config).await
    } else {
        send::<RedeemEscrow>(
            key,
            deposit,
            &(
                deposit.operation_id,
                Syscall::program_id(),
                deposit.receiver,
                deposit.amount,
            ),
            config,
            ChildAction::Native,
        )
        .await
    }
}

/// Reconcile the immutable original payout; retry backed wrapped credit, never a new payout.
pub(super) async fn reconcile_native_log(
    key: (u64, u64),
    deposit: &super::ReceiptDeposit,
    config: &Config,
) -> Result<(), Error> {
    let super::ReceiptDeposit {
        log_index,
        sender: erc20_sender,
        token_id,
        receiver,
        amount,
        operation_id,
        ..
    } = *deposit;
    if matches!(
        super::deposit_status(key, log_index),
        Some(super::DepositStatus::Processed | super::DepositStatus::Refunded)
    ) {
        return Ok(());
    }
    if !matches!(
        super::deposit_status(key, log_index),
        Some(super::DepositStatus::NativeQueued)
    ) {
        return Err(Error::NativeSettlementPending);
    }
    let bytes = msg::send_bytes_with_gas_for_reply(
        token_id,
        GetRedemption::encode_call(operation_id),
        config.gas_for_token_ops,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|e| Error::Internal(format!("{e:?}")))?
    .up_to(Some(config.reply_timeout))
    .map_err(|e| Error::Internal(format!("{e:?}")))?
    .await
    .map_err(|e| match e {
        GStdError::Timeout(..) => Error::ReplyTimeout(format!("{e:?}")),
        _ => Error::Internal(format!("{e:?}")),
    })?;
    let mut redemption = decode_complete::<GetRedemption>(&bytes)?.ok_or(Error::InvalidReply)?;
    if (redemption.from, redemption.to, redemption.amount)
        != (Syscall::program_id(), receiver, amount)
        || redemption.child.is_zero()
    {
        return Err(Error::InvalidReply);
    }
    // A committed original full-value rejection may still await wrapped credit.
    // Query first: recovery must never create a missing operation/native payout.
    if redemption.status == PayoutStatus::Queued && U256::from(redemption.returned_value) == amount
    {
        let payout_child = redemption.child;
        let bytes = msg::send_bytes_with_gas_for_reply(
            token_id,
            RedeemEscrow::encode_call(operation_id, Syscall::program_id(), receiver, amount),
            config.gas_for_token_ops,
            0,
            config.gas_for_reply_deposit,
        )
        .map_err(|e| Error::Internal(format!("{e:?}")))?
        .up_to(Some(config.reply_timeout))
        .map_err(|e| Error::Internal(format!("{e:?}")))?
        .await
        .map_err(|e| match e {
            GStdError::Timeout(..) => Error::ReplyTimeout(format!("{e:?}")),
            _ => Error::Internal(format!("{e:?}")),
        })?;
        redemption = decode_complete::<RedeemEscrow>(&bytes)?;
        if (
            redemption.from,
            redemption.to,
            redemption.amount,
            redemption.child,
        ) != (Syscall::program_id(), receiver, amount, payout_child)
        {
            return Err(Error::InvalidReply);
        }
    }
    match redemption.status {
        PayoutStatus::Delivered | PayoutStatus::Refunded => {
            let refunded = redemption.status == PayoutStatus::Refunded;
            if (refunded && U256::from(redemption.returned_value) != amount)
                || (!refunded && redemption.returned_value != 0)
            {
                return Err(Error::InvalidReply);
            }
            if matches!(
                super::deposit_status(key, log_index),
                Some(super::DepositStatus::Processed | super::DepositStatus::Refunded)
            ) {
                return Ok(());
            }
            if !matches!(
                super::deposit_status(key, log_index),
                Some(super::DepositStatus::NativeQueued)
            ) {
                return Err(Error::NativeSettlementPending);
            }
            super::set_deposit_status(
                key,
                log_index,
                if refunded {
                    super::DepositStatus::Refunded
                } else {
                    super::DepositStatus::Processed
                },
            );
            if let Some(child) = super::original_child(key, log_index) {
                children().remove(&child);
            }
            if !refunded {
                super::emit_settled(key, log_index);
                emit_event(receiver, erc20_sender, amount, token_id);
            } else {
                super::emit_refunded(key, log_index, redemption.child, redemption.returned_value);
            }
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
    use vft_vara_client::Redemption;

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
