use collections::{btree_map::BTreeMap, btree_set::BTreeSet};
use gstd::{static_mut, static_ref};
use sails_rs::prelude::*;

use super::{error::Error, Config, ReceiptStatus, TokenSupply, VftManager};

pub mod abi;
pub mod token_operations;

/// Successfully processed Ethereum transactions. They're stored to prevent
/// double-spending attacks on this program.
static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Receipt keys whose token operation has not reached a definite outcome yet.
/// Reservations are kept separate so completed-history eviction cannot remove them.
static mut RESERVED_TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Receipts with terminal exceptions are not successful transaction history.
// ponytail: retain exception tombstones indefinitely until an authenticated archival
// protocol can preserve their liability and replay evidence.
static mut RESOLVED_TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Maximum amount of successfully processed Ethereum transactions that this
/// program can store.
pub const TX_HISTORY_DEPTH: usize = 50_000_000;

/// Per-log progress lets one authenticated receipt account for every deposit without
/// repeating effects already completed by an earlier attempt.
static mut RECEIPT_PROGRESS: Option<BTreeMap<(u64, u64), ReceiptProgress>> = None;
// Unmapped authenticated deposits remain liabilities after admission rejection.
static mut UNMAPPED_LIABILITIES: u64 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReceiptDeposit {
    pub log_index: u64,
    pub sender: H160,
    pub receiver: ActorId,
    pub token_id: Option<ActorId>,
    pub eth_token_id: H160,
    pub amount: U256,
    pub supply: Option<TokenSupply>,
    pub native: bool,
    pub operation_id: H256,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum DepositStatus {
    Pending,
    InFlight,
    NativeQueued,
    Processed,
    Refunded,
    Retryable(Error),
    Ambiguous(Error),
    Rejected(ReceiptRejectionEvidence),
    Held,
}

impl DepositStatus {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Processed | Self::Refunded | Self::Rejected(_) | Self::Held
        )
    }
}

/// Retained rejection evidence, including historical native absence tombstones.
#[derive(Clone, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub enum ReceiptRejectionEvidence {
    InvalidRecipient,
    InvalidAmount,
    NativeAmountOverflow,
    UnlockFalse {
        child: MessageId,
    },
    /// Historical decoding only: absence does not prove economic rejection.
    NativeErrorAbsent {
        child: MessageId,
    },
    /// No mapping existed at admission; retained liability, never a refund.
    UnmappedToken,
}

fn initial_deposit_status(deposit: &ReceiptDeposit) -> DepositStatus {
    let rejection = if deposit.token_id.is_none() || deposit.supply.is_none() {
        ReceiptRejectionEvidence::UnmappedToken
    } else if deposit.receiver.is_zero()
        || (deposit.native
            && deposit.receiver == ActorId::from(*b"geargeargeargeargeargeargeargear"))
    {
        ReceiptRejectionEvidence::InvalidRecipient
    } else if deposit.amount.is_zero() {
        ReceiptRejectionEvidence::InvalidAmount
    } else if deposit.native && deposit.amount > (U256::one() << 80) - U256::one() {
        ReceiptRejectionEvidence::NativeAmountOverflow
    } else {
        return DepositStatus::Pending;
    };
    DepositStatus::Rejected(rejection)
}

#[derive(Clone)]
pub(super) struct ReceiptLogProgress {
    pub deposit: ReceiptDeposit,
    pub status: DepositStatus,
    pub child: Option<MessageId>,
}

fn validate_retained_rejection(log: &ReceiptLogProgress) -> Result<(), Error> {
    match &log.status {
        DepositStatus::Rejected(ReceiptRejectionEvidence::NativeErrorAbsent { .. }) => {
            Err(Error::InvalidReconciliation)
        }
        DepositStatus::Rejected(
            ReceiptRejectionEvidence::InvalidRecipient
            | ReceiptRejectionEvidence::InvalidAmount
            | ReceiptRejectionEvidence::NativeAmountOverflow
            | ReceiptRejectionEvidence::UnmappedToken,
        ) if log.child.is_some() => Err(Error::InvalidReconciliation),
        _ => Ok(()),
    }
}

#[derive(Clone)]
struct ReceiptProgress {
    deposits: Vec<ReceiptLogProgress>,
    receipt_hash: H256,
    generation: u64,
    lease_until: u32,
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub enum ReceiptDepositOutcome {
    Pending,
    InFlight,
    Settled,
    Rejected,
    Unknown,
    NativeQueued,
    /// Original native payout rejected; exact backed wrapped value credited.
    Refunded,
    /// A failed original operation remains eligible for a safe retry.
    Retryable,
    /// An unresolved native liability is retained independently of handoff.
    Held,
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub struct ReceiptDepositState {
    pub log_index: u64,
    pub sender: H160,
    pub receiver: ActorId,
    pub token_id: Option<ActorId>,
    pub eth_token_id: H160,
    pub amount: U256,
    pub supply: Option<TokenSupply>,
    pub native: bool,
    pub operation_id: H256,
    pub child: Option<MessageId>,
    pub outcome: ReceiptDepositOutcome,
}

pub fn receipt_deposits(key: (u64, u64)) -> Vec<ReceiptDepositState> {
    receipt_progress_mut()
        .get(&key)
        .map(|p| {
            p.deposits
                .iter()
                .map(|p| {
                    let d = &p.deposit;
                    ReceiptDepositState {
                        log_index: d.log_index,
                        sender: d.sender,
                        receiver: d.receiver,
                        token_id: d.token_id,
                        eth_token_id: d.eth_token_id,
                        amount: d.amount,
                        supply: d.supply,
                        native: d.native,
                        operation_id: d.operation_id,
                        child: p.child,
                        outcome: match p.status {
                            DepositStatus::Pending => ReceiptDepositOutcome::Pending,
                            DepositStatus::InFlight => ReceiptDepositOutcome::InFlight,
                            DepositStatus::Processed => ReceiptDepositOutcome::Settled,
                            DepositStatus::Refunded => ReceiptDepositOutcome::Refunded,
                            DepositStatus::Retryable(_) => ReceiptDepositOutcome::Retryable,
                            DepositStatus::Ambiguous(_) => ReceiptDepositOutcome::Unknown,
                            DepositStatus::NativeQueued => ReceiptDepositOutcome::NativeQueued,
                            DepositStatus::Rejected(_) => ReceiptDepositOutcome::Rejected,
                            DepositStatus::Held => ReceiptDepositOutcome::Held,
                        },
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn receipt_rejection(key: (u64, u64), log_index: u64) -> Option<ReceiptRejectionEvidence> {
    match deposit_status(key, log_index)? {
        DepositStatus::Rejected(evidence) => Some(evidence),
        _ => None,
    }
}

pub fn receipt_retryable_native(
    key: (u64, u64),
    log_index: u64,
    operation_id: H256,
    child: MessageId,
) -> bool {
    let Some(log) = receipt_progress_mut().get(&key).and_then(|progress| {
        progress
            .deposits
            .iter()
            .find(|log| log.deposit.log_index == log_index)
    }) else {
        return false;
    };
    if child.is_zero() || log.child != Some(child) || log.deposit.operation_id != operation_id {
        return false;
    }
    let deposit = log.deposit.clone();
    LogGuard::capture(key, &deposit, false)
        .is_ok_and(|guard| token_operations::retryable_native_child(&guard))
}

#[allow(unused_variables)]
pub(super) fn emit_settled(key: (u64, u64), log: u64) {
    let Some(p) = receipt_progress_mut().get(&key) else {
        return;
    };
    let Some(d) = p
        .deposits
        .iter()
        .find(|p| p.deposit.log_index == log)
        .map(|p| &p.deposit)
    else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    EventEmitter::<super::Event>::new(&[40, 86, 102, 116, 77, 97, 110, 97, 103, 101, 114])
        .emit_event(super::Event::ReceiptDepositSettled {
            slot: key.0,
            transaction_index: key.1,
            log_index: log,
            deposit_count: p.deposits.len() as u64,
            operation_id: d.operation_id,
            eth_token_id: d.eth_token_id,
            vara_token_id: d.token_id.expect("Settled deposit has a mapping"),
            sender: d.sender,
            receiver: d.receiver,
            amount: d.amount,
            native: d.native,
        })
        .expect("Failed to emit settled deposit");
}

#[allow(unused_variables)]
pub(super) fn emit_refunded(
    key: (u64, u64),
    log: u64,
    payout_child: MessageId,
    returned_value: u128,
) {
    let p = receipt_progress_mut()
        .get(&key)
        .expect("Original receipt missing");
    let progress = p
        .deposits
        .iter()
        .find(|p| p.deposit.log_index == log)
        .expect("Original deposit missing");
    let d = &progress.deposit;
    assert!(
        d.native && matches!(progress.status, DepositStatus::Refunded),
        "Not a wrapped refund"
    );
    assert!(
        U256::from(returned_value) >= d.amount,
        "Incomplete returned backing"
    );
    let child = progress.child.expect("Original wrapper child missing");
    assert!(
        !payout_child.is_zero() && payout_child != child,
        "Invalid original payout child"
    );
    #[cfg(target_arch = "wasm32")]
    EventEmitter::<super::Event>::new(&[40, 86, 102, 116, 77, 97, 110, 97, 103, 101, 114])
        .emit_event(super::Event::ReceiptDepositRefunded {
            slot: key.0,
            transaction_index: key.1,
            log_index: log,
            deposit_count: p.deposits.len() as u64,
            operation_id: d.operation_id,
            eth_token_id: d.eth_token_id,
            vara_token_id: d.token_id.expect("Refunded deposit has a mapping"),
            sender: d.sender,
            receiver: d.receiver,
            amount: d.amount,
            native: true,
            child,
            payout_child,
            returned_value,
        })
        .expect("Failed to emit wrapped refund");
}

fn receipt_progress_mut() -> &'static mut BTreeMap<(u64, u64), ReceiptProgress> {
    unsafe { static_mut!(RECEIPT_PROGRESS).get_or_insert_with(BTreeMap::new) }
}

pub(super) fn deposit_status(key: (u64, u64), log_index: u64) -> Option<DepositStatus> {
    let progress = receipt_progress_mut().get(&key)?;
    let index = progress
        .deposits
        .binary_search_by_key(&log_index, |progress| progress.deposit.log_index)
        .ok()?;
    Some(progress.deposits[index].status.clone())
}

pub(super) fn set_deposit_status(key: (u64, u64), log_index: u64, status: DepositStatus) {
    if let Some(progress) = receipt_progress_mut().get_mut(&key) {
        if let Ok(index) = progress
            .deposits
            .binary_search_by_key(&log_index, |progress| progress.deposit.log_index)
        {
            if !progress.deposits[index].status.is_terminal() {
                progress.deposits[index].status = status;
            }
        }
    }
}

fn release_receipt_lease(key: (u64, u64), generation: u64) {
    if let Some(progress) = receipt_progress_mut().get_mut(&key) {
        if progress.generation == generation {
            progress.lease_until = 0;
        }
    }
}

fn owns_generation(key: (u64, u64), generation: u64) -> bool {
    receipt_progress_mut()
        .get(&key)
        .is_some_and(|p| p.generation == generation)
}

/// Fence every asynchronous evidence read against the immutable original log and
/// the receipt generation that authorized it. No configuration is inferred anew.
pub(super) struct LogGuard<'a> {
    key: (u64, u64),
    deposit: &'a ReceiptDeposit,
    generation: u64,
    child: Option<MessageId>,
    revision: u64,
    admin: Option<ActorId>,
}

impl<'a> LogGuard<'a> {
    fn capture(key: (u64, u64), deposit: &'a ReceiptDeposit, admin: bool) -> Result<Self, Error> {
        let service = VftManager::new();
        let guard = Self {
            key,
            deposit,
            generation: receipt_progress_mut()
                .get(&key)
                .ok_or(Error::InvalidReconciliation)?
                .generation,
            child: original_child(key, deposit.log_index),
            revision: service.state().economic_revision,
            admin: admin.then(Syscall::message_source),
        };
        guard.check()?;
        Ok(guard)
    }

    fn check_snapshot(&self, progress: &ReceiptProgress, revision: u64) -> Result<(), Error> {
        if progress.generation != self.generation {
            return Err(Error::ReceiptLeaseActive);
        }
        if revision != self.revision
            || !progress
                .deposits
                .iter()
                .any(|log| log.deposit == *self.deposit && log.child == self.child)
        {
            return Err(Error::InvalidReconciliation);
        }
        Ok(())
    }

    fn check(&self) -> Result<(), Error> {
        let service = VftManager::new();
        service.ensure_receipt_domain(self.key.0)?;
        if self.admin.is_some_and(|admin| {
            service.state().admin != admin
                || Syscall::message_source() != admin
                || !service.state().is_paused
        }) {
            return Err(Error::InvalidReconciliation);
        }
        self.check_snapshot(
            receipt_progress_mut()
                .get(&self.key)
                .ok_or(Error::InvalidReconciliation)?,
            service.state().economic_revision,
        )
    }
}

pub(super) fn record_child(key: (u64, u64), log_index: u64, child: MessageId) {
    if let Some(log) = receipt_progress_mut().get_mut(&key).and_then(|p| {
        p.deposits
            .iter_mut()
            .find(|p| p.deposit.log_index == log_index)
    }) {
        if !log.status.is_terminal() {
            log.child = Some(child);
        }
    }
}

pub(super) fn original_child(key: (u64, u64), log_index: u64) -> Option<MessageId> {
    receipt_progress_mut()
        .get(&key)?
        .deposits
        .iter()
        .find(|p| p.deposit.log_index == log_index)?
        .child
}

/// Get reference to a transactions storage.
pub fn transactions() -> &'static BTreeSet<(u64, u64)> {
    unsafe { static_ref!(TRANSACTIONS).as_ref() }.expect("Program should be constructed")
}

/// Get mutable reference to a transactions storage.
pub fn transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe { static_mut!(TRANSACTIONS).as_mut() }.expect("Program should be constructed")
}

/// Get a reference to receipt reservations.
pub fn reserved_transactions() -> &'static BTreeSet<(u64, u64)> {
    unsafe { static_ref!(RESERVED_TRANSACTIONS).as_ref() }.expect("Program should be constructed")
}

/// Get a mutable reference to receipt reservations.
pub fn reserved_transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe { static_mut!(RESERVED_TRANSACTIONS).as_mut() }.expect("Program should be constructed")
}

fn resolved_transactions() -> &'static BTreeSet<(u64, u64)> {
    unsafe { static_ref!(RESOLVED_TRANSACTIONS).as_ref() }.expect("Program should be constructed")
}

fn resolved_transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe { static_mut!(RESOLVED_TRANSACTIONS).as_mut() }.expect("Program should be constructed")
}

/// Reconcile original native attempts even when a sibling remains ambiguous.
/// This may retry original full-return credit, but never creates a native payout.
pub async fn reconcile_receipt(key: (u64, u64), config: &Config) -> Result<ReceiptStatus, Error> {
    let service = VftManager::new();
    service.ensure_receipt_domain(key.0)?;
    service.ensure_running()?;
    if receipt_status(key) != ReceiptStatus::Reserved {
        return Ok(receipt_status(key));
    }
    let generation = match receipt_progress_mut().get(&key) {
        Some(progress) => progress.generation,
        None => return Ok(ReceiptStatus::Reserved),
    };
    let service = VftManager::new();
    let revision = service.state().economic_revision;
    let native = receipt_progress_mut()
        .get(&key)
        .map(|progress| {
            progress
                .deposits
                .iter()
                .filter(|log| {
                    log.deposit.native && log.child.is_some() && !log.status.is_terminal()
                })
                .map(|log| log.deposit.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut error = None;
    for deposit in native {
        let result = token_operations::reconcile_native_log(key, &deposit, config).await;
        service.ensure_receipt_domain(key.0)?;
        if !owns_generation(key, generation) {
            return Err(Error::ReceiptLeaseActive);
        }
        if service.state().economic_revision != revision {
            return Err(Error::InvalidReconciliation);
        }
        match result {
            Ok(()) | Err(Error::NativeSettlementPending | Error::ReceiptResolved) => {}
            Err(failure) => {
                error.get_or_insert(failure);
            }
        }
    }
    complete_transaction(key);
    if receipt_status(key) != ReceiptStatus::Reserved {
        return Ok(receipt_status(key));
    }
    if let Some(failure) = error.or_else(|| {
        receipt_progress_mut()
            .get(&key)?
            .deposits
            .iter()
            .find_map(|log| match &log.status {
                DepositStatus::Ambiguous(error) | DepositStatus::Retryable(error) => {
                    Some(error.clone())
                }
                _ => None,
            })
    }) {
        return Err(failure);
    }
    Ok(ReceiptStatus::Reserved)
}

/// Resolve only bounded no-effect evidence or an authenticated retained liability.
pub async fn resolve_receipt_deposit(
    service: &mut VftManager,
    key: (u64, u64),
    log_index: u64,
    operation_id: H256,
    child: Option<MessageId>,
) -> Result<ReceiptDepositOutcome, Error> {
    assert!(service.state().is_paused, "Not paused");
    service.ensure_receipt_domain(key.0)?;
    let now = Syscall::block_height();
    let progress = receipt_progress_mut()
        .get(&key)
        .ok_or(Error::InvalidReconciliation)?;
    let log = progress
        .deposits
        .iter()
        .find(|log| log.deposit.log_index == log_index)
        .ok_or(Error::InvalidReconciliation)?;
    if log.deposit.operation_id != operation_id
        || log.child != child
        || child.is_some_and(|child| child.is_zero())
    {
        return Err(Error::InvalidReconciliation);
    }
    if progress.lease_until > now {
        return Err(Error::ReceiptLeaseActive);
    }
    validate_retained_rejection(log)?;
    match log.status {
        DepositStatus::Rejected(_) => {
            complete_transaction(key);
            return Ok(ReceiptDepositOutcome::Rejected);
        }
        DepositStatus::Held => {
            complete_transaction(key);
            return Ok(ReceiptDepositOutcome::Held);
        }
        DepositStatus::Processed | DepositStatus::Refunded => {
            return Err(Error::InvalidReconciliation);
        }
        _ => {}
    }
    let deposit = log.deposit.clone();
    let generation = progress
        .generation
        .checked_add(1)
        .ok_or(Error::ReceiptLeaseActive)?;
    let lease_until = now
        .checked_add(service.config().reply_timeout.max(1))
        .ok_or(Error::ReceiptLeaseActive)?;
    let progress = receipt_progress_mut().get_mut(&key).unwrap();
    progress.generation = generation;
    progress.lease_until = lease_until;
    let result = match LogGuard::capture(key, &deposit, true) {
        Ok(guard) => {
            let evidence = resolve_log(&guard, service.config()).await;
            evidence.and_then(|status| {
                guard.check()?;
                token_operations::quiesce_original(&guard)?;
                let outcome = match status {
                    DepositStatus::Rejected(_) => ReceiptDepositOutcome::Rejected,
                    DepositStatus::Held => ReceiptDepositOutcome::Held,
                    _ => return Err(Error::InvalidReconciliation),
                };
                set_deposit_status(key, log_index, status);
                complete_transaction(key);
                Ok(outcome)
            })
        }
        Err(error) => Err(error),
    };
    release_receipt_lease(key, generation);
    result
}

async fn resolve_log(guard: &LogGuard<'_>, config: &Config) -> Result<DepositStatus, Error> {
    use vft_vara_client::native_escrow::io::{HeldRedemption, Redemption as GetRedemption};
    let deposit = guard.deposit;
    if guard.child.is_none() {
        return match initial_deposit_status(deposit) {
            status @ DepositStatus::Rejected(_) => Ok(status),
            _ => Err(Error::InvalidReconciliation),
        };
    }
    if token_operations::unlock_false(guard) {
        return Ok(DepositStatus::Rejected(
            ReceiptRejectionEvidence::UnlockFalse {
                child: guard.child.unwrap(),
            },
        ));
    }
    if !deposit.native {
        return Err(Error::InvalidReconciliation);
    }
    if token_operations::native_mode(deposit.token_id.ok_or(Error::InvalidReconciliation)?)?
        == super::NativeWrapperMode::EscrowV1
    {
        return Err(Error::NativeHoldUnsupported);
    }
    let original =
        token_operations::query_native::<GetRedemption>(guard, &deposit.operation_id, config)
            .await?;
    guard.check()?;
    let original = original.ok_or(Error::InvalidReconciliation)?;
    token_operations::validate_redemption(guard, &original)?;
    if !matches!(
        original.status,
        vft_vara_client::PayoutStatus::Queued
            | vft_vara_client::PayoutStatus::Returned
            | vft_vara_client::PayoutStatus::Ambiguous
    ) {
        return Err(Error::InvalidReconciliation);
    }
    let held =
        token_operations::query_native::<HeldRedemption>(guard, &deposit.operation_id, config)
            .await?
            .ok_or(Error::InvalidReconciliation)?;
    guard.check()?;
    token_operations::validate_redemption(guard, &held)?;
    if held.child != original.child
        || !matches!(
            held.status,
            vft_vara_client::PayoutStatus::Queued
                | vft_vara_client::PayoutStatus::Returned
                | vft_vara_client::PayoutStatus::Ambiguous
        )
    {
        return Err(Error::InvalidReconciliation);
    }
    Ok(DepositStatus::Held)
}

/// Read the retained state for one Ethereum receipt key.
pub fn receipt_status(key: (u64, u64)) -> ReceiptStatus {
    receipt_status_in(
        transactions(),
        reserved_transactions(),
        resolved_transactions(),
        key,
    )
}

fn receipt_status_in(
    processed: &BTreeSet<(u64, u64)>,
    reserved: &BTreeSet<(u64, u64)>,
    resolved: &BTreeSet<(u64, u64)>,
    key: (u64, u64),
) -> ReceiptStatus {
    if resolved.contains(&key) {
        ReceiptStatus::Resolved
    } else if processed.contains(&key) {
        ReceiptStatus::Processed
    } else if reserved.contains(&key) {
        ReceiptStatus::Reserved
    } else {
        ReceiptStatus::Unknown
    }
}

pub(super) fn is_quiescent() -> bool {
    reserved_transactions().is_empty()
        && token_operations::is_quiescent()
        && unsafe { UNMAPPED_LIABILITIES == 0 }
}

/// Complete only fully terminal receipts, keeping exceptions out of successful history.
pub fn complete_transaction(key: (u64, u64)) {
    let Some(progress) = receipt_progress_mut().get(&key) else {
        return;
    };
    if progress.deposits.is_empty() || !progress.deposits.iter().all(|log| log.status.is_terminal())
    {
        return;
    }
    let resolved = progress
        .deposits
        .iter()
        .any(|log| matches!(log.status, DepositStatus::Rejected(_) | DepositStatus::Held));
    reserved_transactions_mut().remove(&key);
    if resolved {
        resolved_transactions_mut().insert(key);
        return;
    }
    transactions_mut().insert(key);
    if transactions().len() > TX_HISTORY_DEPTH {
        if let Some(old) = transactions_mut().pop_first() {
            receipt_progress_mut().remove(&old);
        }
    }
}

#[cfg(test)]
fn complete_transaction_in(
    processed: &mut BTreeSet<(u64, u64)>,
    reserved: &mut BTreeSet<(u64, u64)>,
    key: (u64, u64),
    history_depth: usize,
) {
    reserved.remove(&key);
    processed.insert(key);
    if processed.len() > history_depth {
        processed.pop_first();
    }
}

/// Initialize state that's used by this VFT Manager method.
pub fn seed() {
    unsafe {
        TRANSACTIONS = Some(BTreeSet::new());
        RESERVED_TRANSACTIONS = Some(BTreeSet::new());
        RESOLVED_TRANSACTIONS = Some(BTreeSet::new());
        RECEIPT_PROGRESS = Some(BTreeMap::new());
        UNMAPPED_LIABILITIES = 0;
    }
}

/// Seed one isolated synthetic log, then measure the production mint/unlock reply path.
#[cfg(feature = "mocks")]
pub(super) async fn benchmark_reply(
    service: &mut VftManager,
    key: (u64, u64),
    supply: TokenSupply,
) -> Result<(), Error> {
    if transactions().contains(&key)
        || resolved_transactions().contains(&key)
        || reserved_transactions().contains(&key)
        || receipt_progress_mut().contains_key(&key)
    {
        return Err(Error::AlreadyProcessed);
    }
    service.ensure_economic_mutable();
    let lease_until = Syscall::block_height()
        .checked_add(service.config().reply_timeout.max(1))
        .ok_or(Error::ReceiptLeaseActive)?;
    let source = Syscall::message_source();
    let identity = H256(gstd::msg::id().into_bytes());
    let deposit = ReceiptDeposit {
        log_index: 0,
        sender: H160::zero(),
        receiver: source,
        token_id: Some(source),
        eth_token_id: H160::zero(),
        amount: 100u32.into(),
        supply: Some(supply),
        native: false,
        operation_id: identity,
    };
    reserved_transactions_mut().insert(key);
    receipt_progress_mut().insert(
        key,
        ReceiptProgress {
            deposits: vec![ReceiptLogProgress {
                deposit: deposit.clone(),
                status: DepositStatus::InFlight,
                child: None,
            }],
            receipt_hash: identity,
            generation: 1,
            lease_until,
        },
    );
    let revision = service.state().economic_revision;
    let result = match supply {
        TokenSupply::Ethereum => token_operations::mint_log(key, &deposit, service.config()).await,
        TokenSupply::Gear => token_operations::unlock_log(key, &deposit, service.config()).await,
    };
    if !owns_generation(key, 1) {
        return Err(Error::ReceiptLeaseActive);
    }
    if service.state().economic_revision != revision {
        release_receipt_lease(key, 1);
        return Err(Error::InvalidReconciliation);
    }
    match &result {
        Ok(()) => {}
        Err(error) if matches!(deposit_status(key, 0), Some(DepositStatus::InFlight)) => {
            set_deposit_status(key, 0, DepositStatus::Ambiguous(error.clone()));
        }
        Err(_) => {}
    }
    complete_transaction(key);
    release_receipt_lease(key, 1);
    result
}

/// Submit rlp-encoded transaction receipt.
///
/// This receipt is decoded under the hood and checked that it's a valid receipt from tx
/// sent to `ERC20Manager` contract. Also it will check that this transaction haven't been
/// processed yet.
///
/// This method can be called only by [State::historical_proxy_address] program.
pub async fn submit_receipt(
    service: &mut VftManager,
    slot: u64,
    transaction_index: u64,
    receipt_rlp: Vec<u8>,
) -> Result<(), Error> {
    service.ensure_receipt_domain(slot)?;
    use alloy_sol_types::SolEvent;
    use ethereum_common::utils::decode_canonical_receipt;

    let Some(erc20_manager_address) = service.state().erc20_manager_address else {
        panic!("Address of the ERC20Manger is not set");
    };
    if Syscall::message_source() != service.state().historical_proxy_address {
        return Err(Error::NotHistoricalProxy);
    }

    use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
    let receipt_hash = KeccakHasher::hash(&receipt_rlp);
    let receipt = decode_canonical_receipt(&receipt_rlp).map_err(|_| Error::UnsupportedEthEvent)?;
    if receipt
        .as_receipt()
        .and_then(|receipt| receipt.status.as_eip658())
        != Some(true)
    {
        return Err(Error::UnsupportedEthEvent);
    }

    let key = (slot, transaction_index);
    if transactions().contains(&key) || resolved_transactions().contains(&key) {
        return Err(Error::AlreadyProcessed);
    }
    let mut deposits = if let Some(progress) = receipt_progress_mut().get(&key) {
        if progress.receipt_hash != receipt_hash {
            return Err(Error::AlreadyProcessed);
        }
        progress
            .deposits
            .iter()
            .map(|p| p.deposit.clone())
            .collect()
    } else {
        Vec::new()
    };
    if deposits.is_empty() {
        for (log_index, log) in receipt.logs().iter().enumerate() {
            if H160::from(log.address.0 .0) != erc20_manager_address {
                continue;
            }
            let Ok(event) = abi::ERC20_MANAGER::BridgingRequested::decode_raw_log_validate(
                log.topics(),
                &log.data.data,
            ) else {
                continue;
            };
            let eth_token_id = H160::from(event.token.0 .0);
            let token_id = service
                .state()
                .token_map
                .get_vara_token_id(&eth_token_id)
                .ok();
            let supply =
                token_id.and_then(|token| service.state().token_map.get_supply_type(&token).ok());
            // SCALE of this fixed-width identity is exactly 161 bytes, with no tags
            // or vector prefix. Keep the same wire identity without a per-log allocation.
            let mut operation = [0u8; 161];
            operation[..21].copy_from_slice(b"vara/native-escrow/v1");
            operation[21..53].copy_from_slice(Syscall::program_id().as_ref());
            operation[53..85].copy_from_slice(service.state().historical_proxy_address.as_ref());
            operation[85..105].copy_from_slice(&erc20_manager_address.0);
            operation[105..113].copy_from_slice(&slot.to_le_bytes());
            operation[113..121].copy_from_slice(&transaction_index.to_le_bytes());
            operation[121..129].copy_from_slice(&(log_index as u64).to_le_bytes());
            operation[129..].copy_from_slice(&receipt_hash.0);
            deposits.push(ReceiptDeposit {
                log_index: log_index as u64,
                sender: H160::from(event.from.0 .0),
                receiver: ActorId::from(event.to.0),
                token_id,
                eth_token_id,
                amount: U256::from_little_endian(event.amount.as_le_slice()),
                supply,
                native: service
                    .state()
                    .native_wrapper
                    .is_some_and(|wrapper| Some(wrapper.actor_id) == token_id),
                operation_id: KeccakHasher::hash(&operation),
            });
        }
    }
    if deposits.is_empty() {
        return Err(Error::UnsupportedEthEvent);
    }

    let now = Syscall::block_height();
    let lease_until = now
        .checked_add(service.config().reply_timeout.max(1))
        .ok_or(Error::ReceiptLeaseActive)?;
    let generation;
    match receipt_progress_mut().get(&key) {
        Some(progress) => {
            if progress.lease_until > now {
                return Err(Error::ReceiptLeaseActive);
            }
            if !reserved_transactions().contains(&key) || progress.receipt_hash != receipt_hash {
                return Err(Error::AlreadyProcessed);
            }
            generation = progress
                .generation
                .checked_add(1)
                .ok_or(Error::ReceiptLeaseActive)?;
            let progress = receipt_progress_mut().get_mut(&key).unwrap();
            progress.generation = generation;
            progress.lease_until = lease_until;
        }
        None => {
            if reserved_transactions().contains(&key) {
                // Preserve pair-keyed reservations created before per-log progress existed.
                return Err(Error::AlreadyProcessed);
            }
            if transactions().len() >= TX_HISTORY_DEPTH
                && transactions()
                    .first()
                    .map(|first| &key < first)
                    .unwrap_or(false)
            {
                return Err(Error::TransactionTooOld);
            }
            generation = 1;
            let progress_deposits: Vec<_> = deposits
                .iter()
                .cloned()
                .map(|deposit| ReceiptLogProgress {
                    status: initial_deposit_status(&deposit),
                    deposit,
                    child: None,
                })
                .collect();
            let unmapped = progress_deposits
                .iter()
                .filter(|log| {
                    matches!(
                        log.status,
                        DepositStatus::Rejected(ReceiptRejectionEvidence::UnmappedToken)
                    )
                })
                .count() as u64;
            unsafe {
                UNMAPPED_LIABILITIES = UNMAPPED_LIABILITIES
                    .checked_add(unmapped)
                    .expect("Liability count overflow");
            }
            let has_processing_work = progress_deposits
                .iter()
                .any(|log| !log.status.is_terminal());
            if has_processing_work {
                reserved_transactions_mut().insert(key);
            }
            receipt_progress_mut().insert(
                key,
                ReceiptProgress {
                    deposits: progress_deposits,
                    receipt_hash,
                    generation,
                    lease_until: if has_processing_work { lease_until } else { 0 },
                },
            );
        }
    }

    service.record_receipt_admission(slot);
    let revision = service.state().economic_revision;
    let mut retryable_error = None;
    let mut ambiguous_error = None;
    for deposit in &deposits {
        if let Err(error) = service
            .ensure_running()
            .and_then(|_| service.ensure_receipt_domain(slot))
        {
            complete_transaction(key);
            release_receipt_lease(key, generation);
            return Err(error);
        }
        if !owns_generation(key, generation) {
            return Err(Error::ReceiptLeaseActive);
        }
        if service.state().economic_revision != revision {
            release_receipt_lease(key, generation);
            return Err(Error::InvalidReconciliation);
        }
        let Some(status) = deposit_status(key, deposit.log_index) else {
            release_receipt_lease(key, generation);
            return Err(Error::Internal("Receipt log progress is missing".into()));
        };
        match &status {
            DepositStatus::Processed
            | DepositStatus::Refunded
            | DepositStatus::Rejected(_)
            | DepositStatus::Held => continue,
            DepositStatus::Ambiguous(error) => {
                ambiguous_error.get_or_insert(error.clone());
                continue;
            }
            DepositStatus::InFlight => {
                let error = Error::ReplyTimeout(format!(
                    "Receipt log {} was interrupted in flight",
                    deposit.log_index
                ));
                set_deposit_status(
                    key,
                    deposit.log_index,
                    DepositStatus::Ambiguous(error.clone()),
                );
                ambiguous_error.get_or_insert(error);
                continue;
            }
            DepositStatus::Pending | DepositStatus::Retryable(_) | DepositStatus::NativeQueued => {}
        }

        if !(matches!(&status, DepositStatus::NativeQueued)
            || deposit.native && original_child(key, deposit.log_index).is_some())
        {
            set_deposit_status(key, deposit.log_index, DepositStatus::InFlight);
        }
        let result = match deposit.supply {
            Some(TokenSupply::Ethereum) => {
                token_operations::mint_log(key, deposit, service.config()).await
            }
            Some(TokenSupply::Gear) if deposit.native => {
                token_operations::redeem_native_log(key, deposit, service.config()).await
            }
            Some(TokenSupply::Gear) => {
                token_operations::unlock_log(key, deposit, service.config()).await
            }
            None => Err(Error::InvalidReconciliation),
        };
        if !owns_generation(key, generation) {
            return Err(Error::ReceiptLeaseActive);
        }
        if service.state().economic_revision != revision {
            release_receipt_lease(key, generation);
            return Err(Error::InvalidReconciliation);
        }
        if let Err(error) = service.ensure_running() {
            complete_transaction(key);
            release_receipt_lease(key, generation);
            return Err(error);
        }
        if let Err(error) = result {
            match deposit_status(key, deposit.log_index) {
                Some(DepositStatus::Retryable(error)) => {
                    retryable_error.get_or_insert(error);
                }
                Some(DepositStatus::Ambiguous(error)) => {
                    ambiguous_error.get_or_insert(error);
                }
                Some(
                    DepositStatus::Processed
                    | DepositStatus::Refunded
                    | DepositStatus::Rejected(_)
                    | DepositStatus::Held,
                ) => {}
                Some(DepositStatus::NativeQueued) if error == Error::NativeSettlementPending => {}
                Some(DepositStatus::NativeQueued) => {
                    ambiguous_error.get_or_insert(error.clone());
                }
                Some(DepositStatus::InFlight | DepositStatus::Pending) | None => {
                    ambiguous_error.get_or_insert(error.clone());
                    set_deposit_status(key, deposit.log_index, DepositStatus::Ambiguous(error));
                }
            }
        }
    }

    if let Some(error) = ambiguous_error.or(retryable_error) {
        release_receipt_lease(key, generation);
        return Err(error);
    }
    if deposits.iter().any(|deposit| {
        !deposit_status(key, deposit.log_index).is_some_and(|status| status.is_terminal())
    }) {
        release_receipt_lease(key, generation);
        if deposits.iter().all(|deposit| {
            deposit_status(key, deposit.log_index).is_some_and(|status| {
                status.is_terminal() || matches!(status, DepositStatus::NativeQueued)
            })
        }) {
            return Err(Error::NativeSettlementPending);
        }
        return Err(Error::Internal(
            "Receipt log processing is incomplete".into(),
        ));
    }

    complete_transaction(key);
    release_receipt_lease(key, generation);
    if receipt_status(key) == ReceiptStatus::Resolved {
        Err(Error::ReceiptResolved)
    } else {
        Ok(())
    }
}

pub fn fill_transactions() -> bool {
    let transactions = transactions_mut();
    if TX_HISTORY_DEPTH <= transactions.len() {
        return false;
    }

    let count = cmp::min(
        TX_HISTORY_DEPTH - transactions.len(),
        super::SIZE_FILL_TRANSACTIONS_STEP,
    );
    let (last, _) = transactions.last().copied().unwrap();
    for i in 0..count {
        transactions.insert((last + 1, i as u64));
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_history_never_evicts_an_in_flight_reservation() {
        let mut processed = BTreeSet::from([(2, 0), (3, 0)]);
        let mut reserved = BTreeSet::from([(1, 0), (4, 0)]);

        complete_transaction_in(&mut processed, &mut reserved, (4, 0), 2);

        assert_eq!(processed, BTreeSet::from([(3, 0), (4, 0)]));
        assert_eq!(reserved, BTreeSet::from([(1, 0)]));
    }
    #[test]
    fn receipt_status_tracks_reservation_processing_and_history_eviction() {
        let mut processed = BTreeSet::from([(1, 0)]);
        let mut reserved = BTreeSet::new();
        let mut resolved = BTreeSet::new();
        let key = (2, 7);

        assert_eq!(
            receipt_status_in(&processed, &reserved, &resolved, key),
            ReceiptStatus::Unknown
        );
        reserved.insert(key);
        assert_eq!(
            receipt_status_in(&processed, &reserved, &resolved, key),
            ReceiptStatus::Reserved
        );

        complete_transaction_in(&mut processed, &mut reserved, key, 1);
        assert_eq!(
            receipt_status_in(&processed, &reserved, &resolved, key),
            ReceiptStatus::Processed
        );
        assert_eq!(
            receipt_status_in(&processed, &reserved, &resolved, (1, 0)),
            ReceiptStatus::Unknown
        );

        complete_transaction_in(&mut processed, &mut reserved, (3, 0), 1);
        assert_eq!(
            receipt_status_in(&processed, &reserved, &resolved, key),
            ReceiptStatus::Unknown
        );

        resolved.insert(key);
        complete_transaction_in(&mut processed, &mut reserved, (4, 0), 1);
        assert_eq!(
            receipt_status_in(&processed, &reserved, &resolved, key),
            ReceiptStatus::Resolved
        );
    }

    pub(super) fn original_deposit() -> ReceiptDeposit {
        ReceiptDeposit {
            log_index: 0,
            sender: H160::zero(),
            receiver: ActorId::from([1; 32]),
            token_id: Some(ActorId::from([2; 32])),
            eth_token_id: H160::zero(),
            amount: 5u64.into(),
            supply: Some(TokenSupply::Gear),
            native: true,
            operation_id: H256([3; 32]),
        }
    }

    #[test]
    fn asynchronous_log_fence_rejects_stale_generation_revision_and_identity() {
        let deposit = original_deposit();
        let child = Some(MessageId::from([4; 32]));
        let guard = LogGuard {
            key: (1, 0),
            deposit: &deposit,
            generation: 7,
            child,
            revision: 9,
            admin: None,
        };
        let mut progress = ReceiptProgress {
            deposits: vec![ReceiptLogProgress {
                deposit: deposit.clone(),
                status: DepositStatus::Ambiguous(Error::InvalidReply),
                child,
            }],
            receipt_hash: H256::zero(),
            generation: 7,
            lease_until: 0,
        };
        assert_eq!(guard.check_snapshot(&progress, 9), Ok(()));
        progress.generation += 1;
        assert_eq!(
            guard.check_snapshot(&progress, 9),
            Err(Error::ReceiptLeaseActive)
        );
        progress.generation -= 1;
        assert_eq!(
            guard.check_snapshot(&progress, 10),
            Err(Error::InvalidReconciliation)
        );
        progress.deposits[0].child = Some(MessageId::from([5; 32]));
        assert_eq!(
            guard.check_snapshot(&progress, 9),
            Err(Error::InvalidReconciliation)
        );
        progress.deposits[0].child = child;
        progress.deposits[0].deposit.operation_id = H256([6; 32]);
        assert_eq!(
            guard.check_snapshot(&progress, 9),
            Err(Error::InvalidReconciliation)
        );
    }

    #[test]
    fn invalid_native_inputs_are_rejected_before_dispatch() {
        for (receiver, amount, expected) in [
            (
                ActorId::zero(),
                5u64.into(),
                ReceiptRejectionEvidence::InvalidRecipient,
            ),
            (
                ActorId::from([1; 32]),
                U256::zero(),
                ReceiptRejectionEvidence::InvalidAmount,
            ),
            (
                ActorId::from([1; 32]),
                U256::one() << 80,
                ReceiptRejectionEvidence::NativeAmountOverflow,
            ),
        ] {
            let mut deposit = original_deposit();
            deposit.receiver = receiver;
            deposit.amount = amount;
            assert!(matches!(
                initial_deposit_status(&deposit),
                DepositStatus::Rejected(reason) if reason == expected
            ));
        }
    }

    #[test]
    fn native_amount_boundary_does_not_restrict_non_native_inputs() {
        let mut deposit = original_deposit();
        deposit.amount = (U256::one() << 80) - U256::one();
        assert!(matches!(
            initial_deposit_status(&deposit),
            DepositStatus::Pending
        ));
        deposit.amount += U256::one();
        assert!(matches!(
            initial_deposit_status(&deposit),
            DepositStatus::Rejected(ReceiptRejectionEvidence::NativeAmountOverflow)
        ));
        deposit.native = false;
        assert!(matches!(
            initial_deposit_status(&deposit),
            DepositStatus::Pending
        ));
    }

    #[test]
    fn historical_native_absence_evidence_keeps_discriminant_four() {
        let child = MessageId::from([4; 32]);
        let mut encoded = vec![4];
        child.encode_to(&mut encoded);
        assert_eq!(
            ReceiptRejectionEvidence::decode(&mut encoded.as_slice()).unwrap(),
            ReceiptRejectionEvidence::NativeErrorAbsent { child }
        );
    }

    #[test]
    fn historical_rejections_cannot_authorize_dispatched_invalid_inputs_or_absence() {
        let child = MessageId::from([4; 32]);
        let mut log = ReceiptLogProgress {
            deposit: original_deposit(),
            status: DepositStatus::Rejected(ReceiptRejectionEvidence::NativeErrorAbsent { child }),
            child: Some(child),
        };
        for original_child in [None, Some(child)] {
            log.child = original_child;
            assert_eq!(
                validate_retained_rejection(&log),
                Err(Error::InvalidReconciliation)
            );
        }
        for reason in [
            ReceiptRejectionEvidence::InvalidRecipient,
            ReceiptRejectionEvidence::InvalidAmount,
            ReceiptRejectionEvidence::NativeAmountOverflow,
        ] {
            log.status = DepositStatus::Rejected(reason);
            log.child = Some(child);
            assert_eq!(
                validate_retained_rejection(&log),
                Err(Error::InvalidReconciliation)
            );
            log.child = None;
            assert_eq!(validate_retained_rejection(&log), Ok(()));
        }
        log.status = DepositStatus::Rejected(ReceiptRejectionEvidence::UnlockFalse { child });
        log.child = Some(child);
        assert_eq!(validate_retained_rejection(&log), Ok(()));
    }
}
