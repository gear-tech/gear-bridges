use gstd::MessageId;
use sails_rtl::{
    gstd::{gservice, msg, ExecContext},
    prelude::*,
};

use gstd::exec;
mod grc20_gateway;
mod utils;
pub struct BridgePayment<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Encode, Decode, TypeInfo)]
pub enum BridgePaymentEvents {
    TeleportVaraToEth {
        nonce: U256,
        sender: ActorId,
        amount: U256,
        receiver: H160,
        eth_token_id: H160,
    },
}

static mut DATA: Option<BridgePaymentData> = None;
static mut CONFIG: Option<Config> = None;

#[derive(Debug)]
pub struct BridgePaymentData {
    admin_address: ActorId,
    grc20_gateway_address: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    admin_address: ActorId,
    grc20_gateway_address: ActorId,
    config: Config,
}

impl InitConfig {
    pub fn new(admin_address: ActorId, grc20_gateway_address: ActorId, config: Config) -> Self {
        Self {
            admin_address,
            grc20_gateway_address,
            config,
        }
    }
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Config {
    fee: u128,
    gas_for_reply_deposit: u64,
    gas_to_send_request_to_gateway: u64,
}

impl Config {
    pub fn new(fee: u128, gas_for_reply_deposit: u64, gas_to_send_request_to_gateway: u64) -> Self {
        Self {
            fee,
            gas_for_reply_deposit,
            gas_to_send_request_to_gateway,
        }
    }
}
impl<T> BridgePayment<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            DATA = Some(BridgePaymentData {
                admin_address: exec_context.actor_id(),
                grc20_gateway_address: config.grc20_gateway_address,
            });
            CONFIG = Some(config.config);
        }
    }
    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn data(&self) -> &BridgePaymentData {
        unsafe {
            DATA.as_ref()
                .expect("BridgePaymentData::seed() should be called")
        }
    }

    fn data_mut(&mut self) -> &mut BridgePaymentData {
        unsafe {
            DATA.as_mut()
                .expect("BridgePaymentData::seed() should be called")
        }
    }

    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("BridgePaymentData::seed() should be called")
        }
    }

    fn config_mut(&self) -> &mut Config {
        unsafe {
            CONFIG
                .as_mut()
                .expect("BridgePaymentData::seed() should be called")
        }
    }

    pub fn handle_reply() {}
}

#[gservice(events = BridgePaymentEvents)]
impl<T> BridgePayment<T>
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
        msg::send_with_gas(data.admin_address.into(), "", 0, fee_balance)
            .expect("Failed to reclaim fees");
    }

    pub async fn request_to_gateway(
        &mut self,
        sender: ActorId,
        amount: U256,
        receiver: [u8; 20],
        eth_token_id: [u8; 20],
    ) {
        let data = self.data();
        let config = self.config();

        let attached_value = msg::value();

        if attached_value < config.fee {
            panic!("Not enough fee");
        }
        match utils::send_message_to_gateway(
            data.grc20_gateway_address,
            sender,
            amount,
            receiver,
            eth_token_id,
            config,
        )
        .await
        {
            Ok(Ok(nonce)) => {
                self.notify_on(BridgePaymentEvents::TeleportVaraToEth {
                    nonce,
                    sender,
                    amount,
                    receiver: receiver.into(),
                    eth_token_id: eth_token_id.into(),
                })
                .expect("Error in depositing event");

                // return remainder
                let refund = self.config().fee - attached_value;
                if refund >= exec::env_vars().existential_deposit {
                    handle_refund(self.exec_context.actor_id(), refund);
                }
            }

            _ => {
                // Handle refund for the full attached value on any error
                handle_refund(self.exec_context.actor_id(), attached_value);
            }
        }
    }
}

fn handle_refund(actor_id: ActorId, amount: u128) {
    msg::send_with_gas(actor_id.into(), "", 0, amount).expect("Error in refund");
}
