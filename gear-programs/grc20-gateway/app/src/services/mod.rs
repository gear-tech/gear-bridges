use gstd::MessageId;
use sails_rtl::{
    gstd::{gservice, ExecContext},
    prelude::*,
};

mod erc20;
pub mod utils;
use utils::Error;
pub struct GRC20Gateway<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Encode, Decode, TypeInfo)]
pub enum GrcProviderEvents {}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Request {
    pub receiver: H160,
    pub token_id: H160,
    pub amount: U256,
}

static mut DATA: Option<GRC20GatewayData> = None;
static mut CONFIG: Option<Config> = None;

#[derive(Debug)]
pub struct GRC20GatewayData {
    gear_bridge_builtin: ActorId,
    admin: ActorId,
    token_id: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    token_id: ActorId,
    gear_bridge_builtin: ActorId,
    config: Config,
}

impl InitConfig {
    pub fn new(token_id: ActorId, gear_bridge_builtin: ActorId, config: Config) -> Self {
        Self {
            token_id,
            gear_bridge_builtin,
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

impl<T> GRC20Gateway<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            DATA = Some(GRC20GatewayData {
                gear_bridge_builtin: config.gear_bridge_builtin,
                token_id: config.token_id,
                admin: exec_context.actor_id(),
            });
            CONFIG = Some(config.config);
        }
    }
    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn data(&self) -> &GRC20GatewayData {
        unsafe {
            DATA.as_ref()
                .expect("GRC20Gateway::seed() should be called")
        }
    }

    fn data_mut(&mut self) -> &mut GRC20GatewayData {
        unsafe {
            DATA.as_mut()
                .expect("GRC20Gateway::seed() should be called")
        }
    }

    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("GRC20Gateway::seed() should be called")
        }
    }

    fn config_mut(&self) -> &Config {
        unsafe {
            CONFIG
                .as_mut()
                .expect("GRC20Gateway::seed() should be called")
        }
    }

    pub fn handle_reply() {}
}

#[gservice(events = GrcProviderEvents)]
impl<T> GRC20Gateway<T>
where
    T: ExecContext,
{
    pub async fn teleport_vara_to_eth(
        &mut self,
        sender: ActorId,
        amount: U256,
        receiver: [u8; 20],
        eth_token_id: [u8; 20],
    ) -> Result<U256, Error> {
        let data = self.data();
        let config = self.config();

        utils::burn_tokens(data.token_id, sender, amount, config).await?;

        let nonce = match utils::send_message_to_bridge_builtin(
            data.gear_bridge_builtin,
            receiver.into(),
            eth_token_id.into(),
            amount,
            config,
        )
        .await
        {
            Ok(nonce) => nonce,
            Err(_) => {
                // In case of failure, mint tokens back to the sender
                utils::mint_tokens(data.token_id, sender, amount, config).await?;
                // Return an error indicating the tokens were refunded
                return Err(Error::TokensRefundedError);
            }
        };

        Ok(nonce)
    }
}
