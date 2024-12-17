use collections::btree_set::BTreeSet;
use sails_rs::{gstd::ExecContext, prelude::*};

use super::{error::Error, TokenSupply, VftManager};

pub mod abi;
mod msg_tracker;
mod token_operations;

use msg_tracker::{msg_tracker_mut, MessageStatus, TxDetails};

pub use msg_tracker::{msg_tracker_state, MessageInfo as MsgTrackerMessageInfo};

pub(crate) static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;
const CAPACITY: usize = 500_000;

pub(crate) fn transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe {
        TRANSACTIONS
            .as_mut()
            .expect("Program should be constructed")
    }
}

pub fn seed() {
    msg_tracker::init();

    unsafe {
        TRANSACTIONS = Some(BTreeSet::new());
    }
}

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
        return Err(Error::NotEthClient);
    }

    let receipt =
        ReceiptEnvelope::decode(&mut &receipt_rlp[..]).map_err(|_| Error::NotSupportedEvent)?;

    if !receipt.is_success() {
        return Err(Error::NotSupportedEvent);
    }

    // decode log and check that it is from an allowed address
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

    if transactions.len() >= CAPACITY
        && transactions
            .first()
            .map(|first| &key < first)
            .unwrap_or(false)
    {
        return Err(Error::TransactionTooOld);
    }

    let msg_id = gstd::msg::id();
    let amount = U256::from_little_endian(event.amount.as_le_slice());
    let receiver = ActorId::from(event.to.0);
    let supply_type = service.state().token_map.get_supply_type(&vara_token_id)?;
    let transaction_details = TxDetails {
        vara_token_id,
        receiver,
        amount,
        token_supply: supply_type,
    };

    if transactions.len() >= CAPACITY {
        transactions.pop_first();
    }
    transactions.insert((slot, transaction_index));

    msg_tracker_mut().insert_message_info(
        msg_id,
        MessageStatus::SendingMessageToWithdrawTokens,
        transaction_details,
    );
    set_critical_hook(msg_id);

    match supply_type {
        TokenSupply::Ethereum => {
            token_operations::mint(vara_token_id, receiver, amount, service.config(), msg_id)
                .await?;
        }
        TokenSupply::Gear => {
            token_operations::unlock(vara_token_id, receiver, amount, service.config(), msg_id)
                .await?;
        }
    }

    msg_tracker_mut().check_withdraw_result(&msg_id)
}

pub async fn handle_interrupted_transfer<T: ExecContext>(
    service: &mut VftManager<T>,
    msg_id: MessageId,
) -> Result<(U256, H160), Error> {
    let config = service.config();
    let msg_tracker = msg_tracker_mut();

    let msg_info = msg_tracker
        .get_message_info(&msg_id)
        .expect("Unexpected: msg status does not exist");

    let TxDetails {
        vara_token_id,
        amount,
        receiver,
        token_supply,
    } = msg_info.details;

    match msg_info.status {
        MessageStatus::SendingMessageToWithdrawTokens | MessageStatus::TokenWithdrawFailed => {
            msg_tracker_mut()
                .update_message_status(msg_id, MessageStatus::SendingMessageToWithdrawTokens);
            set_critical_hook(msg_id);

            match token_supply {
                TokenSupply::Ethereum => {
                    token_operations::mint(vara_token_id, receiver, amount, config, msg_id).await?;
                }
                TokenSupply::Gear => {
                    token_operations::unlock(vara_token_id, receiver, amount, config, msg_id)
                        .await?;
                }
            }

            msg_tracker_mut().check_withdraw_result(&msg_id)?;

            // TODO: Not refunded, just minted.
            Err(Error::TokensRefunded)
        }
        MessageStatus::WaitingReplyFromTokenWithdrawMessage
        | MessageStatus::TokenWithdrawCompleted => {
            panic!("Unexpected status or transaction completed.")
        }
    }
}

fn set_critical_hook(msg_id: MessageId) {
    gstd::critical::set_hook(move || {
        let msg_tracker = msg_tracker_mut();
        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg info does not exist");

        match msg_info.status {
            MessageStatus::SendingMessageToWithdrawTokens => {
                msg_tracker.update_message_status(
                    msg_id,
                    MessageStatus::WaitingReplyFromTokenWithdrawMessage,
                );
            }
            _ => {}
        };
    });
}
