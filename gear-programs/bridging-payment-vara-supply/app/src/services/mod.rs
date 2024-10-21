use sails_rs::{
    gstd::{msg, ExecContext},
    prelude::*,
};

use gstd::exec;
mod error;
use error::Error;
mod utils;
mod vft_treasury_msg;

use vft_treasury_msg::send_message_to_treasury;

pub struct BridgingPayment<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Encode, Decode, TypeInfo)]
pub enum BridgingPaymentEvents {
    DepositToTreasury {
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
    vft_treasury_address: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    admin_address: ActorId,
    vft_treasury_address: ActorId,
    config: Config,
}

impl InitConfig {
    pub fn new(admin_address: ActorId, vft_treasury_address: ActorId, config: Config) -> Self {
        Self {
            admin_address,
            vft_treasury_address,
            config,
        }
    }
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    fee: u128,
    gas_for_reply_deposit: u64,
    gas_to_send_request_to_treasury: u64,
    reply_timeout: u32,
    gas_for_request_to_treasury_msg: u64,
}

impl Config {
    pub fn new(
        fee: u128,
        gas_for_reply_deposit: u64,
        gas_to_send_request_to_treasury: u64,
        reply_timeout: u32,
        gas_for_request_to_treasury_msg: u64,
    ) -> Self {
        Self {
            fee,
            gas_for_reply_deposit,
            gas_to_send_request_to_treasury,
            reply_timeout,
            gas_for_request_to_treasury_msg,
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
                vft_treasury_address: config.vft_treasury_address,
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

    fn enusure_admin(&self) {
        if self.data().admin_address != self.exec_context.actor_id() {
            panic!("Not admin");
        }
    }
}

#[service(events = BridgingPaymentEvents)]
impl<T> BridgingPayment<T>
where
    T: ExecContext,
{
    pub fn set_fee(&mut self, fee: u128) {
        self.enusure_admin();

        let config: &mut Config = self.config_mut();
        config.fee = fee;
    }

    pub fn reclaim_fee(&mut self) {
        self.enusure_admin();

        let fee_balance = exec::value_available();
        msg::send(self.data().admin_address, "", fee_balance)
            .expect("Failed to reclaim fees");
    }

    pub fn update_config(
        &mut self,
        fee: Option<u128>,
        gas_for_reply_deposit: Option<u64>,
        gas_to_send_request_to_treasury: Option<u64>,
        reply_timeout: Option<u32>,
        gas_for_request_to_treasury_msg: Option<u64>,
    ) {
        self.enusure_admin();

        if let Some(fee) = fee {
            self.config_mut().fee = fee;
        }

        if let Some(gas_for_reply_deposit) = gas_for_reply_deposit {
            self.config_mut().gas_for_reply_deposit = gas_for_reply_deposit;
        }

        if let Some(gas_to_send_request_to_treasury) = gas_to_send_request_to_treasury {
            self.config_mut().gas_to_send_request_to_treasury = gas_to_send_request_to_treasury;
        }

        if let Some(reply_timeout) = reply_timeout {
            self.config_mut().reply_timeout = reply_timeout;
        }

        if let Some(gas_for_request_to_treasury_msg) = gas_for_request_to_treasury_msg {
            self.config_mut().gas_for_request_to_treasury_msg = gas_for_request_to_treasury_msg;
        }
    }

    pub async fn request(&mut self, amount: U256, receiver: H160, vara_token_id: ActorId) {
        let vft_treasury_address = self.data().vft_treasury_address;
        let config = self.config();
        let sender = self.exec_context.actor_id();

        if gstd::exec::gas_available()
            < config.gas_to_send_request_to_treasury
                + config.gas_for_request_to_treasury_msg
                + 2 * config.gas_for_reply_deposit
        {
            panic!("Please attach more gas");
        }

        let attached_value = msg::value();

        if attached_value < config.fee {
            panic!("Not enough fee");
        }

        // Return surplus of attached value
        refund_surplus(sender, attached_value, config.fee);

        let result = handle_treasury_transaction(
            sender,
            vara_token_id,
            amount,
            receiver,
            vft_treasury_address,
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

    pub fn vft_treasury_address(&self) -> ActorId {
        self.data().vft_treasury_address
    }

    pub fn get_config(&self) -> Config {
        self.config().clone()
    }
}

async fn handle_treasury_transaction(
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    vft_treasury_address: ActorId,
    config: &Config,
) -> Result<BridgingPaymentEvents, Error> {
    let (nonce, eth_token_id) = send_message_to_treasury(
        vft_treasury_address,
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

    Ok(BridgingPaymentEvents::DepositToTreasury {
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
