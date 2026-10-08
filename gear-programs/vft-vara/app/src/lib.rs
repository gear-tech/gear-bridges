#![no_std]

use awesome_sails_services::{
    vft::{
        self,
        utils::{Allowances, Balances},
    },
    vft_admin::{
        self,
        utils::{Pausable, Pause},
        Authorities,
    },
    vft_extension,
    vft_metadata::{self, Metadata},
    vft_native_exchange, vft_native_exchange_admin,
};
use core::cell::RefCell;
use sails_rs::{gstd::msg, prelude::*};

/// Specifies the network for deployment of VFT-VARA
#[derive(Decode, TypeInfo)]
#[scale_info(crate = sails_rs::scale_info)]
#[codec(crate = sails_rs::scale_codec)]
pub enum Mainnet {
    Yes,
    No,
}

pub struct Program {
    authorities: RefCell<Authorities>,
    allowances: Pausable<RefCell<Allowances>>,
    balances: Pausable<RefCell<Balances>>,
    metadata: RefCell<Metadata>,
    pause: Pause,
    escrow_manager: RefCell<Option<ActorId>>,
    redemptions: RefCell<collections::BTreeMap<H256, Redemption>>,
    payout_children: RefCell<collections::BTreeMap<MessageId, H256>>,
}

#[program]
impl Program {
    pub fn new(network: Mainnet) -> Self {
        let pause = Pause::default();

        // Allowance is represented as 9 bytes unsigned int.
        // The maximum value is 4,722,366,482,869,645,213,695.
        let mut allowances = Allowances::default();
        // 24h / 3sec block
        allowances.set_expiry_period(24 * 60 * 60 / 3);

        // Minimum balance is zero by default.
        //
        // Balance is represented as 10 bytes unsigned int.
        // The maximum value is 1,208,925,819,614,629,174,706,175.
        let mut balances = Balances::default();
        balances.set_minimum_balance(1_000_000_000_000u64.into());

        let metadata = match network {
            Mainnet::Yes => Metadata::new("Wrapped Vara".into(), "WVARA".into(), 12),

            Mainnet::No => Metadata::new("Wrapped Testnet Vara".into(), "WTVARA".into(), 12),
        };

        Self {
            authorities: RefCell::new(Authorities::from_one(msg::source())),
            allowances: Pausable::new(&pause, RefCell::new(allowances)),
            balances: Pausable::new(&pause, RefCell::new(balances)),
            metadata: RefCell::new(metadata),
            pause,
            escrow_manager: RefCell::new(None),
            redemptions: RefCell::new(collections::BTreeMap::new()),
            payout_children: RefCell::new(collections::BTreeMap::new()),
        }
    }

    #[allow(dead_code)]
    #[handle_reply]
    fn handle_reply(&self) {
        if !self.native_escrow().handle_payout_reply() {
            self.vft_native_exchange_admin().handle_reply()
        }
    }

    pub fn vft(&self) -> vft_common::Service<'_> {
        vft_common::Service::new(&self.allowances, &self.balances, &self.metadata)
    }

    pub fn vft2(&self) -> vft::Service<'_> {
        vft::Service::new(&self.allowances, &self.balances)
    }

    pub fn vft_admin(&self) -> vft_admin::Service<'_> {
        vft_admin::Service::new(
            &self.authorities,
            &self.allowances,
            &self.balances,
            &self.pause,
            self.vft2().emitter(),
        )
    }

    pub fn vft_extension(&self) -> vft_extension::Service<'_> {
        vft_extension::Service::new(&self.allowances, &self.balances, self.vft2().emitter())
    }

    pub fn vft_metadata(&self) -> vft_metadata::Service<'_> {
        vft_metadata::Service::new(&self.metadata)
    }

    pub fn vft_native_exchange(&self) -> vft_native_exchange::Service<'_> {
        vft_native_exchange::Service::new(&self.balances, self.vft2().emitter())
    }

    pub fn vft_native_exchange_admin(&self) -> vft_native_exchange_admin::Service<'_> {
        vft_native_exchange_admin::Service::new(self.vft_admin())
    }
    pub fn native_escrow(&self) -> NativeEscrow<'_> {
        NativeEscrow { program: self }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum PayoutStatus {
    Queued,
    Delivered,
    Returned,
    Ambiguous,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub struct Redemption {
    pub from: ActorId,
    pub to: ActorId,
    pub amount: U256,
    pub child: MessageId,
    pub status: PayoutStatus,
    /// Native value actually returned by the original payout child.
    pub returned_value: u128,
}

pub struct NativeEscrow<'a> {
    program: &'a Program,
}

#[service]
impl NativeEscrow<'_> {
    #[export]
    pub fn configure_manager(&mut self, manager: ActorId) {
        assert_eq!(
            Syscall::message_source(),
            self.program.authorities.borrow().admin(),
            "Not admin"
        );
        assert!(self.program.pause.is_paused(), "Not paused");
        assert!(!manager.is_zero(), "Invalid manager");
        assert!(
            self.program
                .redemptions
                .borrow()
                .values()
                .all(|r| r.status == PayoutStatus::Delivered),
            "Outstanding native obligations"
        );
        *self.program.escrow_manager.borrow_mut() = Some(manager);
    }

    #[export]
    pub fn manager(&self) -> Option<ActorId> {
        *self.program.escrow_manager.borrow()
    }

    /// This acknowledgement proves only enqueueing. Settlement requires the original
    /// payout reply; a user mailbox entry is still an outstanding native obligation.
    #[export]
    pub fn redeem_escrow(
        &mut self,
        operation_id: H256,
        from: ActorId,
        to: ActorId,
        amount: U256,
    ) -> Redemption {
        assert_eq!(
            self.manager(),
            Some(Syscall::message_source()),
            "Not manager"
        );
        assert_eq!(from, Syscall::message_source(), "Not manager escrow");
        if let Some(existing) = self.program.redemptions.borrow().get(&operation_id) {
            assert_eq!(
                (existing.from, existing.to, existing.amount),
                (from, to, amount),
                "Conflicting operation"
            );
            return existing.clone();
        }
        assert!(!self.program.pause.is_paused(), "Paused");
        assert!(
            !operation_id.is_zero() && !to.is_zero() && !amount.is_zero(),
            "Invalid redemption"
        );
        assert!(amount <= U256::from(u128::MAX), "Native amount overflow");
        self.program
            .vft_admin()
            .burn(from, amount)
            .expect("Cannot burn manager escrow");
        let child =
            msg::send_bytes(to, [], amount.as_u128()).expect("Cannot enqueue native payout");
        sails_rs::gstd::exec::reply_deposit(child, 5_000_000_000)
            .expect("Cannot fund payout reply");
        let redemption = Redemption {
            from,
            to,
            amount,
            child,
            status: PayoutStatus::Queued,
            returned_value: 0,
        };
        self.program
            .redemptions
            .borrow_mut()
            .insert(operation_id, redemption.clone());
        self.program
            .payout_children
            .borrow_mut()
            .insert(child, operation_id);
        redemption
    }

    #[export]
    pub fn redemption(&self, operation_id: H256) -> Option<Redemption> {
        self.program
            .redemptions
            .borrow()
            .get(&operation_id)
            .cloned()
    }
}

impl NativeEscrow<'_> {
    fn handle_payout_reply(&self) -> bool {
        let Ok(child) = msg::reply_to() else {
            return false;
        };
        let Some(operation) = self.program.payout_children.borrow().get(&child).copied() else {
            return false;
        };
        let mut redemptions = self.program.redemptions.borrow_mut();
        let redemption = redemptions
            .get_mut(&operation)
            .expect("Original payout missing");
        // Exact original child and recipient, including late and duplicate replies.
        if Syscall::message_source() != redemption.to || redemption.status != PayoutStatus::Queued {
            return true;
        }
        let value = Syscall::message_value();
        redemption.returned_value = value;
        redemption.status = match msg::reply_code() {
            Ok(ReplyCode::Success(_)) if value == 0 => PayoutStatus::Delivered,
            Ok(ReplyCode::Error(_)) if U256::from(value) == redemption.amount => {
                PayoutStatus::Returned
            }
            _ => PayoutStatus::Ambiguous,
        };
        true
    }
}
