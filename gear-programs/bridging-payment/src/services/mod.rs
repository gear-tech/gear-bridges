use sails_rs::{
    gstd::{msg, ExecContext},
    prelude::*,
};

use gstd::exec;
mod error;
use error::Error;
mod msg_tracker;
use msg_tracker::{msg_tracker, msg_tracker_mut, MessageInfo, MessageStatus, TransactionDetails};
mod token_msg;
use msg_tracker::MessageTracker;
use token_msg::{transfer_tokens, transfer_tokens_back};
mod utils;
mod vft;
mod vft_gateway;
mod vft_gateway_msg;
use crate::event_or_panic_async;
use vft_gateway_msg::send_message_to_gateway;

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
static mut MSG_TRACKER: Option<MessageTracker> = None;

#[derive(Debug)]
pub struct BridgingPaymentData {
    admin_address: ActorId,
    vft_gateway_address: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    admin_address: ActorId,
    vft_gateway_address: ActorId,
    config: Config,
}

impl InitConfig {
    pub fn new(admin_address: ActorId, vft_gateway_address: ActorId, config: Config) -> Self {
        Self {
            admin_address,
            vft_gateway_address,
            config,
        }
    }
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    fee: u128,
    gas_for_reply_deposit: u64,
    gas_to_send_request_to_gateway: u64,
    gas_to_transfer_tokens: u64,
    reply_timeout: u32,
    gas_for_request_to_gateway_msg: u64,
}

impl Config {
    pub fn new(
        fee: u128,
        gas_for_reply_deposit: u64,
        gas_to_send_request_to_gateway: u64,
        gas_to_transfer_tokens: u64,
        reply_timeout: u32,
        gas_for_request_to_gateway_msg: u64,
    ) -> Self {
        Self {
            fee,
            gas_for_reply_deposit,
            gas_to_send_request_to_gateway,
            gas_to_transfer_tokens,
            reply_timeout,
            gas_for_request_to_gateway_msg,
        }
    }
}
impl<T> BridgingPayment<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            DATA = Some(BridgingPaymentData {
                admin_address: exec_context.actor_id(),
                vft_gateway_address: config.vft_gateway_address,
            });
            CONFIG = Some(config.config);
            MSG_TRACKER = Some(MessageTracker::default());
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
        msg::send_with_gas(data.admin_address, "", 0, fee_balance).expect("Failed to reclaim fees");
    }

    pub fn update_vft_gateway_address(&mut self, new_vft_gateway_address: ActorId) {
        let data = self.data();
        if data.admin_address != self.exec_context.actor_id() {
            panic!("Not admin");
        }
        self.data_mut().vft_gateway_address = new_vft_gateway_address;
    }

    pub fn update_config(
        &mut self,
        fee: Option<u128>,
        gas_for_reply_deposit: Option<u64>,
        gas_to_send_request_to_gateway: Option<u64>,
        gas_to_transfer_tokens: Option<u64>,
        reply_timeout: Option<u32>,
        gas_for_request_to_gateway_msg: Option<u64>,
    ) {
        if self.data().admin_address != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        if let Some(fee) = fee {
            self.config_mut().fee = fee;
        }

        if let Some(gas_for_reply_deposit) = gas_for_reply_deposit {
            self.config_mut().gas_for_reply_deposit = gas_for_reply_deposit;
        }

        if let Some(gas_to_send_request_to_gateway) = gas_to_send_request_to_gateway {
            self.config_mut().gas_to_send_request_to_gateway = gas_to_send_request_to_gateway;
        }

        if let Some(gas_to_transfer_tokens) = gas_to_transfer_tokens {
            self.config_mut().gas_to_transfer_tokens = gas_to_transfer_tokens;
        }

        if let Some(reply_timeout) = reply_timeout {
            self.config_mut().reply_timeout = reply_timeout;
        }

        if let Some(gas_for_request_to_gateway_msg) = gas_for_request_to_gateway_msg {
            self.config_mut().gas_for_request_to_gateway_msg = gas_for_request_to_gateway_msg;
        }
    }

    pub async fn request_to_gateway(
        &mut self,
        amount: U256,
        receiver: H160,
        vara_token_id: ActorId,
    ) {
        let vft_gateway_address = self.data().vft_gateway_address;
        let config = self.config();
        let sender = self.exec_context.actor_id();

        if gstd::exec::gas_available()
            < config.gas_to_transfer_tokens
                + config.gas_to_send_request_to_gateway
                + config.gas_for_request_to_gateway_msg
                + 3 * config.gas_for_reply_deposit
        {
            panic!("Please attach more gas");
        }

        event_or_panic_async!(self, || async move {
            let attached_value = msg::value();

            if attached_value < config.fee {
                panic!("Not enough fee");
            }

            let program_id = exec::program_id();

            // Transfer tokens to the contract's account
            transfer_tokens(vara_token_id, sender, program_id, amount, config).await?;

            handle_gateway_transaction(
                sender,
                vara_token_id,
                amount,
                receiver,
                attached_value,
                vft_gateway_address,
                config,
            )
            .await
        });
    }

    pub async fn continue_transaction(&mut self, msg_id: MessageId) {
        let vft_gateway_address = self.data().vft_gateway_address;
        let config = self.config();

        event_or_panic_async!(self, || async move {
            let msg_tracker = msg_tracker_mut();
            let msg_info = msg_tracker
                .get_message_info(&msg_id)
                .expect("Unexpected: msg status does not exist");

            match msg_info.status {
                MessageStatus::MessageToGatewayStep => {
                    if let TransactionDetails::SendMessageToGateway {
                        sender,
                        vara_token_id,
                        amount,
                        receiver,
                        attached_value,
                    } = msg_info.details
                    {
                        handle_gateway_transaction(
                            sender,
                            vara_token_id,
                            amount,
                            receiver,
                            attached_value,
                            vft_gateway_address,
                            config,
                        )
                        .await
                    } else {
                        panic!("Unexpected tx details")
                    }
                }
                MessageStatus::GatewayMessageProcessingCompleted((nonce, eth_token_id)) => {
                    if let TransactionDetails::SendMessageToGateway {
                        sender,
                        amount,
                        receiver,
                        attached_value,
                        ..
                    } = msg_info.details
                    {
                        process_refund(sender, attached_value, config);

                        Ok(BridgingPaymentEvents::TeleportVaraToEth {
                            nonce,
                            sender,
                            amount,
                            receiver,
                            eth_token_id,
                        })
                    } else {
                        panic!("Unexpected tx details");
                    }
                }
                _ => {
                    // Handle any other status or unexpected cases
                    panic!("Unexpected status or transaction completed.");
                }
            }
        });
    }

    pub async fn return_tokens(&mut self, msg_id: MessageId) {
        let msg_tracker = msg_tracker_mut();

        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg status does not exist");
        if msg_info.status != MessageStatus::ReturnTokensBackStep {
            panic!("Wrong status");
        }

        let config = self.config();
        if let TransactionDetails::SendMessageToGateway {
            sender,
            vara_token_id,
            amount,
            receiver: _,
            attached_value,
        } = msg_info.details
        {
            transfer_tokens_back(vara_token_id, exec::program_id(), sender, amount, config)
                .await
                .expect("Unable to send tokens back");
            process_refund(sender, attached_value, config);
        }
    }
    pub fn msg_tracker_state(&self) -> Vec<(MessageId, MessageInfo)> {
        msg_tracker().message_info.clone().into_iter().collect()
    }

    pub fn admin_address(&self) -> ActorId {
        self.data().admin_address
    }

    pub fn vft_gateway_address(&self) -> ActorId {
        self.data().vft_gateway_address
    }

    pub fn get_config(&self) -> Config {
        self.config().clone()
    }
}

async fn handle_gateway_transaction(
    sender: ActorId,
    vara_token_id: ActorId,
    amount: U256,
    receiver: H160,
    attached_value: u128,
    vft_gateway_address: ActorId,
    config: &Config,
) -> Result<BridgingPaymentEvents, Error> {
    let (nonce, eth_token_id) = send_message_to_gateway(
        vft_gateway_address,
        sender,
        vara_token_id,
        amount,
        receiver,
        attached_value,
        config,
    )
    .await?;

    process_refund(sender, attached_value, config);

    Ok(BridgingPaymentEvents::TeleportVaraToEth {
        nonce,
        sender,
        amount,
        receiver,
        eth_token_id,
    })
}

fn process_refund(sender: ActorId, attached_value: u128, config: &Config) {
    let refund = attached_value - config.fee;
    if refund >= exec::env_vars().existential_deposit {
        handle_refund(sender, refund);
    }
}

fn handle_refund(actor_id: ActorId, amount: u128) {
    msg::send_with_gas(actor_id, "", 0, amount).expect("Error in refund");
}
