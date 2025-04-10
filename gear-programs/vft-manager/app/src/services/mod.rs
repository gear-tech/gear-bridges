use vft_client::traits::*;
use sails_rs::{
    calls::*,
    gstd::{self, calls::GStdRemoting, msg, ExecContext},
    prelude::*,
};

mod error;
mod token_mapping;

use error::Error;
use request_bridging::{MessageStatus, TxDetails};
use token_mapping::TokenMap;

mod request_bridging;
pub mod submit_receipt;

pub use submit_receipt::abi as eth_abi;

pub const SIZE_FILL_TRANSACTIONS_STEP: usize = 50_000;

#[derive(Debug, Clone, Decode, TypeInfo)]
pub enum Order {
    Direct,
    Reverse,
}

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
        /// Type of the token supply.
        supply_type: TokenSupply,
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
        /// Type of the token supply.
        supply_type: TokenSupply,
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
    /// Vft-manager was paused by an admin.
    ///
    /// It means that any user requests to it will be rejected.
    Paused,
    /// Vft-manager was unpaused by an admin.
    ///
    /// It means that normal operation is continued after the pause.
    Unpaused,
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
    /// - Pausing/unpausing the current program
    /// - Changing [State::pause_admin]
    /// - Changing [State::admin]
    admin: ActorId,
    /// Governance of this program. This address is in charge of
    /// pausing and unpausing the current program.
    pause_admin: ActorId,
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
    /// Is the `vft-manager` currently on pause.
    is_paused: bool,
    /// Address of the new vft-manager program which the current should upgrade to.
    /// It is required to handle cases when gas exhausted during execution of `upgrade` method.
    vft_manager_new: Option<ActorId>,
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
    /// Fee to pay `gear-eth-bridge` built-in actor.
    fee_bridge: u128,
    /// Incoming fee.
    fee_incoming: u128,
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
            supply_type,
        })
        .expect("Failed to emit event");
    }

    /// Remove the token pair from [State::token_map]. Can be called only by a [State::admin].
    pub fn remove_vara_to_eth_address(&mut self, vara_token_id: ActorId) {
        self.ensure_admin();

        let (eth_token_id, supply_type) = self.state_mut().token_map.remove(vara_token_id);

        self.notify_on(Event::TokenMappingRemoved {
            vara_token_id,
            eth_token_id,
            supply_type,
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

    /// Change [State::admin]. Can be called only by a [State::admin].
    pub fn set_admin(&mut self, new_admin: ActorId) {
        self.ensure_admin();

        self.state_mut().admin = new_admin;
    }

    /// Change [State::pause_admin]. Can be called only by a [State::admin].
    pub fn set_pause_admin(&mut self, new_pause_admin: ActorId) {
        self.ensure_admin();

        self.state_mut().pause_admin = new_pause_admin;
    }

    /// Ensure that message sender is a [State::admin].
    fn ensure_admin(&self) {
        if self.state().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
    }

    /// Pause the `vft-manager`.
    ///
    /// When `vft-manager` is paused it means that any requests to
    /// `submit_receipt`, `request_bridging` and `handle_request_bridging_interrupted_transfer`
    /// will be rejected.
    ///
    /// Can be called only by a [State::admin] or [State::pause_admin].
    pub fn pause(&mut self) {
        let sender = self.exec_context.actor_id();
        let state = &self.state();

        if sender != state.admin && sender != state.pause_admin {
            panic!("Access rejected");
        }

        if state.is_paused {
            panic!("Already paused");
        }

        self.state_mut().is_paused = true;

        self.notify_on(Event::Paused)
            .expect("Failed to deposit event");
    }

    /// Unpause the `vft-manager`.
    ///
    /// It will effectively cancel effect of the [VftManager::pause].
    ///
    /// Can be called only by a [State::admin] or [State::pause_admin].
    pub fn unpause(&mut self) {
        let sender = self.exec_context.actor_id();
        let state = &self.state();

        if sender != state.admin && sender != state.pause_admin {
            panic!("Access rejected");
        }

        if !state.is_paused {
            panic!("Already unpaused");
        }

        if state.vft_manager_new.is_some() {
            panic!("Upgrading")
        }

        self.state_mut().is_paused = false;

        self.notify_on(Event::Unpaused)
            .expect("Failed to deposit event");
    }

    fn ensure_running(&self) -> Result<(), Error> {
        if self.state().is_paused {
            Err(Error::Paused)
        } else {
            Ok(())
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
        self.ensure_running()?;

        submit_receipt::submit_receipt(self, slot, transaction_index, receipt_rlp).await
    }

    /// Request bridging of tokens from Gear to Ethereum.
    ///
    /// Allowance should be granted to the current program to spend `amount` tokens
    /// from the source address.
    pub async fn request_bridging(
        &mut self,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
    ) -> Result<(U256, H160), Error> {
        self.ensure_running()?;

        let value = msg::value();
        let fee = self.config().fee_incoming;
        if value != fee {
            panic!("Please attach exactly {fee} value");
        }

        let sender = self.exec_context.actor_id();

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
        self.ensure_running()?;

        request_bridging::handle_interrupted_transfer(self, msg_id).await
    }

    pub async fn upgrade(&mut self, vft_manager_new: ActorId) {
        self.ensure_admin();

        if !self.state().is_paused {
            panic!("Not paused");
        }

        if self
            .state()
            .vft_manager_new
            .map(|address| address != vft_manager_new)
            .unwrap_or(false)
        {
            panic!(
                "Upgrade called with vft_manager_new = {:?}",
                self.state().vft_manager_new
            );
        }

        self.state_mut().vft_manager_new = Some(vft_manager_new);

        let vft_manager = gstd::exec::program_id();
        let mut service = vft_client::Vft::new(GStdRemoting);
        let mappings = self.state().token_map.read_state();
        for (vft, _erc20, _supply) in mappings {
            let balance = service
                .balance_of(vft_manager)
                .recv(vft)
                .await
                .expect("Unable to get the balance of VftManager");

            if balance > 0.into()
                && !service
                    .transfer(vft_manager_new, balance)
                    .send_recv(vft)
                    .await
                    .expect("Unable to request a transfer to the new VftManager")
            {
                panic!("Unable to transfer tokens to the new VftManager ({vft:?})");
            }
        }

        gstd::exec::exit(vft_manager_new);
    }

    /// Get state of a `request_bridging` message tracker.
    pub fn request_briding_msg_tracker_state(
        &self,
        start: u32,
        count: u32,
    ) -> Vec<(MessageId, request_bridging::MsgTrackerMessageInfo)> {
        request_bridging::msg_tracker_ref()
            .message_info
            .iter()
            .skip(start as usize)
            .take(count as usize)
            .map(|(k, v)| (*k, v.clone()))
            .collect()
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

    /// Get current [State::pause_admin] address.
    pub fn pause_admin(&self) -> ActorId {
        self.state().pause_admin
    }

    /// Check if `vft-manager` is currently paused.
    pub fn is_paused(&self) -> bool {
        self.state().is_paused
    }

    /// Get current [Config].
    pub fn get_config(&self) -> Config {
        self.config().clone()
    }

    /// Get current [State::historical_proxy_address].
    pub fn historical_proxy_address(&self) -> ActorId {
        self.state().historical_proxy_address
    }

    pub fn transactions(&self, order: Order, start: u32, count: u32) -> Vec<(u64, u64)> {
        fn collect<'a, T: 'a + Copy>(
            start: u32,
            count: u32,
            iter: impl DoubleEndedIterator<Item = &'a T>,
        ) -> Vec<T> {
            iter.skip(start as usize)
                .take(count as usize)
                .copied()
                .collect()
        }

        match order {
            Order::Direct => collect(start, count, submit_receipt::transactions().iter()),
            Order::Reverse => collect(start, count, submit_receipt::transactions().iter().rev()),
        }
    }

    /// The method is intended for tests and is available only when the feature `mocks`
    /// is enabled. Populates the collection with processed transactions.
    ///
    /// Returns false when the collection is populated.
    pub fn fill_transactions(&mut self) -> bool {
        #[cfg(feature = "mocks")]
        {
            submit_receipt::fill_transactions()
        }

        #[cfg(not(feature = "mocks"))]
        panic!("Please rebuild with enabled `mocks` feature")
    }

    /// The method is intended for tests and is available only when the feature `mocks`
    /// is enabled. Inserts the message info into the corresponding collection.
    pub fn insert_message_info(
        &mut self,
        _msg_id: MessageId,
        _status: MessageStatus,
        _details: TxDetails,
    ) {
        #[cfg(feature = "mocks")]
        {
            request_bridging::msg_tracker_mut().insert_message_info(_msg_id, _status, _details);
        }

        #[cfg(not(feature = "mocks"))]
        panic!("Please rebuild with enabled `mocks` feature")
    }

    /// The method is intended for tests and is available only when the feature `mocks`
    /// is enabled. Sends a VFT-message to the sender to mint/unlock tokens depending
    /// on the `_supply_type`.
    ///
    /// Designed for benchmarking gas consumption by the VFT-response processing function.
    pub async fn calculate_gas_for_reply(
        &mut self,
        _slot: u64,
        _transaction_index: u64,
        _supply_type: TokenSupply,
    ) -> Result<(), Error> {
        #[cfg(feature = "mocks")]
        {
            use submit_receipt::token_operations;

            let source = self.exec_context.actor_id();
            match _supply_type {
                TokenSupply::Ethereum => {
                    token_operations::mint(
                        _slot,
                        _transaction_index,
                        source,
                        source,
                        100u32.into(),
                        self.config(),
                    )
                    .await
                }

                TokenSupply::Gear => {
                    token_operations::unlock(
                        _slot,
                        _transaction_index,
                        source,
                        source,
                        100u32.into(),
                        self.config(),
                    )
                    .await
                }
            }
        }

        #[cfg(not(feature = "mocks"))]
        panic!("Please rebuild with enabled `mocks` feature")
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
                admin: exec_context.actor_id(),
                pause_admin: exec_context.actor_id(),
                erc20_manager_address: config.erc20_manager_address,
                token_map: TokenMap::default(),
                historical_proxy_address: config.historical_proxy_address,
                is_paused: false,
                vft_manager_new: None,
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
        #[allow(clippy::deref_addrof)]
        unsafe { (*&raw const STATE).as_ref() }.expect("VftManager::seed() should be called")
    }

    /// Get a mutable reference to the global [State].
    fn state_mut(&mut self) -> &mut State {
        #[allow(clippy::deref_addrof)]
        unsafe { (*&raw mut STATE).as_mut() }.expect("VftManager::seed() should be called")
    }

    /// Get a reference to the global [Config].
    fn config(&self) -> &Config {
        #[allow(clippy::deref_addrof)]
        unsafe { (*&raw const CONFIG).as_ref() }.expect("VftManager::seed() should be called")
    }
}
