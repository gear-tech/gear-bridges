use crate::services::{decode_complete_reply, Config, Error, NativeWrapperMode, VftManager};
use core::cell::Cell;
use gstd::{errors::Error as GStdError, msg};
use sails_rs::{calls::ActionIo, prelude::*, rc::Rc};
use vft_client::{vft::io::TransferFrom, vft_admin::io::Mint};
use vft_vara_client::{
    native_escrow::io::{HeldRedemption, RedeemEscrow, Redemption as GetRedemption},
    PayoutStatus, Redemption,
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
    operation_id: H256,
    failure: Option<ChildFailure>,
}

#[derive(Clone, Copy)]
enum ChildAction {
    Mint,
    Unlock,
    Native,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildFailure {
    ErrorReply,
    UnlockFalse,
}
static mut CHILDREN: Option<collections::BTreeMap<MessageId, TxDetails>> = None;
fn children() -> &'static mut collections::BTreeMap<MessageId, TxDetails> {
    unsafe { gstd::static_mut!(CHILDREN).get_or_insert_with(collections::BTreeMap::new) }
}

pub(super) fn is_quiescent() -> bool {
    children().values().all(|child| child.terminal)
}

fn original_details(guard: &super::LogGuard<'_>) -> Option<TxDetails> {
    let details = children().get(&guard.child?)?;
    let deposit = guard.deposit;
    ((details.slot, details.transaction_index) == guard.key
        && details.log_index == deposit.log_index
        && details.operation_id == deposit.operation_id
        && details.token_id == deposit.token_id
        && details.receiver == deposit.receiver
        && details.amount == deposit.amount
        && details.erc20_sender == deposit.sender)
        .then(|| details.clone())
}

pub(super) fn unlock_false(guard: &super::LogGuard<'_>) -> bool {
    original_details(guard).is_some_and(|details| {
        matches!(details.action, ChildAction::Unlock)
            && details.terminal
            && details.failure == Some(ChildFailure::UnlockFalse)
    })
}

pub(super) fn retryable_native_child(guard: &super::LogGuard<'_>) -> bool {
    guard.deposit.native
        && matches!(
            super::deposit_status(guard.key, guard.deposit.log_index),
            Some(super::DepositStatus::Retryable(_))
        )
        && original_details(guard).is_some_and(|details| {
            matches!(details.action, ChildAction::Native)
                && details.terminal
                && details.failure == Some(ChildFailure::ErrorReply)
        })
}

pub(super) fn native_mode(token: ActorId) -> Result<NativeWrapperMode, Error> {
    VftManager::new().state().native_wrapper
        .filter(|wrapper| wrapper.actor_id == token)
        .map(|wrapper| wrapper.mode)
        .ok_or(Error::InvalidReconciliation)
}

fn refund_backed(mode: NativeWrapperMode, amount: U256, returned_value: u128) -> bool {
    match mode {
        NativeWrapperMode::EscrowV1 => U256::from(returned_value) == amount,
        NativeWrapperMode::HeldEscrowV2 => U256::from(returned_value) >= amount,
    }
}

fn retry_backed_credit(mode: NativeWrapperMode, redemption: &Redemption) -> bool {
    match mode {
        NativeWrapperMode::EscrowV1 => {
            redemption.status == PayoutStatus::Queued
                && refund_backed(mode, redemption.amount, redemption.returned_value)
        }
        NativeWrapperMode::HeldEscrowV2 => {
            matches!(
                redemption.status,
                PayoutStatus::Queued | PayoutStatus::Returned | PayoutStatus::Ambiguous
            ) && refund_backed(mode, redemption.amount, redemption.returned_value)
        }
    }
}

/// Only the immutable original manager child is made quiescent by evidence.
pub(super) fn quiesce_original(guard: &super::LogGuard<'_>) -> Result<(), Error> {
    guard.check()?;
    if let Some(child) = guard.child {
        if children().contains_key(&child) {
            original_details(guard).ok_or(Error::InvalidReconciliation)?;
            children().get_mut(&child).unwrap().terminal = true;
        }
    }
    Ok(())
}

pub(super) fn validate_redemption(
    guard: &super::LogGuard<'_>,
    redemption: &Redemption,
) -> Result<(), Error> {
    let deposit = guard.deposit;
    if (redemption.from, redemption.to, redemption.amount)
        != (Syscall::program_id(), deposit.receiver, deposit.amount)
        || redemption.child.is_zero()
        || Some(redemption.child) == guard.child
    {
        return Err(Error::InvalidReconciliation);
    }
    Ok(())
}

/// The single exact query path used by reconciliation and admin resolution.
pub(super) async fn query_native<Action: ActionIo>(
    guard: &super::LogGuard<'_>,
    params: &Action::Params,
    config: &Config,
) -> Result<Action::Reply, Error>
where
    Action::Reply: Encode,
{
    guard.check()?;
    let expected_status = super::deposit_status(guard.key, guard.deposit.log_index);
    let target = guard.deposit.token_id;
    let authenticated = Rc::new(Cell::new(false));
    let reply = msg::send_bytes_with_gas_for_reply(
        target,
        Action::encode_call(params),
        config.gas_for_token_ops,
        0,
        config.gas_for_reply_deposit,
    )
    .map_err(|error| Error::Internal(format!("{error:?}")))?;
    let child = reply.waiting_reply_to;
    let observed = authenticated.clone();
    let result = reply
        .up_to(Some(config.reply_timeout))
        .map_err(|error| Error::Internal(format!("{error:?}")))?
        .handle_reply(move || {
            observed.set(
                msg::source() == target
                    && msg::reply_to() == Ok(child)
                    && matches!(msg::reply_code(), Ok(ReplyCode::Success(_))),
            );
        })
        .map_err(|error| Error::Internal(format!("{error:?}")))?
        .await;
    guard.check()?;
    if super::deposit_status(guard.key, guard.deposit.log_index) != expected_status {
        return Err(Error::InvalidReconciliation);
    }
    decode_authenticated_query::<Action>(
        result.map_err(|error| match error {
            GStdError::Timeout(..) => Error::ReplyTimeout(format!("{error:?}")),
            _ => Error::Internal(format!("{error:?}")),
        }),
        authenticated.get(),
    )
}

fn decode_authenticated_query<Action: ActionIo>(
    result: Result<Vec<u8>, Error>,
    authenticated: bool,
) -> Result<Action::Reply, Error>
where
    Action::Reply: Encode,
{
    let bytes = result?;
    if !authenticated {
        return Err(Error::InvalidReply);
    }
    decode_complete_reply::<Action>(&bytes, |e| Error::Internal(format!("{e:?}")))
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
        operation_id: deposit.operation_id,
        failure: None,
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
    let guard = super::LogGuard::capture((slot, transaction_index), deposit, false)?;
    let reply = reply
        .up_to(Some(config.reply_timeout))
        .expect("Cannot install child timeout: roll back dispatch")
        .handle_reply(handle_persistent_reply)
        .expect("Cannot install child reply hook: roll back dispatch");
    let awaited = reply.await.map_err(|e| match e {
        GStdError::Timeout(..) => Error::ReplyTimeout(format!("{e:?}")),
        _ => Error::Internal(format!("{e:?}")),
    });
    guard.check()?;

    match super::deposit_status((slot, transaction_index), log_index) {
        Some(super::DepositStatus::Processed | super::DepositStatus::Refunded) => Ok(()),
        Some(super::DepositStatus::Rejected(_) | super::DepositStatus::Held) => {
            Err(Error::ReceiptResolved)
        }
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
    if super::deposit_status((data.slot, data.transaction_index), data.log_index)
        .is_none_or(|status| status.is_terminal())
    {
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
            ChildAction::Mint => {
                decode_complete_reply::<Mint>(&bytes, |e| Error::Internal(format!("{e:?}")))
                    .map(|_| false)
            }
            ChildAction::Unlock => {
                decode_complete_reply::<TransferFrom>(&bytes, |e| Error::Internal(format!("{e:?}")))?
                    .then_some(false)
                    .ok_or(Error::TokenOperationRejected)
            }
            ChildAction::Native => {
                let mode = native_mode(data.token_id)?;
                let r = decode_complete_reply::<RedeemEscrow>(&bytes, |e| {
                    Error::Internal(format!("{e:?}"))
                })?;
                if (r.from, r.to, r.amount) != (Syscall::program_id(), data.receiver, data.amount)
                    || r.child.is_zero()
                    || r.child == child
                {
                    return Err(Error::InvalidReply);
                }
                match r.status {
                    PayoutStatus::Delivered if r.returned_value == 0 => Ok(false),
                    PayoutStatus::Refunded if refund_backed(mode, r.amount, r.returned_value) => {
                        refund = Some((r.child, r.returned_value));
                        Ok(true)
                    }
                    PayoutStatus::Queued
                        if r.returned_value == 0 || refund_backed(mode, r.amount, r.returned_value) =>
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
        children().get_mut(&child).unwrap().failure = match &status {
            Err(Error::ReplyFailure(_)) => Some(ChildFailure::ErrorReply),
            Err(Error::TokenOperationRejected) => Some(ChildFailure::UnlockFalse),
            _ => None,
        };
    }
    let key = (data.slot, data.transaction_index);
    let log_index = data.log_index;
    let progress = match &status {
        Ok(false) => super::DepositStatus::Processed,
        Ok(true) => super::DepositStatus::Refunded,
        Err(Error::NativeSettlementPending) => super::DepositStatus::NativeQueued,
        Err(Error::TokenOperationRejected) => {
            super::DepositStatus::Rejected(super::ReceiptRejectionEvidence::UnlockFalse { child })
        }
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
    if super::original_child(key, deposit.log_index).is_some() {
        let guard = super::LogGuard::capture(key, deposit, false)?;
        if !retryable_native_child(&guard) {
            return reconcile_native_log(key, deposit, config).await;
        }
        // A new request must not inherit the completed error attempt's retry permission.
        super::set_deposit_status(key, deposit.log_index, super::DepositStatus::InFlight);
    }
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

/// Reconcile the immutable original payout; retry backed wrapped credit, never a new payout.
pub(super) async fn reconcile_native_log(
    key: (u64, u64),
    deposit: &super::ReceiptDeposit,
    config: &Config,
) -> Result<(), Error> {
    if let Some(status) = super::deposit_status(key, deposit.log_index) {
        match status {
            super::DepositStatus::Processed | super::DepositStatus::Refunded => return Ok(()),
            super::DepositStatus::Rejected(_) | super::DepositStatus::Held => {
                return Err(Error::ReceiptResolved);
            }
            _ => {}
        }
    }
    let guard = super::LogGuard::capture(key, deposit, false)?;
    if guard.child.is_none() {
        return Err(Error::InvalidReconciliation);
    }
    let mode = native_mode(deposit.token_id)?;
    let original = query_native::<GetRedemption>(&guard, &deposit.operation_id, config).await?;
    guard.check()?;
    let mut redemption = original.ok_or(Error::InvalidReconciliation)?;
    validate_redemption(&guard, &redemption)?;
    if mode == NativeWrapperMode::HeldEscrowV2 && matches!(
        redemption.status,
        PayoutStatus::Queued | PayoutStatus::Returned | PayoutStatus::Ambiguous
    ) {
        let held = query_native::<HeldRedemption>(&guard, &deposit.operation_id, config).await?;
        guard.check()?;
        if let Some(held) = held {
            validate_redemption(&guard, &held)?;
            if held.child != redemption.child
                || !matches!(
                    held.status,
                    PayoutStatus::Queued | PayoutStatus::Returned | PayoutStatus::Ambiguous
                )
            {
                return Err(Error::InvalidReconciliation);
            }
            // Only paused admin resolution may turn an authenticated hold into Held.
            return Err(Error::NativeSettlementPending);
        }
    }
    if retry_backed_credit(mode, &redemption) {
        let payout_child = redemption.child;
        // This existing-operation retry can only credit full returned backing.
        // It never starts a replacement native payout.
        redemption = query_native::<RedeemEscrow>(
            &guard,
            &(
                deposit.operation_id,
                Syscall::program_id(),
                deposit.receiver,
                deposit.amount,
            ),
            config,
        )
        .await?;
        guard.check()?;
        validate_redemption(&guard, &redemption)?;
        if redemption.child != payout_child
            || redemption.status != PayoutStatus::Refunded
            || !refund_backed(mode, deposit.amount, redemption.returned_value)
        {
            return Err(Error::InvalidReconciliation);
        }
    }
    match redemption.status {
        PayoutStatus::Delivered | PayoutStatus::Refunded => {
            let refunded = redemption.status == PayoutStatus::Refunded;
            if (refunded && !refund_backed(mode, deposit.amount, redemption.returned_value))
                || (!refunded && redemption.returned_value != 0)
            {
                return Err(Error::InvalidReply);
            }
            guard.check()?;
            quiesce_original(&guard)?;
            if let Some(child) = guard.child {
                if children()
                    .get(&child)
                    .is_some_and(|details| details.failure.is_none())
                {
                    children().remove(&child);
                }
            }
            super::set_deposit_status(
                key,
                deposit.log_index,
                if refunded {
                    super::DepositStatus::Refunded
                } else {
                    super::DepositStatus::Processed
                },
            );
            if !refunded {
                super::emit_settled(key, deposit.log_index);
                emit_event(
                    deposit.receiver,
                    deposit.sender,
                    deposit.amount,
                    deposit.token_id,
                );
            } else {
                super::emit_refunded(
                    key,
                    deposit.log_index,
                    redemption.child,
                    redemption.returned_value,
                );
            }
            Ok(())
        }
        PayoutStatus::Queued => {
            super::set_deposit_status(key, deposit.log_index, super::DepositStatus::NativeQueued);
            Err(Error::NativeSettlementPending)
        }
        PayoutStatus::Returned => {
            super::set_deposit_status(
                key,
                deposit.log_index,
                super::DepositStatus::Ambiguous(Error::NativeSettlementReturned),
            );
            Err(Error::NativeSettlementReturned)
        }
        PayoutStatus::Ambiguous => {
            super::set_deposit_status(
                key,
                deposit.log_index,
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
    fn immutable_v1_refund_and_retry_boundaries_are_not_v2_capabilities() {
        let mut redemption = Redemption {
            from: ActorId::from([1; 32]),
            to: ActorId::from([2; 32]),
            amount: U256::from(5u64),
            child: MessageId::from([3; 32]),
            status: PayoutStatus::Queued,
            returned_value: 5,
        };
        for returned_value in [0, 4, 5, 6] {
            redemption.returned_value = returned_value;
            assert_eq!(
                refund_backed(NativeWrapperMode::EscrowV1, redemption.amount, returned_value),
                returned_value == 5
            );
            assert_eq!(
                refund_backed(NativeWrapperMode::HeldEscrowV2, redemption.amount, returned_value),
                returned_value >= 5
            );
            for status in [
                PayoutStatus::Queued, PayoutStatus::Returned, PayoutStatus::Ambiguous,
                PayoutStatus::Delivered, PayoutStatus::Refunded,
            ] {
                redemption.status = status;
                assert_eq!(
                    retry_backed_credit(NativeWrapperMode::EscrowV1, &redemption),
                    redemption.status == PayoutStatus::Queued && returned_value == 5
                );
                assert_eq!(
                    retry_backed_credit(NativeWrapperMode::HeldEscrowV2, &redemption),
                    matches!(redemption.status,
                        PayoutStatus::Queued | PayoutStatus::Returned | PayoutStatus::Ambiguous)
                        && returned_value >= 5
                );
            }
        }
    }

    #[test]
    fn complete_reply_decoding_preserves_unit_wire_and_rejects_trailing_native_state() {
        let internal = |e| Error::Internal(format!("{e:?}"));
        assert_eq!(decode_complete_reply::<Mint>(&[], internal), Ok(()));
        assert_eq!(
            decode_complete_reply::<Mint>(Mint::ROUTE, internal),
            Err(Error::InvalidReply)
        );
        let mut unlock_false = TransferFrom::ROUTE.to_vec();
        false.encode_to(&mut unlock_false);
        assert_eq!(
            decode_complete_reply::<TransferFrom>(&unlock_false, internal),
            Ok(false)
        );
        unlock_false.push(0);
        assert_eq!(
            decode_complete_reply::<TransferFrom>(&unlock_false, internal),
            Err(Error::InvalidReply),
        );
        let mut reply = GetRedemption::ROUTE.to_vec();
        None::<Redemption>.encode_to(&mut reply);
        assert_eq!(
            decode_complete_reply::<GetRedemption>(&reply, internal),
            Ok(None)
        );
        reply.push(0);
        assert_eq!(
            decode_complete_reply::<GetRedemption>(&reply, internal),
            Err(Error::InvalidReply)
        );

        use vft_client::vft::io::{BalanceOf, Transfer};
        let mut balance = BalanceOf::ROUTE.to_vec();
        U256::from(7u64).encode_to(&mut balance);
        assert_eq!(
            decode_complete_reply::<BalanceOf>(&balance, internal),
            Ok(U256::from(7u64))
        );
        balance.push(0);
        assert_eq!(
            decode_complete_reply::<BalanceOf>(&balance, internal),
            Err(Error::InvalidReply)
        );
        let mut transfer = Transfer::ROUTE.to_vec();
        true.encode_to(&mut transfer);
        assert_eq!(
            decode_complete_reply::<Transfer>(&transfer, internal),
            Ok(true)
        );
        assert!(matches!(
            decode_complete_reply::<TransferFrom>(&transfer, internal),
            Err(Error::Internal(_))
        ));
        transfer.push(0);
        assert_eq!(
            decode_complete_reply::<Transfer>(&transfer, internal),
            Err(Error::InvalidReply)
        );
    }

    #[test]
    fn native_query_absence_requires_success_authentication_and_exact_decode() {
        let mut absent = GetRedemption::ROUTE.to_vec();
        None::<Redemption>.encode_to(&mut absent);
        assert_eq!(
            decode_authenticated_query::<GetRedemption>(Ok(absent.clone()), true),
            Ok(None),
        );
        assert_eq!(
            decode_authenticated_query::<GetRedemption>(Ok(absent.clone()), false),
            Err(Error::InvalidReply),
        );
        let timeout = Error::ReplyTimeout("original query expired".into());
        assert_eq!(
            decode_authenticated_query::<GetRedemption>(Err(timeout.clone()), false),
            Err(timeout),
        );
        let failure = Error::ReplyFailure("original query rejected".into());
        assert_eq!(
            decode_authenticated_query::<GetRedemption>(Err(failure.clone()), true),
            Err(failure),
        );
        absent.push(0);
        assert!(decode_authenticated_query::<GetRedemption>(Ok(absent), true).is_err());
        assert!(decode_authenticated_query::<GetRedemption>(Ok(vec![]), true).is_err());

        let redemption = Redemption {
            from: ActorId::from([1; 32]),
            to: ActorId::from([2; 32]),
            amount: 5u64.into(),
            child: MessageId::from([3; 32]),
            status: PayoutStatus::Queued,
            returned_value: 0,
        };
        let mut existing = GetRedemption::ROUTE.to_vec();
        Some(redemption.clone()).encode_to(&mut existing);
        assert_eq!(
            decode_authenticated_query::<GetRedemption>(Ok(existing.clone()), true),
            Ok(Some(redemption)),
        );
        existing.push(0);
        assert!(decode_authenticated_query::<GetRedemption>(Ok(existing), true).is_err());
    }

    #[test]
    fn native_retry_requires_exact_terminal_error_and_current_retryable_status() {
        let deposit = super::super::tests::original_deposit();
        let child = MessageId::from([21; 32]);
        let guard = super::super::LogGuard {
            key: (2, 3),
            deposit: &deposit,
            generation: 1,
            child: Some(child),
            revision: 0,
            admin: None,
        };
        use super::super::{DepositStatus, ReceiptLogProgress, ReceiptProgress};
        super::super::receipt_progress_mut().insert(
            guard.key,
            ReceiptProgress {
                deposits: vec![ReceiptLogProgress {
                    deposit: deposit.clone(),
                    status: DepositStatus::Retryable(Error::ReplyFailure("paused".into())),
                    child: Some(child),
                }],
                receipt_hash: H256::zero(),
                generation: 1,
                lease_until: 0,
            },
        );
        assert!(!retryable_native_child(&guard));
        children().insert(
            child,
            TxDetails {
                slot: 2,
                transaction_index: 3,
                erc20_sender: deposit.sender,
                receiver: deposit.receiver,
                token_id: deposit.token_id,
                amount: deposit.amount,
                log_index: deposit.log_index,
                action: ChildAction::Native,
                terminal: true,
                operation_id: deposit.operation_id,
                failure: None,
            },
        );
        assert!(!retryable_native_child(&guard));
        children().get_mut(&child).unwrap().failure = Some(ChildFailure::ErrorReply);
        assert!(retryable_native_child(&guard));
        children().get_mut(&child).unwrap().terminal = false;
        assert!(!retryable_native_child(&guard));
        children().get_mut(&child).unwrap().terminal = true;
        children().get_mut(&child).unwrap().failure = Some(ChildFailure::UnlockFalse);
        assert!(!retryable_native_child(&guard));
        children().get_mut(&child).unwrap().failure = Some(ChildFailure::ErrorReply);
        children().get_mut(&child).unwrap().action = ChildAction::Unlock;
        assert!(!retryable_native_child(&guard));
        children().get_mut(&child).unwrap().action = ChildAction::Native;
        let mut non_native = deposit.clone();
        non_native.native = false;
        assert!(!retryable_native_child(&super::super::LogGuard {
            deposit: &non_native,
            ..guard
        }));
        let original = children().get(&child).unwrap().clone();
        for field in 0..8 {
            let mut changed = original.clone();
            match field {
                0 => changed.slot += 1,
                1 => changed.transaction_index += 1,
                2 => changed.log_index += 1,
                3 => changed.operation_id = H256([99; 32]),
                4 => changed.token_id = ActorId::from([99; 32]),
                5 => changed.receiver = ActorId::from([99; 32]),
                6 => changed.amount += U256::one(),
                _ => changed.erc20_sender = H160([99; 20]),
            }
            children().insert(child, changed);
            assert!(!retryable_native_child(&guard));
        }
        children().insert(child, original);
        for status in [
            DepositStatus::Pending,
            DepositStatus::InFlight,
            DepositStatus::NativeQueued,
            DepositStatus::Ambiguous(Error::ReplyTimeout("retry timed out".into())),
        ] {
            super::super::set_deposit_status(guard.key, deposit.log_index, status);
            assert!(!retryable_native_child(&guard));
        }
        super::super::receipt_progress_mut().remove(&guard.key);
        children().remove(&child);
        assert!(!retryable_native_child(&guard));
    }
}
