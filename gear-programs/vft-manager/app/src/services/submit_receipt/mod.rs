use collections::{btree_map::BTreeMap, btree_set::BTreeSet};
use gstd::{static_mut, static_ref};
use sails_rs::prelude::*;

use super::{error::Error, ReceiptStatus, TokenSupply, VftManager};

pub mod abi;
pub mod token_operations;

/// Successfully processed Ethereum transactions. They're stored to prevent
/// double-spending attacks on this program.
static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Receipt keys whose token operation has not reached a definite outcome yet.
/// Reservations are kept separate so completed-history eviction cannot remove them.
static mut RESERVED_TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Maximum amount of successfully processed Ethereum transactions that this
/// program can store.
pub const TX_HISTORY_DEPTH: usize = 50_000_000;

/// A temporary storage for reply statuses. Tracks the status of `handle_reply` hook invocations.
/// Maps a `(slot, transaction_index)` pair to a `Result<(), Error>`.
static mut REPLY_STATUSES: Option<BTreeMap<(u64, u64), Result<(), Error>>> = None;

/// Per-log progress lets one authenticated receipt account for every deposit without
/// repeating effects already completed by an earlier attempt.
static mut RECEIPT_PROGRESS: Option<BTreeMap<(u64, u64), ReceiptProgress>> = None;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReceiptDeposit {
    pub log_index: u64,
    pub sender: H160,
    pub receiver: ActorId,
    pub token_id: ActorId,
    pub eth_token_id: H160,
    pub amount: U256,
    pub supply: TokenSupply,
    pub native: bool,
    pub operation_id: H256,
}

#[derive(Clone)]
pub(super) enum DepositStatus {
    Pending,
    InFlight,
    NativeQueued,
    Processed,
    Retryable(Error),
    Ambiguous(Error),
}

#[derive(Clone)]
pub(super) struct ReceiptLogProgress {
    pub deposit: ReceiptDeposit,
    pub status: DepositStatus,
    pub child: Option<MessageId>,
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
}

#[derive(Clone, Debug, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub struct ReceiptDepositState {
    pub log_index: u64,
    pub sender: H160,
    pub receiver: ActorId,
    pub token_id: ActorId,
    pub eth_token_id: H160,
    pub amount: U256,
    pub supply: TokenSupply,
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
                            DepositStatus::Retryable(_) => ReceiptDepositOutcome::Rejected,
                            DepositStatus::Ambiguous(_) => ReceiptDepositOutcome::Unknown,
                            DepositStatus::NativeQueued => ReceiptDepositOutcome::NativeQueued,
                        },
                    }
                })
                .collect()
        })
        .unwrap_or_default()
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
            vara_token_id: d.token_id,
            sender: d.sender,
            receiver: d.receiver,
            amount: d.amount,
            native: d.native,
        })
        .expect("Failed to emit settled deposit");
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
            progress.deposits[index].status = status;
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

pub(super) fn record_child(key: (u64, u64), log_index: u64, child: MessageId) {
    if let Some(log) = receipt_progress_mut().get_mut(&key).and_then(|p| {
        p.deposits
            .iter_mut()
            .find(|p| p.deposit.log_index == log_index)
    }) {
        log.child = Some(child);
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

/// Reconcile already-dispatched native payouts without replacing the proof request,
/// claiming a generation, or dispatching mint/unlock/redemption again.
pub async fn reconcile_receipt(key: (u64, u64)) -> Result<ReceiptStatus, Error> {
    if receipt_status(key) != ReceiptStatus::Reserved {
        return Ok(receipt_status(key));
    }
    let native = {
        let Some(progress) = receipt_progress_mut().get(&key) else {
            return Ok(ReceiptStatus::Reserved);
        };
        if let Some(error) = progress.deposits.iter().find_map(|p| match &p.status {
            DepositStatus::Ambiguous(error) => Some(error.clone()),
            _ => None,
        }) {
            return Err(error);
        }
        progress
            .deposits
            .iter()
            .filter(|p| p.deposit.native && matches!(p.status, DepositStatus::NativeQueued))
            .map(|p| p.deposit.clone())
            .collect::<Vec<_>>()
    };
    for deposit in native {
        match token_operations::reconcile_native_log(
            key,
            deposit.log_index,
            deposit.sender,
            deposit.token_id,
            deposit.receiver,
            deposit.amount,
            deposit.operation_id,
        )
        .await
        {
            Ok(()) | Err(Error::NativeSettlementPending) => {}
            Err(error) => return Err(error),
        }
    }
    if receipt_progress_mut().get(&key).is_some_and(|p| {
        !p.deposits.is_empty()
            && p.deposits
                .iter()
                .all(|p| matches!(p.status, DepositStatus::Processed))
    }) {
        complete_transaction(key);
    }
    Ok(receipt_status(key))
}

/// Read the retained state for one Ethereum receipt key.
pub fn receipt_status(key: (u64, u64)) -> ReceiptStatus {
    receipt_status_in(transactions(), reserved_transactions(), key)
}

fn receipt_status_in(
    processed: &BTreeSet<(u64, u64)>,
    reserved: &BTreeSet<(u64, u64)>,
    key: (u64, u64),
) -> ReceiptStatus {
    if processed.contains(&key) {
        ReceiptStatus::Processed
    } else if reserved.contains(&key) {
        ReceiptStatus::Reserved
    } else {
        ReceiptStatus::Unknown
    }
}

/// Get a reference to the reply statuses global map.
pub fn reply_statuses() -> &'static BTreeMap<(u64, u64), Result<(), Error>> {
    unsafe { static_ref!(REPLY_STATUSES).as_ref() }.expect("Program should be constructed")
}

/// Get a mutable reference to the reply statuses global map.
pub fn reply_statuses_mut() -> &'static mut BTreeMap<(u64, u64), Result<(), Error>> {
    unsafe { static_mut!(REPLY_STATUSES).as_mut() }.expect("Program should be constructed")
}

/// Move a successful receipt reservation into bounded processed history.
pub fn complete_transaction(key: (u64, u64)) {
    reserved_transactions_mut().remove(&key);
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
        REPLY_STATUSES = Some(BTreeMap::new());
        RECEIPT_PROGRESS = Some(BTreeMap::new());
    }
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
    use alloy_rlp::Decodable;
    use alloy_sol_types::SolEvent;
    use ethereum_common::utils::ReceiptEnvelope;

    let Some(erc20_manager_address) = service.state().erc20_manager_address else {
        panic!("Address of the ERC20Manger is not set");
    };
    if Syscall::message_source() != service.state().historical_proxy_address {
        return Err(Error::NotHistoricalProxy);
    }

    use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
    let receipt_hash = KeccakHasher::hash(&receipt_rlp);
    let mut input = &receipt_rlp[..];
    let receipt = ReceiptEnvelope::decode(&mut input).map_err(|_| Error::UnsupportedEthEvent)?;
    if !input.is_empty() {
        return Err(Error::UnsupportedEthEvent);
    }
    if !receipt.is_success() {
        return Err(Error::UnsupportedEthEvent);
    }

    let key = (slot, transaction_index);
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
                .map_err(|_| Error::UnsupportedEthEvent)?;
            let supply = service
                .state()
                .token_map
                .get_supply_type(&token_id)
                .map_err(|_| Error::UnsupportedEthEvent)?;
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
                native: service.state().native_wrapper == Some(token_id),
                operation_id: KeccakHasher::hash(&operation),
            });
        }
    }
    if deposits.is_empty() {
        return Err(Error::UnsupportedEthEvent);
    }

    if transactions().contains(&key) {
        return Err(Error::AlreadyProcessed);
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
            deposits = progress
                .deposits
                .iter()
                .map(|p| p.deposit.clone())
                .collect();
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
            reserved_transactions_mut().insert(key);
            receipt_progress_mut().insert(
                key,
                ReceiptProgress {
                    deposits: deposits
                        .iter()
                        .cloned()
                        .map(|deposit| ReceiptLogProgress {
                            deposit,
                            status: DepositStatus::Pending,
                            child: None,
                        })
                        .collect(),
                    receipt_hash,
                    generation,
                    lease_until,
                },
            );
        }
    }

    let mut retryable_error = None;
    let mut ambiguous_error = None;
    for deposit in deposits.iter().cloned() {
        if !owns_generation(key, generation) {
            return Err(Error::ReceiptLeaseActive);
        }
        let Some(status) = deposit_status(key, deposit.log_index) else {
            release_receipt_lease(key, generation);
            return Err(Error::Internal("Receipt log progress is missing".into()));
        };
        match &status {
            DepositStatus::Processed => continue,
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

        if !matches!(&status, DepositStatus::NativeQueued) {
            set_deposit_status(key, deposit.log_index, DepositStatus::InFlight);
        }
        let result = match deposit.supply {
            TokenSupply::Ethereum => {
                token_operations::mint_log(key, &deposit, service.config()).await
            }
            TokenSupply::Gear if deposit.native => {
                token_operations::redeem_native_log(key, &deposit, service.config()).await
            }
            TokenSupply::Gear => {
                token_operations::unlock_log(key, &deposit, service.config()).await
            }
        };
        if !owns_generation(key, generation) {
            return Err(Error::ReceiptLeaseActive);
        }
        if let Err(error) = result {
            match deposit_status(key, deposit.log_index) {
                Some(DepositStatus::Retryable(error)) => {
                    retryable_error.get_or_insert(error);
                }
                Some(DepositStatus::Ambiguous(error)) => {
                    ambiguous_error.get_or_insert(error);
                }
                Some(DepositStatus::Processed) => {}
                Some(DepositStatus::NativeQueued) if error == Error::NativeSettlementPending => {}
                Some(DepositStatus::NativeQueued) => {
                    ambiguous_error.get_or_insert(error.clone());
                }
                Some(DepositStatus::InFlight) => {
                    ambiguous_error.get_or_insert(error.clone());
                    set_deposit_status(key, deposit.log_index, DepositStatus::Ambiguous(error));
                }
                Some(DepositStatus::Pending) | None => {
                    ambiguous_error.get_or_insert(error.clone());
                    set_deposit_status(key, deposit.log_index, DepositStatus::Ambiguous(error));
                }
            }
        }
    }

    if let Some(error) = ambiguous_error {
        release_receipt_lease(key, generation);
        return Err(error);
    }
    if let Some(error) = retryable_error {
        release_receipt_lease(key, generation);
        return Err(error);
    }
    if deposits.iter().any(|deposit| {
        !matches!(
            deposit_status(key, deposit.log_index),
            Some(DepositStatus::Processed)
        )
    }) {
        release_receipt_lease(key, generation);
        if deposits.iter().all(|deposit| {
            matches!(
                deposit_status(key, deposit.log_index),
                Some(DepositStatus::Processed | DepositStatus::NativeQueued)
            )
        }) {
            return Err(Error::NativeSettlementPending);
        }
        return Err(Error::Internal(
            "Receipt log processing is incomplete".into(),
        ));
    }

    complete_transaction(key);
    release_receipt_lease(key, generation);
    Ok(())
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
        let key = (2, 7);

        assert_eq!(
            receipt_status_in(&processed, &reserved, key),
            ReceiptStatus::Unknown
        );
        reserved.insert(key);
        assert_eq!(
            receipt_status_in(&processed, &reserved, key),
            ReceiptStatus::Reserved
        );

        complete_transaction_in(&mut processed, &mut reserved, key, 1);
        assert_eq!(
            receipt_status_in(&processed, &reserved, key),
            ReceiptStatus::Processed
        );
        assert_eq!(
            receipt_status_in(&processed, &reserved, (1, 0)),
            ReceiptStatus::Unknown
        );

        complete_transaction_in(&mut processed, &mut reserved, (3, 0), 1);
        assert_eq!(
            receipt_status_in(&processed, &reserved, key),
            ReceiptStatus::Unknown
        );
    }
}
