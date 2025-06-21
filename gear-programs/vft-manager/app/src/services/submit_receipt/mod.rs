use collections::{btree_map::BTreeMap, btree_set::BTreeSet};
use gstd::{static_mut, static_ref};
use sails_rs::prelude::*;

use super::{error::Error, TokenSupply, VftManager};

pub mod abi;
pub mod token_operations;

/// Successfully processed Ethereum transactions. They're stored to prevent
/// double-spending attacks on this program.
static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

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

/// Get reference to a transactions storage.
pub fn reply_statuses() -> &'static BTreeMap<(u64, u64), Result<(), Error>> {
    unsafe { static_ref!(REPLY_STATUSES).as_ref() }.expect("Program should be constructed")
}

/// Get mutable reference to a transactions storage.
pub fn reply_statuses_mut() -> &'static mut BTreeMap<(u64, u64), Result<(), Error>> {
    unsafe { static_mut!(REPLY_STATUSES).as_mut() }.expect("Program should be constructed")
}

/// Initialize state that's used by this VFT Manager method.
pub fn seed() {
    unsafe {
        TRANSACTIONS = Some(BTreeSet::new());
        REPLY_STATUSES = Some(BTreeMap::new());
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

    let state = service.state();
    let Some(erc20_manager_address) = state.erc20_manager_address else {
        panic!("Address of the ERC20Manger is not set");
    };

    let sender = Syscall::message_source();
    if sender != state.historical_proxy_address {
        return Err(Error::NotHistoricalProxy);
    }

    let receipt =
        ReceiptEnvelope::decode(&mut &receipt_rlp[..]).map_err(|_| Error::NotSupportedEvent)?;

    if !receipt.is_success() {
        return Err(Error::NotSupportedEvent);
    }

    // Decode log and check that it is from an allowed address.
    let (vara_token_id, event) = receipt
        .logs()
        .iter()
        .find_map(|log| {
            let address = H160::from(log.address.0 .0);
            let event = abi::ERC20_MANAGER::BridgingRequested::decode_raw_log_validate(
                log.topics(),
                &log.data.data,
            )
            .ok()?;
            let eth_token_id = H160::from(event.token.0 .0);
            let vara_token_id = service
                .state()
                .token_map
                .get_vara_token_id(&eth_token_id)
                .ok()?;

            (erc20_manager_address == address).then_some((vara_token_id, event))
        })
        .ok_or(Error::NotSupportedEvent)?;

    let transactions = transactions_mut();
    let key = (slot, transaction_index);
    if transactions.contains(&key) {
        return Err(Error::AlreadyProcessed);
    }

    if transactions.len() >= TX_HISTORY_DEPTH
        && transactions
            .first()
            .map(|first| &key < first)
            .unwrap_or(false)
    {
        return Err(Error::TransactionTooOld);
    }

    let amount = U256::from_little_endian(event.amount.as_le_slice());
    let receiver = ActorId::from(event.to.0);

    match service.state().token_map.get_supply_type(&vara_token_id)? {
        TokenSupply::Ethereum => {
            token_operations::mint(
                slot,
                transaction_index,
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
                vara_token_id,
                receiver,
                amount,
                service.config(),
            )
            .await
        }
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
