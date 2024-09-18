use collections::HashMap;
pub(super) use error::Error;
use msg_tracker::{MessageInfo, MessageStatus, MessageTracker, TxDetails};
use sails_rs::{gstd::debug, gstd::ExecContext, prelude::*};
mod bridge_builtin_operations;
pub mod error;
mod msg_tracker;
mod token_operations;
mod utils;
mod vft;

pub struct VftTreasury<ExecContext> {
    exec_context: ExecContext,
}

static mut DATA: Option<VftTreasuryData> = None;
static mut CONFIG: Option<Config> = None;
static mut MSG_TRACKER: Option<MessageTracker> = None;

#[derive(Debug, Default)]
struct VftTreasuryData {
    admin: ActorId,
    receiver_contract_address: H160,
    gear_bridge_builtin: ActorId,
    ethereum_event_client: ActorId,
    bridging_payment_service: ActorId,
    eth_to_vara_token_id: HashMap<H160, ActorId>,
    vara_to_eth_token_id: HashMap<ActorId, H160>,
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    pub gas_for_transfer_tokens: u64,
    pub gas_for_reply_deposit: u64,
    pub gas_to_send_request_to_builtin: u64,
    pub reply_timeout: u32,
    pub gas_for_transfer_to_eth_msg: u64,
}

impl Config {
    pub fn new(
        gas_for_transfer_tokens: u64,
        gas_for_reply_deposit: u64,
        gas_to_send_request_to_builtin: u64,
        reply_timeout: u32,
        gas_for_transfer_to_eth_msg: u64,
    ) -> Self {
        Self {
            gas_for_transfer_tokens,
            gas_for_reply_deposit,
            gas_to_send_request_to_builtin,
            reply_timeout,
            gas_for_transfer_to_eth_msg,
        }
    }
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    pub receiver_contract_address: H160,
    pub gear_bridge_builtin: ActorId,
    pub ethereum_event_client: ActorId,
    pub bridging_payment_service: ActorId,
    pub config: Config,
}

impl InitConfig {
    pub fn new(
        receiver_contract_address: H160,
        gear_bridge_builtin: ActorId,
        ethereum_event_client: ActorId,
        bridging_payment_service: ActorId,
        config: Config,
    ) -> Self {
        Self {
            receiver_contract_address,
            gear_bridge_builtin,
            ethereum_event_client,
            bridging_payment_service,
            config,
        }
    }
}
impl<T> VftTreasury<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            DATA = Some(VftTreasuryData {
                receiver_contract_address: config.receiver_contract_address,
                gear_bridge_builtin: config.gear_bridge_builtin,
                ethereum_event_client: config.ethereum_event_client,
                admin: exec_context.actor_id(),
                eth_to_vara_token_id: HashMap::new(),
                vara_to_eth_token_id: HashMap::new(),
                bridging_payment_service: config.bridging_payment_service,
            });
            CONFIG = Some(config.config);
            MSG_TRACKER = Some(MessageTracker::default());
        }
    }

    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn data(&self) -> &VftTreasuryData {
        unsafe { DATA.as_ref().expect("VftTreasury::seed() must be called") }
    }

    fn data_mut(&mut self) -> &mut VftTreasuryData {
        unsafe { DATA.as_mut().expect("VftTreasury::seed() must be called") }
    }

    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("VftTreasury::seed() must be invoked")
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

#[derive(Encode, Decode, TypeInfo)]
pub enum VftTreasuryEvents {
    Deposit {
        from: ActorId,
        to: H160,
        token: ActorId,
        amount: U256,
    },
    Withdraw {
        receiver: ActorId,

        token: ActorId,
        amount: U256,
    },
}

#[service(events = VftTreasuryEvents)]
impl<T> VftTreasury<T>
where
    T: ExecContext,
{
    pub fn update_config(&mut self, config: Config) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("not admin");
        }

        unsafe {
            CONFIG = Some(config);
        }
    }

    pub fn map_vara_to_eth_address(&mut self, ethereum_token: H160, vara_token: ActorId) {
        let data = self.data();

        if data.admin != self.exec_context.actor_id() {
            panic!("Not admin");
        }

        self.data_mut()
            .eth_to_vara_token_id
            .insert(ethereum_token, vara_token);
        self.data_mut()
            .vara_to_eth_token_id
            .insert(vara_token, ethereum_token);
    }

    pub fn unmap_vara_to_eth_address(&mut self, ethereum_token: H160, vara_token: ActorId) {
        let data = self.data();

        if data.admin != self.exec_context.actor_id() {
            panic!("Not admin");
        }

        self.data_mut().eth_to_vara_token_id.remove(&ethereum_token);
        self.data_mut().vara_to_eth_token_id.remove(&vara_token);
    }

    pub fn update_ethereum_event_client_address(&mut self, new_address: ActorId) {
        let data = self.data();
        if data.admin != self.exec_context.actor_id() {
            panic!("Not admin");
        }

        self.data_mut().ethereum_event_client = new_address;
    }

    pub fn update_bridging_payment_service_address(&mut self, new_address: ActorId) {
        let data = self.data();
        if data.admin != self.exec_context.actor_id() {
            panic!("Not admin");
        }

        self.data_mut().bridging_payment_service = new_address;
    }

    pub fn admin(&self) -> ActorId {
        self.data().admin
    }

    pub fn program_address(&self) -> ActorId {
        self.exec_context.actor_id()
    }

    pub fn get_config(&self) -> Config {
        self.config().clone()
    }

    pub fn gear_bridge_builtin(&self) -> ActorId {
        self.data().gear_bridge_builtin
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

    pub async fn deposit_tokens(
        &mut self,
        vara_token_id: ActorId,
        from: ActorId,
        amount: U256,
        to: H160,
    ) -> Result<(U256, H160), Error> {
        let data = self.data();
        let sender = self.exec_context.actor_id();

        if sender != data.bridging_payment_service {
            return Err(Error::NotBridgingClient);
        }

        let config = self.config();

        if gstd::exec::gas_available()
            < config.gas_for_transfer_tokens * 2
                + config.gas_for_reply_deposit * 3
                + config.gas_to_send_request_to_builtin
        {
            return Err(Error::NotEnoughGas);
        }

        let msg_id = gstd::msg::id();
        let eth_token_id = self.get_eth_token_id(&vara_token_id)?;

        token_operations::deposit_to_treasury(
            vara_token_id,
            eth_token_id,
            from,
            amount,
            to,
            config,
            msg_id,
        )
        .await?;
        debug!("Deposit tokens {}", amount);

        let nonce = match bridge_builtin_operations::send_message_to_bridge_builtin(
            data.gear_bridge_builtin,
            data.receiver_contract_address,
            to,
            eth_token_id,
            amount,
            config,
            msg_id,
        )
        .await
        {
            Ok(nonce) => nonce,
            Err(e) => {
                // In case of failure, take tokens from program address and send them back to the sender
                token_operations::withdraw_from_treasury(
                    vara_token_id,
                    eth_token_id,
                    from,
                    amount,
                    config,
                    msg_id,
                )
                .await?;
                return Err(e);
            }
        };

        Ok((nonce, eth_token_id))
    }

    pub async fn withdraw_tokens(
        &mut self,
        eth_token_id: H160,
        recepient: ActorId,
        amount: U256,
    ) -> Result<(), Error> {
        let data = self.data();
        let sender = self.exec_context.actor_id();

        let vara_token_id = data
            .eth_to_vara_token_id
            .get(&eth_token_id)
            .copied()
            .ok_or(Error::NoCorrespondingEthAddress)?;

        if sender != data.ethereum_event_client {
            return Err(Error::NotEthClient);
        }

        let config = self.config();

        if gstd::exec::gas_available()
            < config.gas_for_transfer_tokens + config.gas_for_reply_deposit
        {
            panic!("Please attach more gas");
        }

        let msg_id = gstd::msg::id();

        token_operations::withdraw_from_treasury(
            vara_token_id,
            eth_token_id,
            recepient,
            amount,
            config,
            msg_id,
        )
        .await
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

        let TxDetails::DepositVaraToTreasury {
            vara_token_id,
            eth_token_id,
            sender,
            amount,
            receiver,
        } = msg_info.details
        else {
            panic!("Wrong message type")
        };

        match msg_info.status {
            MessageStatus::TokenTransferCompleted(true) | MessageStatus::BridgeBuiltinStep => {
                match bridge_builtin_operations::send_message_to_bridge_builtin(
                    data.bridging_payment_service,
                    data.receiver_contract_address,
                    receiver,
                    eth_token_id,
                    amount,
                    config,
                    msg_id,
                )
                .await
                {
                    Ok(nonce) => Ok((nonce, eth_token_id)),
                    Err(_) => {
                        token_operations::withdraw_from_treasury(
                            vara_token_id,
                            eth_token_id,
                            sender,
                            amount,
                            config,
                            msg_id,
                        )
                        .await?;
                        Err(Error::TokensRefunded)
                    }
                }
            }

            MessageStatus::BridgeResponseReceived(Some(nonce)) => {
                msg_tracker_mut().remove_message_info(&msg_id);
                Ok((nonce, eth_token_id))
            }

            _ => panic!("Unexpected status or transaction completed"),
        }
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
