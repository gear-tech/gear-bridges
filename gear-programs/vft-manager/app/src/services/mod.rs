use collections::HashMap;
use sails_rs::{gstd::ExecContext, prelude::*};

pub mod abi;
mod bridge_builtin_operations;
pub mod error;
pub mod msg_tracker;
mod utils;
use error::Error;
use msg_tracker::{MessageInfo, MessageStatus, MessageTracker, TxDetails};
mod token_operations;

pub struct VftManager<ExecContext> {
    exec_context: ExecContext,
}

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    TokenMappingAdded {
        vara_token_id: ActorId,
        eth_token_id: H160,
    },
    TokenMappingRemoved {
        vara_token_id: ActorId,
        eth_token_id: H160,
    },
    BridgingRequested {
        nonce: U256,
        vara_token_id: ActorId,
        amount: U256,
        sender: ActorId,
        receiver: H160,
    },
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct Request {
    pub receiver: H160,
    pub token_id: H160,
    pub amount: U256,
}

static mut DATA: Option<VftManagerData> = None;
static mut CONFIG: Option<Config> = None;
static mut MSG_TRACKER: Option<MessageTracker> = None;

#[derive(Debug, Default)]
pub struct VftManagerData {
    gear_bridge_builtin: ActorId,
    admin: ActorId,
    receiver_contract_address: H160,
    vara_to_eth_token_id: HashMap<ActorId, H160>,
    eth_to_vara_token_id: HashMap<H160, ActorId>,
    eth_client: ActorId,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    pub receiver_contract_address: H160,
    pub gear_bridge_builtin: ActorId,
    pub eth_client: ActorId,
    pub config: Config,
}

impl InitConfig {
    pub fn new(
        receiver_contract_address: H160,
        gear_bridge_builtin: ActorId,
        eth_client: ActorId,
        config: Config,
    ) -> Self {
        Self {
            receiver_contract_address,
            gear_bridge_builtin,
            eth_client,
            config,
        }
    }
}

#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct Config {
    gas_to_burn_tokens: u64,
    gas_for_reply_deposit: u64,
    gas_to_mint_tokens: u64,
    gas_to_process_mint_request: u64,
    gas_to_send_request_to_builtin: u64,
    reply_timeout: u32,
    gas_for_transfer_to_eth_msg: u64,
    gas_for_event_sending: u64,
}

impl Config {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gas_to_burn_tokens: u64,
        gas_for_reply_deposit: u64,
        gas_to_mint_tokens: u64,
        gas_to_process_mint_request: u64,
        gas_to_send_request_to_builtin: u64,
        reply_timeout: u32,
        gas_for_transfer_to_eth_msg: u64,
        gas_for_event_sending: u64,
    ) -> Self {
        Self {
            gas_to_burn_tokens,
            gas_for_reply_deposit,
            gas_to_mint_tokens,
            gas_to_process_mint_request,
            gas_to_send_request_to_builtin,
            reply_timeout,
            gas_for_transfer_to_eth_msg,
            gas_for_event_sending,
        }
    }
}

#[service(events = Event)]
impl<T> VftManager<T>
where
    T: ExecContext,
{
    pub fn update_receiver_contract_address(&mut self, new_receiver_contract_address: H160) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }
        self.data_mut().receiver_contract_address = new_receiver_contract_address;
    }

    pub fn update_eth_client(&mut self, eth_client_new: ActorId) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }

        self.data_mut().eth_client = eth_client_new;
    }

    pub fn map_vara_to_eth_address(&mut self, vara_token_id: ActorId, eth_token_id: H160) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }

        let already_present = self
            .data_mut()
            .vara_to_eth_token_id
            .insert(vara_token_id, eth_token_id)
            .is_some();
        if already_present {
            panic!("Token mapping already exists");
        }

        let already_present = self
            .data_mut()
            .eth_to_vara_token_id
            .insert(eth_token_id, vara_token_id)
            .is_some();
        if already_present {
            panic!("Token mapping already exists");
        }

        self.notify_on(Event::TokenMappingAdded {
            vara_token_id,
            eth_token_id,
        })
        .expect("Failed to emit event");
    }

    pub fn remove_vara_to_eth_address(&mut self, vara_token_id: ActorId) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }

        let eth_token_id = self
            .data_mut()
            .vara_to_eth_token_id
            .remove(&vara_token_id)
            .expect("Token mapping not found");

        let _ = self
            .data_mut()
            .eth_to_vara_token_id
            .remove(&eth_token_id)
            .expect(
                "Should be present at this point due to the invariant of map_vara_to_eth_address",
            );

        self.notify_on(Event::TokenMappingRemoved {
            vara_token_id,
            eth_token_id,
        })
        .expect("Failed to emit event");
    }

    pub fn update_config(&mut self, config: Config) {
        if self.data().admin != self.exec_context.actor_id() {
            panic!("Not admin")
        }

        unsafe {
            CONFIG = Some(config);
        }
    }

    pub async fn mint_tokens(&mut self, receipt_rlp: Vec<u8>) -> Result<(), Error> {
        use abi::ERC20_TREASURY;
        use alloy_rlp::Decodable;
        use alloy_sol_types::SolEvent;
        use ethereum_common::utils::ReceiptEnvelope;

        let data = self.data();
        let sender = self.exec_context.actor_id();

        if sender != data.eth_client {
            return Err(Error::NotEthClient);
        }

        let config = self.config();
        if gstd::exec::gas_available()
            < config.gas_to_mint_tokens
                + config.gas_to_process_mint_request
                + config.gas_for_reply_deposit
        {
            return Err(Error::NotEnoughGas);
        }

        let receipt =
            ReceiptEnvelope::decode(&mut &receipt_rlp[..]).map_err(|_| Error::NotSupportedEvent)?;

        if !receipt.is_success() {
            return Err(Error::NotSupportedEvent);
        }

        // decode log and check that it is from an allowed address
        let (vara_token_id, event) = receipt
            .logs()
            .iter()
            .find_map(|log| {
                let address = H160::from(log.address.0 .0);
                let event = ERC20_TREASURY::Deposit::decode_log_data(log, true).ok()?;
                let eth_token_id = H160::from(event.token.0 .0);
                let vara_token_id = self.get_vara_token_id(&eth_token_id).ok()?;

                (self.receiver_contract_address() == address).then_some((vara_token_id, event))
            })
            .ok_or(Error::NotSupportedEvent)?;

        let amount = U256::from_little_endian(event.amount.as_le_slice());
        let receiver = ActorId::from(event.to.0);
        let msg_id = gstd::msg::id();
        let transaction_details = TxDetails::MintTokens {
            vara_token_id,
            receiver,
            amount,
        };
        msg_tracker_mut().insert_message_info(
            msg_id,
            MessageStatus::SendingMessageToMintTokens,
            transaction_details,
        );
        utils::set_critical_hook(msg_id);

        token_operations::mint_tokens(vara_token_id, receiver, amount, config, msg_id).await
    }

    pub async fn transfer_vara_to_eth(
        &mut self,
        sender: ActorId,
        vara_token_id: ActorId,
        amount: U256,
        receiver: H160,
    ) -> Result<(U256, H160), Error> {
        let data = self.data();
        let msg_id = gstd::msg::id();
        let eth_token_id = self.get_eth_token_id(&vara_token_id)?;
        let config = self.config();

        if gstd::exec::gas_available()
            < config.gas_to_burn_tokens
                + config.gas_to_send_request_to_builtin
                + config.gas_for_transfer_to_eth_msg
                + 3 * config.gas_for_reply_deposit
                + config.gas_for_event_sending
        {
            panic!("Please attach more gas");
        }

        token_operations::burn_tokens(vara_token_id, sender, receiver, amount, config, msg_id)
            .await?;
        let nonce = match bridge_builtin_operations::send_message_to_bridge_builtin(
            data.gear_bridge_builtin,
            data.receiver_contract_address,
            receiver,
            eth_token_id,
            amount,
            config,
            msg_id,
        )
        .await
        {
            Ok(nonce) => nonce,
            Err(e) => {
                // In case of failure, mint tokens back to the sender
                token_operations::mint_tokens(vara_token_id, sender, amount, config, msg_id)
                    .await?;
                return Err(e);
            }
        };

        self.notify_on(Event::BridgingRequested {
            nonce,
            vara_token_id,
            amount,
            sender,
            receiver,
        })
        .expect("Failed to emit event");

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

        let TxDetails::TransferVaraToEth {
            vara_token_id,
            sender,
            amount,
            receiver,
        } = msg_info.details
        else {
            panic!("Wrong message type")
        };

        let eth_token_id = self
            .get_eth_token_id(&vara_token_id)
            .expect("No corresponding Ethereum address for the specified Vara token address");

        match msg_info.status {
            MessageStatus::TokenBurnCompleted(true) | MessageStatus::BridgeBuiltinStep => {
                match bridge_builtin_operations::send_message_to_bridge_builtin(
                    data.gear_bridge_builtin,
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
                        // In case of failure, mint tokens back to the sender
                        token_operations::mint_tokens(
                            vara_token_id,
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
            MessageStatus::MintTokensStep => {
                token_operations::mint_tokens(vara_token_id, sender, amount, config, msg_id)
                    .await?;
                Err(Error::TokensRefunded)
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

    pub fn eth_client(&self) -> ActorId {
        self.data().eth_client
    }
}

impl<T> VftManager<T>
where
    T: ExecContext,
{
    pub fn seed(config: InitConfig, exec_context: T) {
        unsafe {
            DATA = Some(VftManagerData {
                gear_bridge_builtin: config.gear_bridge_builtin,
                receiver_contract_address: config.receiver_contract_address,
                admin: exec_context.actor_id(),
                eth_client: config.eth_client,
                ..Default::default()
            });
            CONFIG = Some(config.config);
            MSG_TRACKER = Some(MessageTracker::default());
        }
    }
    pub fn new(exec_context: T) -> Self {
        Self { exec_context }
    }

    fn data(&self) -> &VftManagerData {
        unsafe { DATA.as_ref().expect("VftManager::seed() should be called") }
    }

    fn data_mut(&mut self) -> &mut VftManagerData {
        unsafe { DATA.as_mut().expect("VftManager::seed() should be called") }
    }

    fn config(&self) -> &Config {
        unsafe {
            CONFIG
                .as_ref()
                .expect("VftManager::seed() should be called")
        }
    }

    fn get_eth_token_id(&self, vara_token_id: &ActorId) -> Result<H160, Error> {
        self.data()
            .vara_to_eth_token_id
            .get(vara_token_id)
            .cloned()
            .ok_or(Error::NoCorrespondingEthAddress)
    }

    fn get_vara_token_id(&self, eth_token_id: &H160) -> Result<ActorId, Error> {
        self.data()
            .eth_to_vara_token_id
            .get(eth_token_id)
            .cloned()
            .ok_or(Error::NoCorrespondingVaraAddress)
    }
}
fn msg_tracker() -> &'static MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_ref()
            .expect("VftManager::seed() should be called")
    }
}

fn msg_tracker_mut() -> &'static mut MessageTracker {
    unsafe {
        MSG_TRACKER
            .as_mut()
            .expect("VftManager::seed() should be called")
    }
}
