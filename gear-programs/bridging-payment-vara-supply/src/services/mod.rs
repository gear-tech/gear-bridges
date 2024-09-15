use sails_rs::{
    gstd::{msg, ExecContext},
    prelude::*,
};

use gstd::exec;
mod error;
use error::Error;
mod msg_tracker;
use msg_tracker::MessageTracker;
use msg_tracker::{msg_tracker, msg_tracker_mut, MessageInfo, MessageStatus, TransactionDetails};
mod utils;
mod vft_treasury;
mod vft_treasury_msg;

use utils::maybe_event_or_panic_async;
use vft_treasury_msg::send_message_to_treasury;

pub struct BridgingPayment<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Encode, Decode, TypeInfo)]
pub enum BridgingPaymentEvents {
    DepositVaraToTreasury {
        sender: ActorId,
        amount: U256,
        receiver: H160,
    },
}

static mut DATA: Option<BridgingPaymentData> = None;
static mut CONFIG: Option<Config> = None;
static mut MSG_TRACKER: Option<MessageTracker> = None;

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
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            DATA = Some(BridgingPaymentData {
                admin_address: exec_context.actor_id(),
                vft_treasury_address: config.vft_treasury_address,
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

    pub fn update_vft_treasury_address(&mut self, new_vft_treasury_address: ActorId) {
        let data = self.data();
        if data.admin_address != self.exec_context.actor_id() {
            panic!("Not admin");
        }
        self.data_mut().vft_treasury_address = new_vft_treasury_address;
    }

    pub fn update_config(
        &mut self,
        fee: Option<u128>,
        gas_for_reply_deposit: Option<u64>,
        gas_to_send_request_to_treasury: Option<u64>,
        reply_timeout: Option<u32>,
        gas_for_request_to_treasury_msg: Option<u64>,
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

    pub async fn request_transaction(
        &mut self,
        amount: U256,
        receiver: H160,
        vara_token_id: ActorId,
    ) {
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

        maybe_event_or_panic_async!(self, || async move {
            let attached_value = msg::value();

            if attached_value < config.fee {
                panic!("Not enough fee");
            }

            handle_treasury_transaction(
                sender,
                vara_token_id,
                amount,
                receiver,
                attached_value,
                vft_treasury_address,
                config,
            )
            .await
            .map(Some)
        });
    }

    pub async fn continue_transaction(&mut self, msg_id: MessageId) {
        let vft_treasury_address = self.data().vft_treasury_address;
        let config = self.config();

        maybe_event_or_panic_async!(self, || async move {
            let msg_tracker = msg_tracker_mut();
            let msg_info = msg_tracker
                .get_message_info(&msg_id)
                .expect("Unexpected: msg status does not exist");

            match msg_info.status {
                MessageStatus::SendingMessageToTreasury => {
                    let TransactionDetails {
                        sender,
                        vara_token_id,
                        amount,
                        receiver,
                        attached_value,
                    } = msg_info.details;

                    handle_treasury_transaction(
                        sender,
                        vara_token_id,
                        amount,
                        receiver,
                        attached_value,
                        vft_treasury_address,
                        config,
                    )
                    .await
                    .map(Some)
                }
                MessageStatus::TreasuryMessageProcessingCompleted => {
                    let TransactionDetails {
                        sender,
                        amount,
                        receiver,
                        attached_value,
                        ..
                    } = msg_info.details;

                    process_refund(sender, attached_value, config);

                    Ok(Some(BridgingPaymentEvents::DepositVaraToTreasury {
                        sender,
                        amount,
                        receiver,
                    }))
                }
                MessageStatus::ProcessRefund => {
                    let TransactionDetails {
                        sender,
                        attached_value,
                        ..
                    } = msg_info.details;

                    handle_refund(sender, attached_value);

                    Ok(None)
                }
                _ => {
                    // Handle any other status or unexpected cases
                    panic!("Unexpected status or transaction completed.");
                }
            }
        });
    }

    pub fn msg_tracker_state(&self) -> Vec<(MessageId, MessageInfo)> {
        msg_tracker().message_info.clone().into_iter().collect()
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
    attached_value: u128,
    vft_gateway_address: ActorId,
    config: &Config,
) -> Result<BridgingPaymentEvents, Error> {
    send_message_to_treasury(
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

    Ok(BridgingPaymentEvents::DepositVaraToTreasury {
        sender,
        amount,
        receiver,
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
