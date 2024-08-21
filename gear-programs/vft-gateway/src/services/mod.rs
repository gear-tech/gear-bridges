use collections::HashMap;
use sails_rs::{gstd::ExecContext, prelude::*};

mod bridge_builtin_operations;
pub mod error;
pub mod msg_tracker;
mod utils;
mod vft;
use error::Error;
use msg_tracker::{MessageInfo, MessageStatus, MessageTracker};
mod token_operations;
pub struct VftGateway<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Request {
    pub receiver: H160,
    pub token_id: H160,
    pub amount: U256,
}

static mut DATA: Option<VftGatewayData> = None;
static mut CONFIG: Option<Config> = None;
static mut MSG_TRACKER: Option<MessageTracker> = None;

#[derive(Debug, Default)]
pub struct VftGatewayData {
    gear_bridge_builtin: ActorId,
    admin: ActorId,
    receiver_contract_address: H160,
    vara_to_eth_token_id: HashMap<ActorId, H160>,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    pub receiver_contract_address: H160,
    pub gear_bridge_builtin: ActorId,
    pub config: Config,
}

impl InitConfig {
    pub fn new(
        receiver_contract_address: H160,
        gear_bridge_builtin: ActorId,
        config: Config,
    ) -> Self {
        Self {
            receiver_contract_address,
            gear_bridge_builtin,
            config,
        }
    }
}
#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    gas_to_burn_tokens: u64,
    gas_for_reply_deposit: u64,
    gas_to_mint_tokens: u64,
    gas_to_send_request_to_builtin: u64,
    reply_timeout: u32,
    gas_for_transfer_to_eth_msg: u64,
}

impl Config {
    pub fn new(
        gas_to_burn_tokens: u64,
        gas_for_reply_deposit: u64,
        gas_to_mint_tokens: u64,
        gas_to_send_request_to_builtin: u64,
        reply_timeout: u32,
        gas_for_transfer_to_eth_msg: u64,
    ) -> Self {
        Self {
            gas_to_burn_tokens,
            gas_for_reply_deposit,
            gas_to_mint_tokens,
            gas_to_send_request_to_builtin,
            reply_timeout,
            gas_for_transfer_to_eth_msg,
        }
    }
}

#[service]
impl<T> VftGateway<T>
where
    T: ExecContext,
{
    pub fn update_receiver_contract_address(&mut self, new_receiver_contract_address: H160) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        self.data_mut().receiver_contract_address = new_receiver_contract_address;
    }

    pub fn map_vara_to_eth_address(&mut self, vara_token_id: ActorId, eth_token_id: H160) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        self.data_mut()
            .vara_to_eth_token_id
            .insert(vara_token_id, eth_token_id);
    }

    pub fn remove_vara_to_eth_address(&mut self, vara_token_id: ActorId) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        self.data_mut().vara_to_eth_token_id.remove(&vara_token_id);
    }

    pub fn update_config(
        &mut self,
        gas_to_burn_tokens: Option<u64>,
        gas_to_mint_tokens: Option<u64>,
        gas_for_reply_deposit: Option<u64>,
        gas_to_send_request_to_builtin: Option<u64>,
        reply_timeout: Option<u32>,
        gas_for_transfer_to_eth_msg: Option<u64>,
    ) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        if let Some(gas_to_burn_tokens) = gas_to_burn_tokens {
            self.config_mut().gas_to_burn_tokens = gas_to_burn_tokens;
        }

        if let Some(gas_to_mint_tokens) = gas_to_mint_tokens {
            self.config_mut().gas_to_mint_tokens = gas_to_mint_tokens;
        }

        if let Some(gas_to_send_request_to_builtin) = gas_to_send_request_to_builtin {
            self.config_mut().gas_to_send_request_to_builtin = gas_to_send_request_to_builtin;
        }

        if let Some(reply_timeout) = reply_timeout {
            self.config_mut().reply_timeout = reply_timeout;
        }

        if let Some(gas_for_reply_deposit) = gas_for_reply_deposit {
            self.config_mut().gas_for_reply_deposit = gas_for_reply_deposit;
        }

        if let Some(gas_for_transfer_to_eth_msg) = gas_for_transfer_to_eth_msg {
            self.config_mut().gas_for_transfer_to_eth_msg = gas_for_transfer_to_eth_msg;
        }
    }

    pub async fn transfer_vara_to_eth(
        &mut self,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
    ) -> Result<(U256, H160), Error> {
        let data = self.data();
        let sender = self.exec_context.actor_id();
        let msg_id = gstd::msg::id();

        let eth_token_id = self.get_eth_token_id(&vara_token_id)?;
        let config = self.config();

        if gstd::exec::gas_available()
            < config.gas_to_burn_tokens
                + config.gas_to_send_request_to_builtin
                + config.gas_for_transfer_to_eth_msg
                + 3 * config.gas_for_reply_deposit
        {
            panic!("Please attach more gas");
        }
        token_operations::burn_tokens(vara_token_id, sender, receiver, amount, config, msg_id)
            .await?;
        let nonce = match bridge_builtin_operations::send_message_to_bridge_builtin(
            data.gear_bridge_builtin,
            receiver.into(),
            eth_token_id,
            amount,
            config,
            msg_id,
        )
        .await
        {
            Ok(nonce) => nonce,
            Err(_) => {
                // In case of failure, mint tokens back to the sender
                token_operations::mint_tokens(vara_token_id, sender, amount, config, msg_id)
                    .await?;
                return Err(Error::TokensRefundedError);
            }
        };
        Ok((nonce, eth_token_id))
    }

    pub async fn handle_interrupted_transfer(
        &mut self,
        msg_id: MessageId,
    ) -> Result<(U256, H160), Error> {
        let data = self.data();

        let config = self.config();
        let msg_tracker = msg_tracker_mut();

        let msg_info = msg_tracker
            .get_message_info(&msg_id)
            .expect("Unexpected: msg status does not exist");

        let (sender, amount, receiver, vara_token_id) = msg_info.details.data();
        let eth_token_id = data
            .vara_to_eth_token_id
            .get(&vara_token_id)
            .expect("No corresponding Ethereum address for the specified Vara token address");

        match msg_info.status {
            MessageStatus::TokenBurnCompleted(true) | MessageStatus::BridgeBuiltinStep => {
                match bridge_builtin_operations::send_message_to_bridge_builtin(
                    data.gear_bridge_builtin,
                    receiver.into(),
                    *eth_token_id,
                    amount,
                    config,
                    msg_id,
                )
                .await
                {
                    Ok(nonce) => Ok((nonce, *eth_token_id)),
                    Err(_) => {
                        // In case of failure, mint tokens back to the sender
                        token_operations::mint_tokens(
                            vara_token_id,
                            sender,
                            amount,
                            config,
                            msg_id,
                        )
                        .await?;
                        Err(Error::TokensRefundedError)
                    }
                }
            }
            MessageStatus::BridgeResponseReceived(Some(nonce)) => {
                msg_tracker_mut().remove_message_info(&msg_id);
                Ok((nonce, *eth_token_id))
            }
            MessageStatus::MintTokensStep => {
                token_operations::mint_tokens(vara_token_id, sender, amount, config, msg_id)
                    .await?;
                Err(Error::TokensRefundedError)
            }
            _ => {
                panic!("Unexpected status or transaction completed.")
            }
        }
    }

    pub fn msg_tracker_state(&self) -> Vec<(MessageId, MessageInfo)> {
        msg_tracker().message_info.clone().into_iter().collect()
    }

    pub fn vara_to_eth_addresses(&self) -> Vec<(ActorId, H160)> {
        self.data()
            .vara_to_eth_token_id
            .clone()
            .into_iter()
            .collect()
    }

    pub fn receiver_contract_address(&self) -> H160 {
        self.data().receiver_contract_address
    }

    pub fn gear_bridge_builtin(&self) -> ActorId {
        self.data().gear_bridge_builtin
    }

    pub fn admin(&self) -> ActorId {
        self.data().admin
    }

    pub fn get_config(&self) -> Config {
        self.config().clone()
    }
}

impl<T> VftGateway<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            DATA = Some(VftGatewayData {
                gear_bridge_builtin: config.gear_bridge_builtin,
                receiver_contract_address: config.receiver_contract_address,
                admin: exec_context.actor_id(),
                ..Default::default()
            });
            CONFIG = Some(config.config);
            MSG_TRACKER = Some(MessageTracker::default());
        }
    }
    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn data(&self) -> &VftGatewayData {
        unsafe { DATA.as_ref().expect("VftGateway::seed() should be called") }
    }

    fn data_mut(&mut self) -> &mut VftGatewayData {
        unsafe { DATA.as_mut().expect("VftGateway::seed() should be called") }
    }

    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("VftGateway::seed() should be called")
        }
    }

    fn config_mut(&mut self) -> &mut Config {
        unsafe {
            CONFIG
                .as_mut()
                .expect("VftGateway::seed() should be called")
        }
    }

    fn get_eth_token_id(&self, vara_token_id: &ActorId) -> Result<H160, Error> {
        self.data()
            .vara_to_eth_token_id
            .get(vara_token_id)
            .cloned()
            .ok_or(Error::NoCorrespondingEthAddress)
    }
}
fn msg_tracker() -> &'static MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_ref()
            .expect("VftGateway::seed() should be called")
    }
}

fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_mut()
            .expect("VftGateway::seed() should be called")
    }
}
