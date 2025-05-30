//! Bridging Payment service implementation.

use gstd::{exec, static_mut, static_ref};
use sails_rs::{
    gstd::{msg,},
    prelude::*,
};

/// Bridging Payment service.
pub struct BridgingPayment;

/// Events emitted by Bridging Payment service.
#[derive(Encode, Decode, TypeInfo)]
pub enum BridgingPaymentEvents {
    /// Fee for the message processing by relayer was paid.
    BridgingPaid {
        /// Nonce of the message that was paid for.
        nonce: U256,
    },
}

static mut STATE: Option<State> = None;

/// Global state of the Bridging Payment service.
#[derive(Debug, Decode, Encode, TypeInfo, Clone)]
pub struct State {
    /// Admin of this service. Admin is in charge of:
    /// - Changing fee
    /// - Withdrawing collected fees from the program address
    /// - Updating [State] of this service
    pub admin_address: ActorId,
    /// Fee amount that will be charged from users.
    pub fee: u128,
}

impl BridgingPayment
{
    /// Initialize state of the Bridging Payment service.
    pub fn seed(initial_state: State) {
        unsafe {
            STATE = Some(initial_state);
        }
    }

    /// Create Bridging Payment service.
    pub fn new() -> Self {
        Self {  }
    }

    fn state(&self) -> &State {
        unsafe { static_ref!(STATE).as_ref() }.expect("BridgingPayment::seed() should be called")
    }

    fn state_mut(&mut self) -> &mut State {
        unsafe { static_mut!(STATE).as_mut() }.expect("BridgingPayment::seed() should be called")
    }
}

#[service(events = BridgingPaymentEvents)]
impl BridgingPayment
{
    /// Set fee that this program will take from incoming requests.
    ///
    /// This method can be called only by admin.
    pub fn set_fee(&mut self, fee: u128) {
        self.ensure_admin();

        self.state_mut().fee = fee;
    }

    /// Withdraw fees that were collected from user requests.
    ///
    /// This method can be called only by admin.
    pub fn reclaim_fee(&mut self) {
        self.ensure_admin();

        let fee_balance = exec::value_available();
        msg::send(self.state().admin_address, "", fee_balance).expect("Failed to reclaim fees");
    }

    /// Set new admin.
    ///
    /// This method can be called only by admin.
    pub fn set_admin(&mut self, new_admin: ActorId) {
        self.ensure_admin();

        self.state_mut().admin_address = new_admin;
    }

    fn ensure_admin(&self) {
        if self.state().admin_address != Syscall::program_id() {
            panic!("Not an admin")
        }
    }

    /// Pay fees for message processing to the admin.
    ///
    /// This method requires that **exactly** [Config::fee] must
    /// be attached as a value when sending message to this method.
    ///
    /// Current fee amount can be retreived by calling `get_state`.
    pub async fn pay_fees(&mut self, nonce: U256) {
        let fee = self.state().fee;

        let attached_value = msg::value();
        if attached_value != fee {
            panic!("Please attach exactly {} value", fee);
        }

        self.emit_event(BridgingPaymentEvents::BridgingPaid { nonce })
            .expect("Error depositing event");
    }

    /// Get current service [State].
    pub fn get_state(&self) -> State {
        self.state().clone()
    }
}
