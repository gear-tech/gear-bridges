use sails_rs::{
    gstd::{msg, ExecContext},
    prelude::*,
};

use gstd::exec;
mod error;
use error::Error;
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
        let data = self.data();
        if data.admin_address != self.exec_context.actor_id() {
            panic!("Not admin");
        }
        let config: &mut Config = self.config_mut();
        config.fee = fee;
    }

    pub fn reclaim_fee(&mut self) {
        let data = self.data();
        if data.admin_address != self.exec_context.actor_id() {
            panic!("Not admin");
        }

        let fee_balance = exec::value_available();
        msg::send(data.admin_address, "", fee_balance).expect("Failed to reclaim fees");
    }

    pub fn update_vft_manager_address(&mut self, new_vft_manager_address: ActorId) {
        let data = self.data();
        if data.admin_address != self.exec_context.actor_id() {
            panic!("Not admin");
        }
        self.data_mut().vft_manager_address = new_vft_manager_address;
    }

    pub fn set_config(&mut self, config: Config) {
        if self.data().admin_address != self.exec_context.actor_id() {
            panic!("Not admin")
        }

        *self.config_mut() = config;
    }

    pub async fn make_request(&mut self, amount: U256, receiver: H160, vara_token_id: ActorId) {
        let vft_manager_address = self.data().vft_manager_address;
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

        if attached_value < config.fee {
            panic!("Not enough fee");
        }

        // Return surplus of attached value
        refund_surplus(sender, attached_value, config.fee);

        let result = handle_vft_manager_transaction(
            sender,
            vara_token_id,
            amount,
            receiver,
            vft_manager_address,
            config,
        )
        .await;

        match result {
            Ok(value) => {
                if let Err(e) = self.notify_on(value) {
                    panic!("Error in depositing events: {:?}", e);
                }
            }
            Err(e) => {
                panic!("Message processing failed with error: {:?}", e);
            }
        }
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

async fn handle_vft_manager_transaction(
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    vft_manager_address: ActorId,
    config: &Config,
) -> Result<BridgingPaymentEvents, Error> {
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
        refund_fee(sender, config.fee);
    })?;

    Ok(BridgingPaymentEvents::TeleportVaraToEth {
        nonce,
        sender,
        amount,
        receiver,
        eth_token_id,
    })
}

fn refund_surplus(sender: ActorId, attached_value: u128, fee: u128) {
    let refund = attached_value - fee;
    if refund >= exec::env_vars().existential_deposit {
        send_refund(sender, refund);
    }
}

fn refund_fee(sender: ActorId, fee: u128) {
    send_refund(sender, fee);
}

fn send_refund(actor_id: ActorId, amount: u128) {
    msg::send_with_gas(actor_id, "", 0, amount).expect("Error in refund");
}
