use collections::{btree_map::BTreeMap, btree_set::BTreeSet};
use gstd::{static_mut, static_ref};
use sails_rs::prelude::*;

use super::{error::Error, TokenSupply, VftManager};

pub mod abi;
pub mod token_operations;

/// Successfully processed Ethereum transactions. They're stored to prevent
/// double-spending attacks on this program.
static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Receipt keys whose token operation has not reached a definite outcome yet.
/// Reservations are kept separate so completed-history eviction cannot remove them.
static mut RESERVED_TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Receipt logs whose token operation completed successfully, keyed by
/// `(slot, transaction_index, log_index)`.
///
/// SECURITY(H-1): this is the per-log identity that the tx-level `TRANSACTIONS`
/// history cannot express: one Ethereum transaction may lock several deposits
/// and each `BridgingRequested` log needs its own mint record.
static mut PROCESSED_LOGS: Option<BTreeSet<(u64, u64, u64)>> = None;

/// Logs of already-seen transactions that carry no mint record and therefore
/// require operator action (recovery via the admin import APIs). They are
/// surfaced by `submit_receipt` as a `BackloggedTransfer` failure instead of a
/// silent success.
static mut BACKLOGGED_LOGS: Option<BTreeSet<(u64, u64, u64)>> = None;

/// Maximum amount of successfully processed Ethereum transactions that this
/// program can store.
pub const TX_HISTORY_DEPTH: usize = 50_000_000;

/// A temporary storage for reply statuses. Tracks the status of `handle_reply` hook invocations.
/// Maps a `(slot, transaction_index)` pair to a `Result<(), Error>`.
static mut REPLY_STATUSES: Option<BTreeMap<(u64, u64), Result<(), Error>>> = None;

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

/// Get a reference to the reply statuses global map.
pub fn reply_statuses() -> &'static BTreeMap<(u64, u64), Result<(), Error>> {
    unsafe { static_ref!(REPLY_STATUSES).as_ref() }.expect("Program should be constructed")
}

/// Get a mutable reference to the reply statuses global map.
pub fn reply_statuses_mut() -> &'static mut BTreeMap<(u64, u64), Result<(), Error>> {
    unsafe { static_mut!(REPLY_STATUSES).as_mut() }.expect("Program should be constructed")
}

/// Get a reference to the per-log mint records.
pub fn processed_logs() -> &'static BTreeSet<(u64, u64, u64)> {
    unsafe { static_ref!(PROCESSED_LOGS).as_ref() }.expect("Program should be constructed")
}

/// Get a mutable reference to the per-log mint records.
pub fn processed_logs_mut() -> &'static mut BTreeSet<(u64, u64, u64)> {
    unsafe { static_mut!(PROCESSED_LOGS).as_mut() }.expect("Program should be constructed")
}

/// Get a reference to the actionable backlogged-transfer log keys.
pub fn backlogged_logs() -> &'static BTreeSet<(u64, u64, u64)> {
    unsafe { static_ref!(BACKLOGGED_LOGS).as_ref() }.expect("Program should be constructed")
}

/// Get a mutable reference to the actionable backlogged-transfer log keys.
pub fn backlogged_logs_mut() -> &'static mut BTreeSet<(u64, u64, u64)> {
    unsafe { static_mut!(BACKLOGGED_LOGS).as_mut() }.expect("Program should be constructed")
}

/// Build the per-log receipt identity `(slot, transaction_index, log_index)`.
pub fn log_key(key: (u64, u64), log_index: u64) -> (u64, u64, u64) {
    (key.0, key.1, log_index)
}

/// Move a successful receipt reservation into bounded processed history.
pub fn complete_transaction(key: (u64, u64)) {
    complete_transaction_in(
        transactions_mut(),
        reserved_transactions_mut(),
        key,
        TX_HISTORY_DEPTH,
    );
}

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
        PROCESSED_LOGS = Some(BTreeSet::new());
        BACKLOGGED_LOGS = Some(BTreeSet::new());
    }
}

/// Submit rlp-encoded transaction receipt.
///
/// This receipt is decoded under the hood and checked that it's a valid receipt from tx
/// sent to `ERC20Manager` contract. Every `BridgingRequested` log of the receipt is
/// processed under its own `(slot, transaction_index, log_index)` identity, exactly once
/// per log.
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

    let state = service.state();
    let Some(erc20_manager_address) = state.erc20_manager_address else {
        panic!("Address of the ERC20Manger is not set");
    };

    let sender = Syscall::message_source();
    if sender != state.historical_proxy_address {
        return Err(Error::NotHistoricalProxy);
    }

    let receipt =
        ReceiptEnvelope::decode(&mut &receipt_rlp[..]).map_err(|_| Error::UnsupportedEthEvent)?;

    if !receipt.is_success() {
        return Err(Error::UnsupportedEthEvent);
    }

    // SECURITY(H-1): receipt identity is per-log, `(slot, transaction_index,
    // log_index)`, not per-transaction: one Ethereum tx can lock several deposits
    // (composable callers / Multicall3), and each `BridgingRequested` log must be
    // deliverable exactly once. Collect every matching log instead of selecting
    // only the first one.
    let mut transfers = Vec::new();
    for (log_index, log) in receipt.logs().iter().enumerate() {
        // Decode log and check that it is from an allowed address.
        let address = H160::from(log.address.0 .0);
        if address != erc20_manager_address {
            continue;
        }
        let Ok(event) = abi::ERC20_MANAGER::BridgingRequested::decode_raw_log_validate(
            log.topics(),
            &log.data.data,
        ) else {
            continue;
        };
        let eth_token_id = H160::from(event.token.0 .0);
        let Ok(vara_token_id) = service.state().token_map.get_vara_token_id(&eth_token_id) else {
            continue;
        };
        transfers.push((log_index as u64, vara_token_id, event));
    }

    if transfers.is_empty() {
        return Err(Error::UnsupportedEthEvent);
    }

    let key = (slot, transaction_index);

    // Logs of this tx that do not carry a completed-mint record yet.
    let missing: Vec<u64> = transfers
        .iter()
        .map(|(log_index, ..)| *log_index)
        .filter(|log_index| !processed_logs().contains(&log_key(key, *log_index)))
        .collect();

    if transactions().contains(&key) || reserved_transactions().contains(&key) {
        if missing.is_empty() {
            // Every log was minted by an earlier submission: definite duplicate.
            return Err(Error::AlreadyProcessed);
        }

        // SECURITY(H-1): a duplicate tx whose logs carry no mint record must not
        // resolve to `AlreadyProcessed` - the relayer maps that to a silent
        // message Success and the deposits would be lost. Re-minting them
        // in-line is not safe either (the tx-level reservation cannot tell a
        // pre-fix minted log from an in-flight one), so park them as an
        // actionable backlogged transfer and report a failure instead.
        let backlog = backlogged_logs_mut();
        for log_index in &missing {
            backlog.insert(log_key(key, *log_index));
        }
        return Err(Error::Internal(format!(
            "BackloggedTransfer: tx ({slot}, {transaction_index}) logs without mint record: {missing:?}"
        )));
    }

    if transactions().len() >= TX_HISTORY_DEPTH
        && transactions()
            .first()
            .map(|first| &key < first)
            .unwrap_or(false)
    {
        return Err(Error::TransactionTooOld);
    }

    // Reserve before the first await so concurrent submissions cannot both
    // pass the replay check. Success moves the key into bounded history; a
    // definite failure releases it, while an ambiguous result keeps it reserved.
    reserved_transactions_mut().insert(key);

    // Process each not-yet-processed log. The mint lifecycle is unchanged:
    // every operation awaits its own reply before the next one is dispatched,
    // so the `(slot, transaction_index)`-keyed reply bookkeeping inside
    // `token_operations` stays collision-free.
    let mut first_error = None;
    for (log_index, vara_token_id, event) in transfers {
        let lkey = log_key(key, log_index);
        if processed_logs().contains(&lkey) {
            // Already minted: skip silently.
            backlogged_logs_mut().remove(&lkey);
            continue;
        }

        let amount = U256::from_little_endian(event.amount.as_le_slice());
        let receiver = ActorId::from(event.to.0);
        let erc20_sender = H160::from(event.from.0 .0);

        let result = match service.state().token_map.get_supply_type(&vara_token_id)? {
            TokenSupply::Ethereum => {
                token_operations::mint(
                    slot,
                    transaction_index,
                    erc20_sender,
                    vara_token_id,
                    receiver,
                    amount,
                    service.config(),
                )
                .await
            }

            TokenSupply::Gear => {
                token_operations::unlock(
                    slot,
                    transaction_index,
                    erc20_sender,
                    vara_token_id,
                    receiver,
                    amount,
                    service.config(),
                )
                .await
            }
        };

        match result {
            Ok(()) => {
                let processed = processed_logs_mut();
                processed.insert(lkey);
                if processed.len() > TX_HISTORY_DEPTH {
                    processed.pop_first();
                }
                backlogged_logs_mut().remove(&lkey);
            }
            // No mint record was written, so a later submission still surfaces
            // this log as a backlogged transfer. Report the failure without
            // swallowing the remaining logs of the tx.
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    first_error.map_or(Ok(()), Err)
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
}
