//! Bridging Payment service implementation.

use sails_rs::{
    gstd::{msg, ExecContext},
    prelude::*,
};

use gstd::exec;
mod error;
mod vft_manager_msg;

/// Bridging Payment service.
pub struct BridgingPayment<ExecContext> {
    exec_context: ExecContext,
}

/// Events emitted by Bridging Payment service.
#[derive(Encode, Decode, TypeInfo)]
pub enum BridgingPaymentEvents {
    /// Bridging of tokens from Gear to Ethereum is requested successfully.
    TeleportVaraToEth {
        /// Nonce of the bridging message. Returned by the `pallet-gear-eth-bridge` built-in actor.
        nonce: U256,
        /// Original owner of the `VFT` tokens on Gear side.
        sender: ActorId,
        /// Amount of tokens.
        amount: U256,
        /// Receiver of the `ERC20` tokens on Ethereum side.
        receiver: H160,
        /// Address of the `ERC20` token that will be received by `receiver` on Ethereum.
        eth_token_id: H160,
    },
}

static mut STATE: Option<State> = None;
static mut CONFIG: Option<Config> = None;

/// Global state of the Bridging Payment service.
#[derive(Debug)]
pub struct State {
    /// Admin of this service. Admin is in charge of:
    /// - Changing fee
    /// - Withdrawing collected fees from the program address
    /// - Updating `vft-manager` address
    /// - Updating [Config] of this service
    admin_address: ActorId,
    /// Address of the `vft-manager` where bridging request will be sent.
    vft_manager_address: ActorId,
}

/// Config usd for Bridging Payment service initialization.
#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    /// Admin of this service. For detailed description see [State::admin_address].
    pub admin_address: ActorId,
    /// Address of the `vft-manager` where bridging request will be sent.
    pub vft_manager_address: ActorId,
    /// [Config] that will be initially set for this service.
    pub config: Config,
}

/// Config of the Bridging Payment service.
#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    /// Fee that will be taken from all the incoming bridging requests.
    pub fee: u128,
    /// Gas that will be reserved for reply processing.
    pub gas_for_reply_deposit: u64,
    /// Gas that will be attached to a message to the `vft-manager`.
    pub gas_to_send_request_to_vft_manager: u64,
    /// Reply timeout that will be applied to a message to the `vft-manager`.
    pub reply_timeout: u32,
    /// Gas that's required to process `make_request` logic.
    pub gas_for_request_to_vft_manager_msg: u64,
}

impl<T> BridgingPayment<T>
where
    T: ExecContext,
{
    /// Initialize state of the Bridging Payment service.
    pub fn seed(config: InitConfig) {
        unsafe {
            STATE = Some(State {
                admin_address: config.admin_address,
                vft_manager_address: config.vft_manager_address,
            });
            CONFIG = Some(config.config);
        }
    }

    /// Create Bridging Payment service.
    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn state(&self) -> &State {
        #[allow(clippy::deref_addrof)]
        unsafe { (*&raw const STATE).as_ref() }.expect("BridgingPayment::seed() should be called")
    }

    fn state_mut(&mut self) -> &mut State {
        #[allow(clippy::deref_addrof)]
        unsafe { (*&raw mut STATE).as_mut() }.expect("BridgingPayment::seed() should be called")
    }

    fn config(&self) -> &Config {
        #[allow(clippy::deref_addrof)]
        unsafe { (*&raw const CONFIG).as_ref() }.expect("BridgingPayment::seed() should be called")
    }

    fn config_mut(&mut self) -> &mut Config {
        #[allow(clippy::deref_addrof)]
        unsafe { (*&raw mut CONFIG).as_mut() }.expect("BridgingPayment::seed() should be called")
    }
}

#[service(events = BridgingPaymentEvents)]
impl<T> BridgingPayment<T>
where
    T: ExecContext,
{
    /// Set fee that this program will take from incoming requests.
    ///
    /// This method can be called only by admin.
    pub fn set_fee(&mut self, fee: u128) {
        self.ensure_admin();

        let config: &mut Config = self.config_mut();
        config.fee = fee;
    }

    /// Withdraw fees that were collected from user requests.
    ///
    /// This method can be called only by admin.
    pub fn reclaim_fee(&mut self) {
        self.ensure_admin();

        let fee_balance = exec::value_available();
        msg::send(self.state().admin_address, "", fee_balance).expect("Failed to reclaim fees");
    }

    /// Set new `vft-manager` address.
    ///
    /// This method can be called only by admin.
    pub fn update_vft_manager_address(&mut self, new_vft_manager_address: ActorId) {
        self.ensure_admin();

        self.state_mut().vft_manager_address = new_vft_manager_address;
    }

    /// Set new config.
    ///
    /// This method can be called only by admin.
    pub fn set_config(&mut self, config: Config) {
        self.ensure_admin();

        *self.config_mut() = config;
    }

    fn ensure_admin(&self) {
        if self.state().admin_address != self.exec_context.actor_id() {
            panic!("Not an admin")
        }
    }

    /// Send request to the underlying `vft-manager` program.
    ///
    /// This method will take additional fee from the message sender, so **exactly** [Config::fee] must
    /// be attached as a value when sending message to this method.
    ///
    /// Current fee amount can be retreived by calling `get_config`.
    pub async fn make_request(&mut self, amount: U256, receiver: H160, vara_token_id: ActorId) {
        let vft_manager_address = self.state().vft_manager_address;
        let config = self.config();
        let sender = self.exec_context.actor_id();

        if gstd::exec::gas_available()
            < config.gas_to_send_request_to_vft_manager
                + config.gas_for_request_to_vft_manager_msg
                + config.gas_for_reply_deposit
        {
            panic!("Please attach more gas");
        }

        let attached_value = msg::value();
        if attached_value != config.fee {
            panic!("Please attach exactly {} value", config.fee);
        }

        let (nonce, eth_token_id) = vft_manager_msg::send(
            vft_manager_address,
            sender,
            vara_token_id,
            amount,
            receiver,
            config,
        )
        .await
        .inspect_err(|_| {
            msg::send_with_gas(sender, "", 0, config.fee).expect("Error in refund");
        })
        .expect("Failed to send message to vft-manager");

        self.notify_on(BridgingPaymentEvents::TeleportVaraToEth {
            nonce,
            sender,
            amount,
            receiver,
            eth_token_id,
        })
        .expect("Error depositing event");
    }

    /// Get admin address.
    pub fn admin_address(&self) -> ActorId {
        self.state().admin_address
    }

    /// Get `vft-manager` address.
    pub fn vft_manager_address(&self) -> ActorId {
        self.state().vft_manager_address
    }

    /// Get currently applied [Config].
    pub fn get_config(&self) -> Config {
        self.config().clone()
    }
}
