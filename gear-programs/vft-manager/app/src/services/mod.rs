use sails_rs::{gstd::ExecContext, prelude::*};

mod error;
mod token_mapping;

use error::Error;
use token_mapping::TokenMap;

mod request_bridging;
mod submit_receipt;

pub use submit_receipt::abi as eth_abi;

pub struct VftManager<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone, Copy)]
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

#[derive(Debug, Default)]
pub struct State {
    gear_bridge_builtin: ActorId,
    admin: ActorId,
    erc20_manager_address: H160,
    token_map: TokenMap,
    historical_proxy_address: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    pub erc20_manager_address: H160,
    pub gear_bridge_builtin: ActorId,
    pub historical_proxy_address: ActorId,
    pub config: Config,
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    gas_for_token_ops: u64,
    gas_for_reply_deposit: u64,
    gas_to_send_request_to_builtin: u64,
    reply_timeout: u32,
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

    pub fn update_historical_proxy_address(&mut self, historical_proxy_address_new: ActorId) {
        self.ensure_admin();

        self.state_mut().historical_proxy_address = historical_proxy_address_new;
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
    /// and checked that it's a valid receipt from tx sent to `ERC20Manager` contract.
    /// This entrypoint can be called only by `historical-proxy`.
    pub async fn submit_receipt(
        &mut self,
        slot: u64,
        transaction_index: u64,
        receipt_rlp: Vec<u8>,
    ) -> Result<(), Error> {
        submit_receipt::submit_receipt(self, slot, transaction_index, receipt_rlp).await
    }

    /// Request bridging of tokens from gear to ethereum.
    ///
    /// Allowance should allow `vft-manager` to spend `amount` tokens from the `sender` address.
    pub async fn request_bridging(
        &mut self,
        sender: ActorId,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
    ) -> Result<(U256, H160), Error> {
        request_bridging::request_bridging(self, sender, vara_token_id, amount, receiver).await
    }

    pub async fn handle_request_bridging_interrupted_transfer(
        &mut self,
        msg_id: MessageId,
    ) -> Result<(), Error> {
        request_bridging::handle_interrupted_transfer(self, msg_id).await
    }

    pub async fn handle_submit_receipt_interrupted_transfer(
        &mut self,
        msg_id: MessageId,
    ) -> Result<(), Error> {
        submit_receipt::handle_interrupted_transfer(self, msg_id).await
    }

    pub fn request_briding_msg_tracker_state(
        &self,
    ) -> Vec<(MessageId, request_bridging::MsgTrackerMessageInfo)> {
        request_bridging::msg_tracker_state()
    }

    pub fn submit_receipt_msg_tracker_state(
        &self,
    ) -> Vec<(MessageId, submit_receipt::MsgTrackerMessageInfo)> {
        submit_receipt::msg_tracker_state()
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

    pub fn historical_proxy_address(&self) -> ActorId {
        self.state().historical_proxy_address
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
                historical_proxy_address: config.historical_proxy_address,
                ..Default::default()
            });
            CONFIG = Some(config.config);
        }

        request_bridging::seed();
        submit_receipt::seed();
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
