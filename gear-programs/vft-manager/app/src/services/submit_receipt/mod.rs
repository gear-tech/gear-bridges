use collections::btree_set::BTreeSet;
use sails_rs::{gstd::ExecContext, prelude::*};

use super::{error::Error, TokenSupply, VftManager};

pub mod abi;
pub mod token_operations;

/// Successfully processed Ethereum transactions. They're stored to prevent
/// double-spending attacks on this program.
static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;

/// Maximum amount of successfully processed Ethereum transactions that this
/// program can store.
const TX_HISTORY_DEPTH: usize = 50_000_000;

/// Get mutable reference to a transactions storage.
pub fn transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe {
        TRANSACTIONS
            .as_mut()
            .expect("Program should be constructed")
    }
}

/// Initialize state that's used by this VFT Manager method.
pub fn seed() {
    unsafe {
        TRANSACTIONS = Some(BTreeSet::new());
    }
}

/// Submit rlp-encoded transaction receipt.
///
/// This receipt is decoded under the hood and checked that it's a valid receipt from tx
/// sent to `ERC20Manager` contract. Also it will check that this transaction haven't been
/// processed yet.
///
/// This method can be called only by [State::historical_proxy_address] program.
pub async fn submit_receipt<T: ExecContext>(
    service: &mut VftManager<T>,
    slot: u64,
    transaction_index: u64,
    receipt_rlp: Vec<u8>,
) -> Result<(), Error> {
    use alloy_rlp::Decodable;
    use alloy_sol_types::SolEvent;
    use ethereum_common::utils::ReceiptEnvelope;

    let state = service.state();
    let sender = service.exec_context.actor_id();

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
            let event = abi::ERC20_MANAGER::BridgingRequested::decode_log_data(log, true).ok()?;
            let eth_token_id = H160::from(event.token.0 .0);
            let vara_token_id = service
                .state()
                .token_map
                .get_vara_token_id(&eth_token_id)
                .ok()?;

            (service.erc20_manager_address() == address).then_some((vara_token_id, event))
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
            token_operations::mint(slot, transaction_index, vara_token_id, receiver, amount, service.config())
                .await
        }

        TokenSupply::Gear => {
            token_operations::unlock(slot, transaction_index, vara_token_id, receiver, amount, service.config())
                .await
        }
    }
}

pub fn fill_transactions(
) -> bool {
    let transactions = transactions_mut();
    if TX_HISTORY_DEPTH == transactions.len() {
        return false;
    }

    let count = cmp::min(TX_HISTORY_DEPTH - transactions.len(), super::SIZE_FILL_TRANSACTIONS_STEP);
    let (last, _) = transactions.last().copied().unwrap();
    for i in 0..count {
        transactions.insert((last + 1, i as u64));
    }

    true
}
