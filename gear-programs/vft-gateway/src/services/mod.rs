use collections::HashMap;
use gstd::MessageId;
use sails_rtl::{
    gstd::{gservice, msg, ExecContext},
    prelude::*,
};

mod bridge_builtin_operations;
pub mod error;
mod message_tracker;
mod security_handlers;
mod vft_master;
use error::Error;
use message_tracker::{MessageStatus, MessageTracker, MsgData};
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
    receiver_contract_id: H160,
    vara_to_eth_token_id: HashMap<ActorId, H160>,
    bridge_payment_id: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    receiver_contract_id: H160,
    gear_bridge_builtin: ActorId,
    bridge_payment_id: ActorId,
    config: Config,
}

impl InitConfig {
    pub fn new(
        receiver_contract_id: H160,
        gear_bridge_builtin: ActorId,
        bridge_payment_id: ActorId,
        config: Config,
    ) -> Self {
        Self {
            receiver_contract_id,
            gear_bridge_builtin,
            bridge_payment_id,
            config,
        }
    }
}
#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Config {
    gas_to_burn_tokens: u64,
    gas_for_reply_deposit: u64,
    gas_to_mint_tokens: u64,
    gas_to_send_request_to_builtin: u64,
    reply_timeout: u32,
}

impl Config {
    pub fn new(
        gas_to_burn_tokens: u64,
        gas_for_reply_deposit: u64,
        gas_to_mint_tokens: u64,
        gas_to_send_request_to_builtin: u64,
        reply_timeout: u32,
    ) -> Self {
        Self {
            gas_to_burn_tokens,
            gas_for_reply_deposit,
            gas_to_mint_tokens,
            gas_to_send_request_to_builtin,
            reply_timeout,
        }
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
                receiver_contract_id: config.receiver_contract_id,
                admin: exec_context.actor_id(),
                bridge_payment_id: config.bridge_payment_id,
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

    fn config_mut(&self) -> &mut Config {
        unsafe {
            CONFIG
                .as_mut()
                .expect("VftGatewayData::seed() should be called")
        }
    }
}

#[gservice]
impl<T> VftGateway<T>
where
    T: ExecContext,
{
    pub fn update_receiver_contract_id(&mut self, new_receiver_contract_id: H160) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        self.data_mut().receiver_contract_id = new_receiver_contract_id;
    }

    pub fn map_vara_to_eth_address(&mut self, vara_token_id: ActorId, eth_token_id: H160) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        self.data_mut()
            .vara_to_eth_token_id
            .insert(vara_token_id, eth_token_id);
    }
    pub async fn transfer_vara_to_eth(
        &mut self,
        vara_token_id: ActorId,
        sender: ActorId,
        amount: U256,
        receiver: H160,
    ) -> Result<(U256, H160), Error> {
        let data = self.data();
        if data.bridge_payment_id != self.exec_context.actor_id() {
            panic!("Only bridge payment contract can send reuests to transfer tokens")
        }
        let eth_token_id = data
            .vara_to_eth_token_id
            .get(&vara_token_id)
            .expect("No corresponding Ethereum address for the specified Vara token address");
        let config = self.config();
        let msg_id = msg::id();
        let msg_tracker = msg_tracker_mut();

        gstd::critical::set_hook(move || {
            security_handlers::panic_handler(
                msg_tracker_mut(),
                MsgData::new(sender, amount, receiver, vara_token_id),
            );
        });

        token_operations::burn_tokens(vara_token_id, sender, amount, config, msg_id, msg_tracker)
            .await?;

        let nonce = match bridge_builtin_operations::send_message_to_bridge_builtin(
            data.gear_bridge_builtin,
            receiver.into(),
            *eth_token_id,
            amount,
            config,
            msg_id,
            msg_tracker,
        )
        .await
        {
            Ok(nonce) => nonce,
            Err(_) => {
                // In case of failure, mint tokens back to the sender
                token_operations::mint_tokens(vara_token_id, sender, amount, config).await?;
                // Return an error indicating the tokens were refunded
                msg_tracker.remove_message_status(&msg_id);
                return Err(Error::TokensRefundedError);
            }
        };

        msg_tracker.remove_message_status(&msg_id);

        Ok((nonce, *eth_token_id))
    }

    pub async fn handle_interrupted_transfer(
        &mut self,
        interrupted_msg_id: MessageId,
    ) -> Result<U256, Error> {
        let data = self.data();

        let config = self.config();
        let msg_id = msg::id();
        let msg_tracker = msg_tracker_mut();

        let (msg_status, msg_data) = msg_tracker
            .remove_pending_message(&interrupted_msg_id)
            .expect("Pending message doesn't exist");
        let (sender, amount, receiver, vara_token_id) = msg_data.data();
        let eth_token_id = data
            .vara_to_eth_token_id
            .get(&vara_token_id)
            .expect("No corresponding Ethereum address for the specified Vara token address");
        gstd::critical::set_hook(move || {
            security_handlers::panic_handler(
                msg_tracker_mut(),
                MsgData::new(sender, amount, receiver, vara_token_id),
            );
        });

        let nonce = match msg_status {
            MessageStatus::TokenBurnCompleted(true)
            | MessageStatus::SendingMessageToBridgeBuiltin(_) => {
                let nonce = match bridge_builtin_operations::send_message_to_bridge_builtin(
                    data.gear_bridge_builtin,
                    receiver.into(),
                    *eth_token_id,
                    amount,
                    config,
                    msg_id,
                    msg_tracker,
                )
                .await
                {
                    Ok(nonce) => nonce,
                    Err(_) => {
                        // In case of failure, mint tokens back to the sender
                        token_operations::mint_tokens(vara_token_id, sender, amount, config)
                            .await?;
                        // Return an error indicating the tokens were refunded
                        return Err(Error::TokensRefundedError);
                    }
                };

                nonce
            }
            MessageStatus::BridgeResponseReceived(true, nonce) => nonce,
            MessageStatus::BridgeResponseReceived(false, _)
            | MessageStatus::SendingMessageToMintTokens(_)
            | MessageStatus::TokenMintCompleted(false) => {
                // In case of failure, mint tokens back to the sender
                token_operations::mint_tokens(vara_token_id, sender, amount, config).await?;
                // Return an error indicating the tokens were refunded
                msg_tracker.remove_message_status(&msg_id);
                return Err(Error::TokensRefundedError);
            }
            MessageStatus::TokenMintCompleted(true) => {
                return Err(Error::TokensRefundedError);
            }
            _ => unreachable!(),
        };
        msg_tracker.remove_message_status(&msg_id);
        Ok(nonce)
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
