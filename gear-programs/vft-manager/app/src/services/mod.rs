use sails_rs::{gstd::ExecContext, prelude::*};

mod error;
mod token_mapping;

use error::Error;
use token_mapping::TokenMap;

mod request_bridging;
mod submit_receipt;

pub use submit_receipt::abi as eth_abi;

/// VFT Manager service.
pub struct VftManager<ExecContext> {
    exec_context: ExecContext,
}

/// Type of the token supply.
#[derive(Debug, Decode, Encode, TypeInfo, Clone, Copy)]
#[repr(u8)]
pub enum TokenSupply {
    /// Token supply is located on Ethereum.
    ///
    /// This means that we're working with some pre-existing `ERC20` token on Ethereum and with
    /// wrapped `VFT` token on Gear.
    ///
    /// When this type of token supply is activated corresponding tokens will be minted/burned
    /// on the gear side and locked/unlocked on the Ethereum side.
    ///
    /// For example this type of token supply can be used to work with
    /// `USDT ERC20 token`/`wrappedUSDT VFT token` pair.
    Ethereum = 0,
    /// Token supply is located on Gear.
    ///
    /// This means that we're working with some pre-existing `VFT` token on Gear and with
    /// wrapped `ERC20` token on Ethereum.
    ///
    /// When this type of token supply is activated corresponding tokens will be locked/unlocked
    /// on the gear side and minted/burned on the Gear side.
    ///
    /// For example this type of token supply can be used to work with
    /// `VARA VFT token`/`wrappedVARA ERC20 token` pair.
    Gear = 1,
}

/// Events emitted by VFT Manager service.
#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    /// Token mapping was added.
    ///
    /// This means that VFT Manager service now supports specified
    /// [vara_token_id](Event::TokenMappingAdded::vara_token_id)/[eth_token_id](Event::TokenMappingAdded::eth_token_id) pair.
    TokenMappingAdded {
        /// `VFT` token address that was added into mapping.
        vara_token_id: ActorId,
        /// `ERC20` token address that was added into mapping.
        eth_token_id: H160,
    },
    /// Token mapping was removed.
    ///
    /// This means that VFT Manager service doesn't support specified
    /// [vara_token_id](Event::TokenMappingRemoved::vara_token_id)/[eth_token_id](Event::TokenMappingRemoved::eth_token_id)
    /// pair anymore.
    TokenMappingRemoved {
        /// `VFT` token address that was removed from mapping.
        vara_token_id: ActorId,
        /// `ERC20` token address that was removed from mapping.
        eth_token_id: H160,
    },
    /// Bridging of tokens from Gear to Ethereum was requested.
    ///
    /// When this event is emitted it means that `VFT` tokens were locked/burned and
    /// a message to the gear-eth-bridge built-in actor was successfully submitted.
    BridgingRequested {
        /// Nonce that gear-eth-bridge built-in actor have returned.
        nonce: U256,
        /// `VFT` token address that was locked/burned.
        vara_token_id: ActorId,
        /// Amount of tokens that should be bridged.
        amount: U256,
        /// Original token owner on the Gear side.
        sender: ActorId,
        /// Receiver of the tokens on the Ethereum side.
        receiver: H160,
    },
}

static mut STATE: Option<State> = None;
static mut CONFIG: Option<Config> = None;

/// State of the VFT Manager service.
#[derive(Debug, Default)]
pub struct State {
    /// Address of the gear-eth-bridge built-in actor.
    gear_bridge_builtin: ActorId,
    /// Governance of this program. This address is in the charge of:
    /// - Changing [Config]
    /// - Updating [State::erc20_manager_address]
    /// - Updating [State::historical_proxy_address]
    /// - Managing token mapping in [State::token_map]
    admin: ActorId,
    /// Address of the `ERC20Manager` contract address on Ethereum.
    ///
    /// Can be adjusted by the [State::admin].
    erc20_manager_address: H160,
    /// Mapping between `VFT` and `ERC20` tokens.
    ///
    /// Can be adjusted by the [State::admin].
    token_map: TokenMap,
    /// Address of the `historical-proxy` program.
    ///
    /// VFT Manager service will only accept incoming requests on token withdrawals
    /// from this address.
    ///
    /// Can be adjusted by the [State::admin].
    historical_proxy_address: ActorId,
}

/// Config that should be provided to this service on initialization.
#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    /// Address of the `ERC20Manager` contract on ethereum.
    ///
    /// For more info see [State::erc20_manager_address].
    pub erc20_manager_address: H160,
    /// Address of the gear-eth-bridge built-in actor.
    pub gear_bridge_builtin: ActorId,
    /// Address of the `historical-proxy` program.
    ///
    /// For more info see [State::historical_proxy_address].
    pub historical_proxy_address: ActorId,
    /// Config that will be used to send messages to the other programs.
    ///
    /// For more info see [Config].
    pub config: Config,
}

/// Config that will be used to send messages to the other programs.
#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    /// Gas limit for token operations. Token operations include:
    /// - Mint
    /// - Burn
    /// - TransferFrom
    gas_for_token_ops: u64,
    /// Gas to reserve for reply processing.
    gas_for_reply_deposit: u64,
    /// Gas limit for gear-eth-bridge built-in actor request.
    gas_to_send_request_to_builtin: u64,
    /// Timeout in blocks that current program will wait for reply from
    /// the other programs such as `extended-vft` and `gear-eth-bridge` built-in actor.
    reply_timeout: u32,
}

/// VFT Manager service implementation.
#[service(events = Event)]
impl<T> VftManager<T>
where
    T: ExecContext,
{
    /// Change [State::erc20_manager_address]. Can be called only by a [State::admin].
    pub fn update_erc20_manager_address(&mut self, new_erc20_manager_address: H160) {
        self.ensure_admin();

        self.state_mut().erc20_manager_address = new_erc20_manager_address;
    }

    /// Change [State::historical_proxy_address]. Can be called only by a [State::admin].
    pub fn update_historical_proxy_address(&mut self, historical_proxy_address_new: ActorId) {
        self.ensure_admin();

        self.state_mut().historical_proxy_address = historical_proxy_address_new;
    }

    /// Add a new token pair to a [State::token_map]. Can be called only by a [State::admin].
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

    /// Remove the token pair from [State::token_map]. Can be called only by a [State::admin].
    pub fn remove_vara_to_eth_address(&mut self, vara_token_id: ActorId) {
        self.ensure_admin();

        let eth_token_id = self.state_mut().token_map.remove(vara_token_id);

        self.notify_on(Event::TokenMappingRemoved {
            vara_token_id,
            eth_token_id,
        })
        .expect("Failed to emit event");
    }

    /// Change [Config]. Can be called only by a [State::admin].
    ///
    /// For more info see [Config] docs.
    pub fn update_config(&mut self, config: Config) {
        self.ensure_admin();

        unsafe {
            CONFIG = Some(config);
        }
    }

    /// Ensure that message sender is a [State::admin].
    fn ensure_admin(&self) {
        if self.state().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
    }

    /// Submit rlp-encoded transaction receipt.
    ///
    /// This receipt is decoded under the hood and checked that it's a valid receipt from tx
    /// sent to `ERC20Manager` contract.
    ///
    /// This method can be called only by [State::historical_proxy_address] program.
    pub async fn submit_receipt(
        &mut self,
        slot: u64,
        transaction_index: u64,
        receipt_rlp: Vec<u8>,
    ) -> Result<(), Error> {
        submit_receipt::submit_receipt(self, slot, transaction_index, receipt_rlp).await
    }

    /// Request bridging of tokens from Gear to Ethereum.
    ///
    /// Allowance should allow current program to spend `amount` tokens from the `sender` address.
    pub async fn request_bridging(
        &mut self,
        sender: ActorId,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
    ) -> Result<(U256, H160), Error> {
        request_bridging::request_bridging(self, sender, vara_token_id, amount, receiver).await
    }

    /// Process message further if some error was encountered during the `request_bridging`.
    ///
    /// This method should be called only to recover funds that were stuck in the middle of the bridging
    /// and is not a part of a normal workflow.
    ///
    /// There can be several reasons for `request_bridging` to fail:
    /// - Gas attached to a message wasn't enough to execute entire logic in `request_bridging`.
    /// - Network was heavily loaded and some message was stuck so `request_bridging` failed.
    pub async fn handle_request_bridging_interrupted_transfer(
        &mut self,
        msg_id: MessageId,
    ) -> Result<(), Error> {
        request_bridging::handle_interrupted_transfer(self, msg_id).await
    }

    /// Process message further if some error was encountered during the `submit_receipt`.
    ///
    /// This method should be called only to recover funds that were stuck in the middle of the bridging
    /// and is not a part of a normal workflow.
    ///
    /// There can be several reasons for `submit_receipt` to fail:
    /// - Gas attached to a message wasn't enough to execute entire logic in `submit_receipt`.
    /// - Network was heavily loaded and some message was stuck so `submit_receipt` failed.
    pub async fn handle_submit_receipt_interrupted_transfer(
        &mut self,
        msg_id: MessageId,
    ) -> Result<(), Error> {
        submit_receipt::handle_interrupted_transfer(self, msg_id).await
    }

    /// Get state of a `request_bridging` message tracker.
    pub fn request_briding_msg_tracker_state(
        &self,
    ) -> Vec<(MessageId, request_bridging::MsgTrackerMessageInfo)> {
        request_bridging::msg_tracker_state()
    }

    /// Get state of a `submit_receipt` message tracker.
    pub fn submit_receipt_msg_tracker_state(
        &self,
    ) -> Vec<(MessageId, submit_receipt::MsgTrackerMessageInfo)> {
        submit_receipt::msg_tracker_state()
    }

    /// Get current [token mapping](State::token_map).
    pub fn vara_to_eth_addresses(&self) -> Vec<(ActorId, H160, TokenSupply)> {
        self.state().token_map.read_state()
    }

    /// Get current [State::erc20_manager_address] address.
    pub fn erc20_manager_address(&self) -> H160 {
        self.state().erc20_manager_address
    }

    /// Get current [State::gear_bridge_builtin] address.
    pub fn gear_bridge_builtin(&self) -> ActorId {
        self.state().gear_bridge_builtin
    }

    /// Get current [State::admin] address.
    pub fn admin(&self) -> ActorId {
        self.state().admin
    }

    /// Get current [Config].
    pub fn get_config(&self) -> Config {
        self.config().clone()
    }

    /// Get current [State::historical_proxy_address].
    pub fn historical_proxy_address(&self) -> ActorId {
        self.state().historical_proxy_address
    }
}

impl<T> VftManager<T>
where
    T: ExecContext,
{
    /// Initialize VFT Manager service.
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

    /// Create VFT Manager service.
    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    /// Get a reference to the global [State].
    fn state(&self) -> &State {
        unsafe { STATE.as_ref().expect("VftManager::seed() should be called") }
    }

    /// Get a mutable reference to the global [State].
    fn state_mut(&mut self) -> &mut State {
        unsafe { STATE.as_mut().expect("VftManager::seed() should be called") }
    }

    /// Get a reference to the global [Config].
    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("VftManager::seed() should be called")
        }
    }
}
