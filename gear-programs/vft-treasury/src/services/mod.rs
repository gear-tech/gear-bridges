use collections::HashMap;
use gstd::{debug, msg};
use sails_rs::{gstd::ExecContext, prelude::*};

mod utils;
mod vft;

pub struct VftTreasury<ExecContext> {
    exec_context: ExecContext,
}

static mut DATA: Option<VftTreasuryData> = None;

#[derive(Debug, Default)]
struct VftTreasuryData {
    admin: ActorId,
    ethereum_event_client: ActorId,
    bridging_payment_service: ActorId,
    eth_to_vara_token_id: HashMap<H160, ActorId>,
    vara_to_eth_token_id: HashMap<ActorId, H160>,
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    pub ethereum_event_client: ActorId,
    pub bridging_payment_service: ActorId,
}

impl InitConfig {
    pub fn new(ethereum_event_client: ActorId, bridging_payment_service: ActorId) -> Self {
        Self {
            ethereum_event_client,
            bridging_payment_service,
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
                ethereum_event_client: config.ethereum_event_client,
                admin: exec_context.actor_id(),
                eth_to_vara_token_id: HashMap::new(),
                vara_to_eth_token_id: HashMap::new(),
                bridging_payment_service: config.bridging_payment_service,
            });
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
    pub fn add_ethereum_to_vara_mapping(&mut self, ethereum_token: H160, vara_token: ActorId) {
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

    pub fn remove_ethereum_to_vara_mapping(&mut self, ethereum_token: H160) -> Option<ActorId> {
        let data = self.data();

        if data.admin != self.exec_context.actor_id() {
            panic!("Not admin");
        }

        self.data_mut().eth_to_vara_token_id.remove(&ethereum_token)
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

    pub async fn deposit(
        &mut self,
        sender: ActorId,
        token: ActorId,
        amount: U256,
        to: H160,
    ) -> H160 {
        let destination = gstd::exec::program_id();
        let eth_token_id = self
            .data()
            .vara_to_eth_token_id
            .get(&token)
            .copied()
            .expect("ethereum token not found");
        // transfer tokens to contract address
        let result = utils::transfer_tokens(token, sender, destination, amount)
            .await
            .expect("failed to transfer");
        debug!(
            "locking {} of tokens from {} in treasury ({}), transfer result = {}",
            amount, sender, destination, result
        );
        self.notify_on(VftTreasuryEvents::Deposit {
            from: sender,
            to,
            token,
            amount,
        })
        .expect("Error in depositing events");
        eth_token_id
    }

    pub async fn withdraw(&mut self, token: H160, recepient: ActorId, amount: U256) {
        if msg::source() != self.data().ethereum_event_client {
            panic!("Unable to unlock funds because message sender is not etherem event client");
        }
        let vara_token = self
            .data()
            .eth_to_vara_token_id
            .get(&token)
            .copied()
            .expect("ethereum to vara token mapping not found");
        let source = gstd::exec::program_id();
        utils::transfer_tokens(vara_token, source, recepient, amount)
            .await
            .expect("failed to transfer tokens");
        self.notify_on(VftTreasuryEvents::Withdraw {
            receiver: recepient,
            token: vara_token,
            amount,
        })
        .expect("Error in depositing events");
        debug!("Send {} tokens to {}", amount, recepient);
    }

    pub fn admin(&self) -> ActorId {
        self.data().admin
    }

    pub fn program_address(&self) -> ActorId {
        self.exec_context.actor_id()
    }
}
