use gstd::{debug, exec, msg};
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
}

#[derive(Debug, Decode, Encode, TypeInfo)]
pub struct InitConfig {
    pub ethereum_event_client: ActorId,
}

impl InitConfig {
    pub fn new(ethereum_event_client: ActorId) -> Self {
        Self {
            ethereum_event_client,
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
    pub fn update_ethereum_event_client_address(&mut self, new_address: ActorId) {
        let data = self.data();
        if data.admin != self.exec_context.actor_id() {
            panic!("Not admin");
        }

        self.data_mut().ethereum_event_client = new_address;
    }

    pub async fn deposit(&mut self, token: ActorId, amount: U256, to: H160) {
        let source = msg::source();
        let destination = exec::program_id();
        // transfer tokens to contract address
        utils::transfer_tokens(token, source, destination, amount).await;
        debug!(
            "locking {} of tokens from {} in treasury ({})",
            amount, source, destination
        );
        self.notify_on(VftTreasuryEvents::Deposit {
            from: source,
            to,
            token,
            amount,
        })
        .expect("Error in depositing events");
    }

    pub async fn withdraw(&mut self, token: ActorId, recepient: ActorId, amount: U256) {
        if msg::source() != self.data().ethereum_event_client {
            panic!("Unable to unlock funds because message sender is not etherem event client");
        }
        let source = exec::program_id();
        utils::transfer_tokens(token, source, recepient, amount).await;
        self.notify_on(VftTreasuryEvents::Withdraw {
            receiver: recepient,
            token,
            amount,
        })
        .expect("Error in depositing events");
        debug!("Send {} tokens to {}", amount, recepient);
    }

    pub fn admin(&self) -> ActorId {
        self.data().admin
    }
}
