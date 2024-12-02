use bridge_builtin_operations::Payload;
use collections::btree_set::BTreeSet;
use sails_rs::{gstd::ExecContext, prelude::*};

pub mod abi;
mod bridge_builtin_operations;
pub mod error;
pub mod msg_tracker;
mod utils;
use error::Error;
use msg_tracker::{MessageInfo, MessageStatus, MessageTracker, TxDetails};
use token_mapping::TokenMap;
mod token_mapping;
mod token_operations;

pub(crate) static mut TRANSACTIONS: Option<BTreeSet<(u64, u64)>> = None;
const CAPACITY: usize = 500_000;

pub(crate) fn transactions_mut() -> &'static mut BTreeSet<(u64, u64)> {
    unsafe {
        TRANSACTIONS
            .as_mut()
            .expect("Program should be constructed")
    }
}

pub struct VftManager<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
#[repr(u8)]
pub enum TokenSupply {
    Ethereum = 0,
    Gear = 1,
}

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    TokenMappingAdded {
        vara_token_id: ActorId,
        eth_token_id: H160,
    },
    TokenMappingRemoved {
        vara_token_id: ActorId,
        eth_token_id: H160,
    },
    BridgingRequested {
        nonce: U256,
        vara_token_id: ActorId,
        amount: U256,
        sender: ActorId,
        receiver: H160,
    },
}

static mut STATE: Option<State> = None;
static mut CONFIG: Option<Config> = None;
static mut MSG_TRACKER: Option<MessageTracker> = None;

#[derive(Debug, Default)]
pub struct State {
    gear_bridge_builtin: ActorId,
    admin: ActorId,
    erc20_manager_address: H160,
    token_map: TokenMap,
    eth_client: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    pub erc20_manager_address: H160,
    pub gear_bridge_builtin: ActorId,
    pub eth_client: ActorId,
    pub config: Config,
}

impl InitConfig {
    pub fn new(
        erc20_manager_address: H160,
        gear_bridge_builtin: ActorId,
        eth_client: ActorId,
        config: Config,
    ) -> Self {
        Self {
            erc20_manager_address,
            gear_bridge_builtin,
            eth_client,
            config,
        }
    }
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    gas_for_token_ops: u64,
    gas_for_reply_deposit: u64,
    gas_for_submit_receipt: u64,
    gas_to_send_request_to_builtin: u64,
    reply_timeout: u32,
    gas_for_request_bridging: u64,
}

#[service(events = Event)]
impl<T> VftManager<T>
where
    T: ExecContext,
{
    pub fn update_erc20_manager_address(&mut self, new_erc20_manager_address: H160) {
        self.ensure_admin();

        self.state_mut().erc20_manager_address = new_erc20_manager_address;
    }

    pub fn update_eth_client(&mut self, eth_client_new: ActorId) {
        self.ensure_admin();

        self.state_mut().eth_client = eth_client_new;
    }

    pub fn map_vara_to_eth_address(
        &mut self,
        vara_token_id: ActorId,
        eth_token_id: H160,
        supply_type: TokenSupply,
    ) {
        self.ensure_admin();

        self.state_mut()
            .token_map
            .insert(vara_token_id, eth_token_id, supply_type);

        self.notify_on(Event::TokenMappingAdded {
            vara_token_id,
            eth_token_id,
        })
        .expect("Failed to emit event");
    }

    pub fn remove_vara_to_eth_address(&mut self, vara_token_id: ActorId) {
        self.ensure_admin();

        let eth_token_id = self.state_mut().token_map.remove(vara_token_id);

        self.notify_on(Event::TokenMappingRemoved {
            vara_token_id,
            eth_token_id,
        })
        .expect("Failed to emit event");
    }

    pub fn update_config(&mut self, config: Config) {
        self.ensure_admin();

        unsafe {
            CONFIG = Some(config);
        }
    }

    fn ensure_admin(&self) {
        if self.state().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
    }

    /// Submit rlp-encoded transaction receipt. This receipt is decoded under the hood
    /// and checked that it's a valid receipt from tx send to `ERC20Manager` contract.
    /// This entrypoint can be called only by `ethereum-event-client`.
    pub async fn submit_receipt(
        &mut self,
        slot: u64,
        transaction_index: u64,
        receipt_rlp: Vec<u8>,
    ) -> Result<(), Error> {
        use alloy_rlp::Decodable;
        use alloy_sol_types::SolEvent;
        use ethereum_common::utils::ReceiptEnvelope;

        let state = self.state();
        let sender = self.exec_context.actor_id();

        if sender != state.eth_client {
            return Err(Error::NotEthClient);
        }

        let config = self.config();
        if gstd::exec::gas_available()
            < config.gas_for_token_ops
                + config.gas_for_submit_receipt
                + config.gas_for_reply_deposit
        {
            return Err(Error::NotEnoughGas);
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
                let event =
                    abi::ERC20_MANAGER::BridgingRequested::decode_log_data(log, true).ok()?;
                let eth_token_id = H160::from(event.token.0 .0);
                let vara_token_id = self
                    .state()
                    .token_map
                    .get_vara_token_id(&eth_token_id)
                    .ok()?;

                (self.erc20_manager_address() == address).then_some((vara_token_id, event))
            })
            .ok_or(Error::NotSupportedEvent)?;

        let transactions = transactions_mut();
        let key = (slot, transaction_index);
        if transactions.contains(&key) {
            return Err(Error::AlreadyProcessed);
        }

        if CAPACITY <= transactions.len()
            && transactions
                .first()
                .map(|first| &key < first)
                .unwrap_or(false)
        {
            return Err(Error::TransactionTooOld);
        }

        let amount = U256::from_little_endian(event.amount.as_le_slice());
        let receiver = ActorId::from(event.to.0);
        let msg_id = gstd::msg::id();
        let transaction_details = TxDetails::SubmitReceipt {
            vara_token_id,
            receiver,
            amount,
        };
        msg_tracker_mut().insert_message_info(
            msg_id,
            MessageStatus::SendingMessageToMintTokens,
            transaction_details,
        );
        utils::set_critical_hook(msg_id);

        let supply_type = self.state().token_map.get_supply_type(&vara_token_id)?;

        match supply_type {
            TokenSupply::Ethereum => {
                token_operations::mint(vara_token_id, receiver, amount, config, msg_id).await?;
            }
            TokenSupply::Gear => {
                token_operations::unlock(vara_token_id, receiver, amount, config, msg_id).await?;
            }
        }

        if CAPACITY <= transactions.len() {
            transactions.pop_first();
        }
        transactions.insert((slot, transaction_index));
        Ok(())
    }

    /// Request bridging of tokens from gear to ethereum. It involves locking/burning
    /// `vft` tokens (specific operation depends on the token supply type) and sending
    /// request to the bridge built-in actor.
    pub async fn request_bridging(
        &mut self,
        sender: ActorId,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
    ) -> Result<(U256, H160), Error> {
        let state = self.state();
        let msg_id = gstd::msg::id();
        let eth_token_id = self.state().token_map.get_eth_token_id(&vara_token_id)?;
        let supply_type = self.state().token_map.get_supply_type(&vara_token_id)?;
        let config = self.config();

        if gstd::exec::gas_available()
            < config.gas_for_token_ops
                + config.gas_to_send_request_to_builtin
                + config.gas_for_request_bridging
                + 3 * config.gas_for_reply_deposit
        {
            panic!("Please attach more gas");
        }

        match supply_type {
            TokenSupply::Ethereum => {
                token_operations::burn(vara_token_id, sender, receiver, amount, config, msg_id)
                    .await?;
            }
            TokenSupply::Gear => {
                token_operations::lock(vara_token_id, sender, amount, receiver, config, msg_id)
                    .await?;
            }
        }

        let payload = Payload {
            receiver,
            token_id: eth_token_id,
            amount,
        };
        let nonce = match bridge_builtin_operations::send_message_to_bridge_builtin(
            state.gear_bridge_builtin,
            state.erc20_manager_address,
            payload,
            config,
            msg_id,
        )
        .await
        {
            Ok(nonce) => nonce,
            Err(e) => {
                // In case of failure, mint tokens back to the sender
                token_operations::mint(vara_token_id, sender, amount, config, msg_id).await?;
                return Err(e);
            }
        };

        self.notify_on(Event::BridgingRequested {
            nonce,
            vara_token_id,
            amount,
            sender,
            receiver,
        })
        .expect("Failed to emit event");

        Ok((nonce, eth_token_id))
    }

    pub async fn handle_interrupted_transfer(
        &mut self,
        msg_id: MessageId,
    ) -> Result<(U256, H160), Error> {
        let state = self.state();

        let config = self.config();
        let msg_tracker = msg_tracker_mut();

        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg status does not exist");

        let TxDetails::RequestBridging {
            vara_token_id,
            sender,
            amount,
            receiver,
        } = msg_info.details
        else {
            panic!("Wrong message type")
        };

        let eth_token_id = self
            .state()
            .token_map
            .get_eth_token_id(&vara_token_id)
            .expect("Failed to get ethereum token id");

        match msg_info.status {
            MessageStatus::TokenBurnCompleted(true) | MessageStatus::BridgeBuiltinStep => {
                let payload = Payload {
                    receiver,
                    token_id: eth_token_id,
                    amount,
                };

                match bridge_builtin_operations::send_message_to_bridge_builtin(
                    state.gear_bridge_builtin,
                    state.erc20_manager_address,
                    payload,
                    config,
                    msg_id,
                )
                .await
                {
                    Ok(nonce) => Ok((nonce, eth_token_id)),
                    Err(_) => {
                        // In case of failure, mint tokens back to the sender
                        token_operations::mint(vara_token_id, sender, amount, config, msg_id)
                            .await?;
                        Err(Error::TokensRefunded)
                    }
                }
            }
            MessageStatus::BridgeResponseReceived(Some(nonce)) => {
                msg_tracker_mut().remove_message_info(&msg_id);
                Ok((nonce, eth_token_id))
            }
            MessageStatus::MintTokensStep => {
                token_operations::mint(vara_token_id, sender, amount, config, msg_id).await?;
                Err(Error::TokensRefunded)
            }
            _ => {
                panic!("Unexpected status or transaction completed.")
            }
        }
    }

    pub fn msg_tracker_state(&self) -> Vec<(MessageId, MessageInfo)> {
        msg_tracker().message_info.clone().into_iter().collect()
    }

    pub fn vara_to_eth_addresses(&self) -> Vec<(ActorId, H160, TokenSupply)> {
        self.state().token_map.read_state()
    }

    pub fn erc20_manager_address(&self) -> H160 {
        self.state().erc20_manager_address
    }

    pub fn gear_bridge_builtin(&self) -> ActorId {
        self.state().gear_bridge_builtin
    }

    pub fn admin(&self) -> ActorId {
        self.state().admin
    }

    pub fn get_config(&self) -> Config {
        self.config().clone()
    }

    pub fn eth_client(&self) -> ActorId {
        self.state().eth_client
    }
}

impl<T> VftManager<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            STATE = Some(State {
                gear_bridge_builtin: config.gear_bridge_builtin,
                erc20_manager_address: config.erc20_manager_address,
                admin: exec_context.actor_id(),
                eth_client: config.eth_client,
                ..Default::default()
            });
            CONFIG = Some(config.config);
            MSG_TRACKER = Some(MessageTracker::default());
        }
    }

    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn state(&self) -> &State {
        unsafe { STATE.as_ref().expect("VftManager::seed() should be called") }
    }

    fn state_mut(&mut self) -> &mut State {
        unsafe { STATE.as_mut().expect("VftManager::seed() should be called") }
    }

    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("VftManager::seed() should be called")
        }
    }
}

fn msg_tracker() -> &'static MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_ref()
            .expect("VftManager::seed() should be called")
    }
}

fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_mut()
            .expect("VftManager::seed() should be called")
    }
}
