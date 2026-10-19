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
use core::cell::{Cell, RefCell};
use sails_rs::{
    gstd::{msg, services::Service as _},
    prelude::*,
};

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
    outstanding_redemptions: Cell<u64>,
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

        // Pooled manager escrow must retain fractional claim remainders.
        // Awesome-sails reaps positive post-burn balances below its minimum.
        let balances = Balances::default();

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
            outstanding_redemptions: Cell::new(0),
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

    pub fn vft_admin(&self) -> VftAdmin<'_> {
        VftAdmin {
            base: self.base_vft_admin(),
            program: self,
        }
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
        vft_native_exchange_admin::Service::new(self.base_vft_admin_exposure())
    }
    pub fn native_escrow(&self) -> NativeEscrow<'_> {
        NativeEscrow { program: self }
    }
}

impl Program {
    // Internal only: exporting this service separately would reopen Exit.
    fn base_vft_admin(&self) -> vft_admin::Service<'_> {
        vft_admin::Service::new(
            &self.authorities,
            &self.allowances,
            &self.balances,
            &self.pause,
            self.vft2().emitter(),
        )
    }

    fn base_vft_admin_exposure(&self) -> vft_admin::ServiceExposure<vft_admin::Service<'_>> {
        // The existing SCALE VftAdmin route is call context, not a public accessor.
        self.base_vft_admin().expose(b"\x20VftAdmin")
    }
}

pub struct VftAdmin<'a> {
    base: vft_admin::Service<'a>,
    program: &'a Program,
}

#[service(extends = vft_admin::Service<'a>)]
impl<'a> VftAdmin<'a> {
    #[export]
    pub fn set_minimum_balance(&mut self, value: U256) {
        assert!(
            value.is_zero(),
            "Native escrow requires zero minimum balance"
        );
        self.program
            .base_vft_admin_exposure()
            .set_minimum_balance(value)
            .expect("Cannot set minimum balance");
    }

    #[export]
    pub fn exit(&mut self, inheritor: ActorId) {
        assert_eq!(
            Syscall::message_source(),
            self.program.authorities.borrow().admin(),
            "Not admin"
        );
        let _ = inheritor;
        panic!("Native wrapper retirement is unsupported");
    }
}

impl<'a> From<VftAdmin<'a>> for vft_admin::Service<'a> {
    fn from(value: VftAdmin<'a>) -> Self {
        value.base
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
    /// The original rejected payout's full returned value backs wrapped tokens
    /// credited exactly once to its original recipient. Not native delivery.
    Refunded,
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
            self.outstanding_redemptions() == 0,
            "Outstanding native obligations"
        );
        *self.program.escrow_manager.borrow_mut() = Some(manager);
    }

    #[export]
    pub fn manager(&self) -> Option<ActorId> {
        *self.program.escrow_manager.borrow()
    }

    #[export]
    pub fn outstanding_redemptions(&self) -> u64 {
        self.program.outstanding_redemptions.get()
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
            .base_vft_admin_exposure()
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
        self.program.outstanding_redemptions.set(
            self.outstanding_redemptions()
                .checked_add(1)
                .expect("Native obligation overflow"),
        );
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
                // Only this authenticated original error/full-value return authorizes
                // wrapped credit. Settlement must also work while new exposure is paused.
                let paused = self.program.pause.is_paused();
                self.program.pause.resume();
                // SAFETY: original child, recipient, exact backing and Queued status
                // were authenticated above; this synchronous mint cannot reenter.
                unsafe {
                    self.program
                        .base_vft_admin_exposure()
                        .do_mint(redemption.to, redemption.amount)
                        .expect("Cannot refund returned native backing");
                }
                if paused {
                    self.program.pause.pause();
                }
                PayoutStatus::Refunded
            }
            _ => PayoutStatus::Ambiguous,
        };
        if matches!(
            redemption.status,
            PayoutStatus::Delivered | PayoutStatus::Refunded
        ) {
            self.program.outstanding_redemptions.set(
                self.program
                    .outstanding_redemptions
                    .get()
                    .checked_sub(1)
                    .expect("Native obligation underflow"),
            );
        }
        true
    }
}
