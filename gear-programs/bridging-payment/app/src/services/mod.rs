use sails_rs::{
    gstd::{msg, ExecContext},
    prelude::*,
};

use gstd::exec;
mod error;
mod vft_manager_msg;

pub struct BridgingPayment<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Encode, Decode, TypeInfo)]
pub enum BridgingPaymentEvents {
    TeleportVaraToEth {
        nonce: U256,
        sender: ActorId,
        amount: U256,
        receiver: H160,
        eth_token_id: H160,
    },
}

static mut DATA: Option<BridgingPaymentData> = None;
static mut CONFIG: Option<Config> = None;

#[derive(Debug)]
pub struct BridgingPaymentData {
    admin_address: ActorId,
    vft_manager_address: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    admin_address: ActorId,
    vft_manager_address: ActorId,
    config: Config,
}

impl InitConfig {
    pub fn new(admin_address: ActorId, vft_manager_address: ActorId, config: Config) -> Self {
        Self {
            admin_address,
            vft_manager_address,
            config,
        }
    }
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    fee: u128,
    gas_for_reply_deposit: u64,
    gas_to_send_request_to_vft_manager: u64,
    reply_timeout: u32,
    gas_for_request_to_vft_manager_msg: u64,
}

impl Config {
    pub fn new(
        fee: u128,
        gas_for_reply_deposit: u64,
        gas_to_send_request_to_vft_manager: u64,
        reply_timeout: u32,
        gas_for_request_to_vft_manager_msg: u64,
    ) -> Self {
        Self {
            fee,
            gas_for_reply_deposit,
            gas_to_send_request_to_vft_manager,
            reply_timeout,
            gas_for_request_to_vft_manager_msg,
        }
    }
}
impl<T> BridgingPayment<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig) {
        unsafe {
            DATA = Some(BridgingPaymentData {
                admin_address: config.admin_address,
                vft_manager_address: config.vft_manager_address,
            });
            CONFIG = Some(config.config);
        }
    }

    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn data(&self) -> &BridgingPaymentData {
        unsafe {
            DATA.as_ref()
                .expect("BridgingPaymentData::seed() should be called")
        }
    }

    fn data_mut(&mut self) -> &mut BridgingPaymentData {
        unsafe {
            DATA.as_mut()
                .expect("BridgingPaymentData::seed() should be called")
        }
    }

    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("BridgingPaymentData::seed() should be called")
        }
    }

    fn config_mut(&mut self) -> &mut Config {
        unsafe {
            CONFIG
                .as_mut()
                .expect("BridgingPaymentData::seed() should be called")
        }
    }
}

#[service(events = BridgingPaymentEvents)]
impl<T> BridgingPayment<T>
where
    T: ExecContext,
{
    pub fn set_fee(&mut self, fee: u128) {
        self.ensure_admin();

        let config: &mut Config = self.config_mut();
        config.fee = fee;
    }

    pub fn reclaim_fee(&mut self) {
        self.ensure_admin();

        let fee_balance = exec::value_available();
        msg::send(self.data().admin_address, "", fee_balance).expect("Failed to reclaim fees");
    }

    pub fn update_vft_manager_address(&mut self, new_vft_manager_address: ActorId) {
        self.ensure_admin();

        self.data_mut().vft_manager_address = new_vft_manager_address;
    }

    pub fn set_config(&mut self, config: Config) {
        self.ensure_admin();

        *self.config_mut() = config;
    }

    fn ensure_admin(&self) {
        if self.data().admin_address != self.exec_context.actor_id() {
            panic!("Not an admin")
        }
    }

    pub async fn make_request(&mut self, amount: U256, receiver: H160, vara_token_id: ActorId) {
        let vft_manager_address = self.data().vft_manager_address;
        let config = self.config();
        let sender = self.exec_context.actor_id();

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

    pub fn admin_address(&self) -> ActorId {
        self.data().admin_address
    }

    pub fn vft_manager_address(&self) -> ActorId {
        self.data().vft_manager_address
    }

    pub fn get_config(&self) -> Config {
        self.config().clone()
    }
}
