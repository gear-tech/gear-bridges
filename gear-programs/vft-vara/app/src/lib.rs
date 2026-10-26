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
    held_redemptions: RefCell<collections::BTreeMap<H256, Redemption>>,
    held_outstanding: Cell<u64>,
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
            authorities: RefCell::new(Authorities::from_one(Syscall::message_source())),
            allowances: Pausable::new(&pause, RefCell::new(allowances)),
            balances: Pausable::new(&pause, RefCell::new(balances)),
            metadata: RefCell::new(metadata),
            pause,
            escrow_manager: RefCell::new(None),
            redemptions: RefCell::new(collections::BTreeMap::new()),
            payout_children: RefCell::new(collections::BTreeMap::new()),
            outstanding_redemptions: Cell::new(0),
            held_redemptions: RefCell::new(collections::BTreeMap::new()),
            held_outstanding: Cell::new(0),
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
    /// Returned backing covers wrapped tokens credited exactly once at the
    /// original amount. Excess remains native backing; this is not delivery.
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
            self.blocking_redemptions() == 0,
            "Blocking native obligations"
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

    /// Financial obligations remain outstanding even when explicitly held for handoff.
    #[export]
    pub fn blocking_redemptions(&self) -> u64 {
        self.outstanding_redemptions()
            .checked_sub(self.program.held_outstanding.get())
            .expect("Held obligation count exceeds outstanding")
    }

    #[export]
    pub fn held_redemption(&self, operation_id: H256) -> Option<Redemption> {
        self.program
            .held_redemptions
            .borrow()
            .get(&operation_id)
            .cloned()
    }

    /// Assume the original liability without credit, redispatch or settlement.
    #[export]
    pub fn hold_redemption(
        &mut self,
        operation_id: H256,
        expected: Redemption,
        acknowledge_irreversible_hold: bool,
    ) -> Redemption {
        assert_eq!(
            Syscall::message_source(),
            self.program.authorities.borrow().admin(),
            "Not admin"
        );
        assert!(self.program.pause.is_paused(), "Not paused");
        if let Some(saved) = self.held_redemption(operation_id) {
            assert_eq!(saved, expected, "Conflicting hold");
            return saved;
        }
        let original = self.redemption(operation_id).expect("Missing redemption");
        assert_eq!(original, expected, "Redemption changed");
        assert!(!original.child.is_zero(), "Missing payout child");
        assert!(
            matches!(
                original.status,
                PayoutStatus::Queued | PayoutStatus::Returned | PayoutStatus::Ambiguous
            ),
            "Invalid held liability"
        );
        let still_resolvable = original.status == PayoutStatus::Queued
            || U256::from(original.returned_value) >= original.amount;
        assert!(
            !still_resolvable || acknowledge_irreversible_hold,
            "Acknowledge irreversible hold"
        );
        assert!(self.blocking_redemptions() > 0, "No blocking obligation");
        let held = self
            .program
            .held_outstanding
            .get()
            .checked_add(1)
            .expect("Held obligation overflow");
        self.program
            .held_redemptions
            .borrow_mut()
            .insert(operation_id, original.clone());
        self.program.held_outstanding.set(held);
        original
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
        if let Some(existing) = self.program.redemptions.borrow_mut().get_mut(&operation_id) {
            assert_eq!(
                (existing.from, existing.to, existing.amount),
                (from, to, amount),
                "Conflicting operation"
            );
            if self
                .program
                .held_redemptions
                .borrow()
                .contains_key(&operation_id)
            {
                return existing.clone();
            }
            // Only authenticated original backing authorizes this pending credit.
            // Never mint in handle_reply: a trapped reply is consumed.
            if matches!(
                existing.status,
                PayoutStatus::Queued | PayoutStatus::Returned | PayoutStatus::Ambiguous
            ) && U256::from(existing.returned_value) >= existing.amount
            {
                let paused = self.program.pause.is_paused();
                self.program.pause.resume();
                // SAFETY: the immutable original payout and full returned backing
                // authorize only its recipient. Failure rolls back this command,
                // not the already committed original reply.
                unsafe {
                    self.program
                        .base_vft_admin_exposure()
                        .do_mint(existing.to, existing.amount)
                        .expect("Cannot refund returned native backing");
                }
                if paused {
                    self.program.pause.pause();
                }
                existing.status = PayoutStatus::Refunded;
                self.complete_obligation(operation_id);
            }
            return existing.clone();
        }
        assert!(!self.program.pause.is_paused(), "Paused");
        assert!(
            !operation_id.is_zero() && !to.is_zero() && !amount.is_zero(),
            "Invalid redemption"
        );
        assert!(
            amount <= (U256::one() << 80) - U256::one(),
            "Native amount overflow"
        );
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
        let Ok(child) = Syscall::reply_to() else {
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
        if Syscall::message_source() != redemption.to
            || redemption.status != PayoutStatus::Queued
            || redemption.returned_value != 0
        {
            return true;
        }
        let value = Syscall::message_value();
        redemption.returned_value = value;
        redemption.status = match Syscall::reply_code() {
            Ok(ReplyCode::Success(_)) if value == 0 => PayoutStatus::Delivered,
            // Persist the authenticated backing before attempting wrapped credit
            // in a separate retryable RedeemEscrow command. Capacity exhaustion
            // (or an event-emission failure after mint) must not lose this reply.
            Ok(ReplyCode::Success(_) | ReplyCode::Error(_))
                if U256::from(value) >= redemption.amount =>
            {
                PayoutStatus::Queued
            }
            _ => PayoutStatus::Ambiguous,
        };
        if redemption.status == PayoutStatus::Delivered {
            self.complete_obligation(operation);
        }
        true
    }

    fn complete_obligation(&self, operation_id: H256) {
        let outstanding = self
            .program
            .outstanding_redemptions
            .get()
            .checked_sub(1)
            .expect("Native obligation underflow");
        let held = if self
            .program
            .held_redemptions
            .borrow()
            .contains_key(&operation_id)
        {
            self.program
                .held_outstanding
                .get()
                .checked_sub(1)
                .expect("Held obligation underflow")
        } else {
            self.program.held_outstanding.get()
        };
        self.program.outstanding_redemptions.set(outstanding);
        self.program.held_outstanding.set(held);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use awesome_sails_services::vft::utils::Balance;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    #[test]
    fn held_full_return_never_credits_and_snapshot_survives_callbacks() {
        let manager = ActorId::from(1_000u64);
        let recipient = ActorId::from(2_000u64);
        let operation = H256([17; 32]);
        let child = MessageId::from([18; 32]);
        Syscall::with_message_source(manager);
        let program = Program::new(Mainnet::No);
        *program.escrow_manager.borrow_mut() = Some(manager);
        let original = Redemption {
            from: manager,
            to: recipient,
            amount: 5u64.into(),
            child,
            status: PayoutStatus::Queued,
            returned_value: 0,
        };
        program
            .redemptions
            .borrow_mut()
            .insert(operation, original.clone());
        program
            .payout_children
            .borrow_mut()
            .insert(child, operation);
        program.outstanding_redemptions.set(1);
        let mut escrow = program.native_escrow();
        assert!(catch_unwind(AssertUnwindSafe(|| {
            escrow.hold_redemption(operation, original.clone(), true);
        }))
        .is_err());
        program.pause.pause();
        assert_eq!(
            escrow.hold_redemption(operation, original.clone(), true),
            original
        );
        assert_eq!(escrow.blocking_redemptions(), 0);
        Syscall::with_message_source(recipient);
        Syscall::with_reply_to(Ok(child));
        Syscall::with_reply_code(Ok(ReplyCode::Error(ErrorReplyReason::Execution(
            SimpleExecutionError::UserspacePanic,
        ))));
        Syscall::with_message_value(5);
        assert!(escrow.handle_payout_reply());
        let returned = escrow.redemption(operation).unwrap();
        assert_eq!(returned.returned_value, 5);
        assert_eq!(returned.status, PayoutStatus::Queued);
        Syscall::with_message_source(manager);
        assert_eq!(
            escrow.redeem_escrow(operation, manager, recipient, 5u64.into()),
            returned
        );
        assert_eq!(
            escrow.hold_redemption(operation, original.clone(), false),
            original
        );
        assert!(catch_unwind(AssertUnwindSafe(|| {
            escrow.hold_redemption(operation, returned.clone(), true);
        }))
        .is_err());
        assert_eq!(escrow.outstanding_redemptions(), 1);
        assert_eq!(escrow.blocking_redemptions(), 0);
        assert_eq!(program.vft2().total_supply().unwrap(), U256::zero());
        Syscall::with_message_source(recipient);
        Syscall::with_reply_code(Ok(ReplyCode::Success(SuccessReplyReason::Auto)));
        Syscall::with_message_value(0);
        assert!(escrow.handle_payout_reply());
        assert_eq!(escrow.redemption(operation), Some(returned));
        assert_eq!(escrow.held_redemption(operation), Some(original));
        assert_eq!(escrow.outstanding_redemptions(), 1);
    }

    #[test]
    fn hold_rejects_unauthorized_missing_changed_and_terminal_evidence() {
        let admin = ActorId::from(1_000u64);
        let outsider = ActorId::from(2_000u64);
        Syscall::with_message_source(admin);
        let program = Program::new(Mainnet::No);
        program.pause.pause();
        let operation = H256([19; 32]);
        let original = Redemption {
            from: admin,
            to: outsider,
            amount: 5u64.into(),
            child: MessageId::from([20; 32]),
            status: PayoutStatus::Returned,
            returned_value: 2,
        };
        let mut escrow = program.native_escrow();
        assert!(catch_unwind(AssertUnwindSafe(|| {
            escrow.hold_redemption(operation, original.clone(), true);
        }))
        .is_err());
        program
            .redemptions
            .borrow_mut()
            .insert(operation, original.clone());
        program.outstanding_redemptions.set(1);
        Syscall::with_message_source(outsider);
        assert!(catch_unwind(AssertUnwindSafe(|| {
            escrow.hold_redemption(operation, original.clone(), true);
        }))
        .is_err());
        Syscall::with_message_source(admin);
        let mut changed = original.clone();
        changed.returned_value = 3;
        assert!(catch_unwind(AssertUnwindSafe(|| {
            escrow.hold_redemption(operation, changed, true);
        }))
        .is_err());
        for invalid in [
            Redemption {
                child: MessageId::zero(),
                ..original.clone()
            },
            Redemption {
                status: PayoutStatus::Delivered,
                ..original.clone()
            },
            Redemption {
                status: PayoutStatus::Refunded,
                ..original.clone()
            },
        ] {
            program
                .redemptions
                .borrow_mut()
                .insert(operation, invalid.clone());
            assert!(catch_unwind(AssertUnwindSafe(|| {
                escrow.hold_redemption(operation, invalid, true);
            }))
            .is_err());
            assert_eq!(escrow.held_redemption(operation), None);
            assert_eq!(escrow.outstanding_redemptions(), 1);
            assert_eq!(escrow.blocking_redemptions(), 1);
        }
        program
            .redemptions
            .borrow_mut()
            .insert(operation, original.clone());
        assert_eq!(
            escrow.hold_redemption(operation, original.clone(), false),
            original
        );
        assert_eq!(escrow.redemption(operation), Some(original));
        assert_eq!(escrow.outstanding_redemptions(), 1);
        assert_eq!(escrow.blocking_redemptions(), 0);
    }

    #[test]
    fn full_return_survives_balance_capacity_failure_and_retries_once() {
        for returned_value in [5, 6] {
            for code in [
                ReplyCode::Success(SuccessReplyReason::Auto),
                ReplyCode::Error(ErrorReplyReason::Execution(
                    SimpleExecutionError::UserspacePanic,
                )),
            ] {
                for status in [
                    PayoutStatus::Queued,
                    PayoutStatus::Returned,
                    PayoutStatus::Ambiguous,
                ] {
                    refund_after_capacity_failure(returned_value, code, status);
                }
            }
        }
    }

    fn refund_after_capacity_failure(
        returned_value: u128,
        reply_code: ReplyCode,
        retry_status: PayoutStatus,
    ) {
        let manager = ActorId::from(1_000u64);
        let recipient = ActorId::from(2_000u64);
        let operation = H256([7; 32]);
        let child = MessageId::from([8; 32]);
        let amount = U256::from(5u64);
        Syscall::with_message_source(manager);
        let mut program = Program::new(Mainnet::No);
        let mut balances = Balances::try_new(vec![1], Balance::from(0u64)).unwrap();
        balances.allocate_next_shard();
        // Fill the actual allocated shard, then partially burn manager escrow:
        // the burn leaves no free holder slot for the rejected payout's recipient.
        for index in 0..balances.capacity() {
            balances
                .mint(
                    ActorId::from(1_000 + index as u64).try_into().unwrap(),
                    Balance::from(10u64).try_into().unwrap(),
                )
                .unwrap();
        }
        balances
            .burn(
                manager.try_into().unwrap(),
                Balance::from(5u64).try_into().unwrap(),
            )
            .unwrap();
        program.balances = Pausable::new(&program.pause, RefCell::new(balances));
        *program.escrow_manager.borrow_mut() = Some(manager);
        program.redemptions.borrow_mut().insert(
            operation,
            Redemption {
                from: manager,
                to: recipient,
                amount,
                child,
                status: PayoutStatus::Queued,
                returned_value: 0,
            },
        );
        program
            .payout_children
            .borrow_mut()
            .insert(child, operation);
        program.outstanding_redemptions.set(1);
        Syscall::with_reply_to(Ok(child));
        Syscall::with_reply_code(Ok(reply_code));
        Syscall::with_message_value(returned_value);
        // A forged source cannot authorize credit.
        assert!(program.native_escrow().handle_payout_reply());
        assert_eq!(
            program
                .native_escrow()
                .redemption(operation)
                .unwrap()
                .returned_value,
            0
        );
        Syscall::with_message_source(recipient);
        assert!(program.native_escrow().handle_payout_reply());
        let pending = program.native_escrow().redemption(operation).unwrap();
        assert_eq!(pending.status, PayoutStatus::Queued);
        assert_eq!(pending.returned_value, returned_value);
        assert_eq!(program.native_escrow().outstanding_redemptions(), 1);
        // Even a contradictory duplicate of the original callback cannot erase backing.
        Syscall::with_reply_code(Ok(ReplyCode::Success(SuccessReplyReason::Auto)));
        Syscall::with_message_value(0);
        assert!(program.native_escrow().handle_payout_reply());
        assert_eq!(
            program.native_escrow().redemption(operation),
            Some(pending.clone())
        );
        // Retained Returned/Ambiguous records with full backing can also recover.
        program
            .redemptions
            .borrow_mut()
            .get_mut(&operation)
            .unwrap()
            .status = retry_status;
        let pending = program.native_escrow().redemption(operation).unwrap();
        Syscall::with_message_source(manager);
        let supply = program.vft2().total_supply().unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| {
            program
                .native_escrow()
                .redeem_escrow(operation, manager, recipient, amount);
        }))
        .is_err());
        assert_eq!(program.native_escrow().redemption(operation), Some(pending));
        assert_eq!(program.native_escrow().outstanding_redemptions(), 1);
        assert_eq!(program.vft2().total_supply().unwrap(), supply);
        assert_eq!(program.vft2().balance_of(recipient).unwrap(), U256::zero());
        program
            .base_vft_admin_exposure()
            .append_balances_shard(1)
            .unwrap();
        program
            .vft_extension()
            .allocate_next_balances_shard()
            .unwrap();
        program.pause.pause();
        let refunded = program
            .native_escrow()
            .redeem_escrow(operation, manager, recipient, amount);
        assert_eq!(refunded.status, PayoutStatus::Refunded);
        assert_eq!(refunded.returned_value, returned_value);
        assert_eq!(refunded.child, child);
        assert!(program.pause.is_paused());
        assert_eq!(program.native_escrow().outstanding_redemptions(), 0);
        assert_eq!(program.vft2().balance_of(recipient).unwrap(), amount);
        assert_eq!(program.vft2().total_supply().unwrap(), supply + amount);
        assert_eq!(
            program
                .native_escrow()
                .redeem_escrow(operation, manager, recipient, amount),
            refunded,
        );
        assert_eq!(program.vft2().total_supply().unwrap(), supply + amount);
        assert_eq!(program.payout_children.borrow().len(), 1);
    }

    #[test]
    fn resolvable_holds_require_explicit_consent() {
        for (status, returned_value) in [
            (PayoutStatus::Queued, 0),
            (PayoutStatus::Queued, 5),
            (PayoutStatus::Queued, 6),
            (PayoutStatus::Returned, 5),
            (PayoutStatus::Returned, 6),
            (PayoutStatus::Ambiguous, 5),
            (PayoutStatus::Ambiguous, 6),
        ] {
            let manager = ActorId::from(1_000u64);
            Syscall::with_message_source(manager);
            let program = Program::new(Mainnet::No);
            program.pause.pause();
            let operation = H256([27; 32]);
            let original = Redemption {
                from: manager,
                to: ActorId::from(2_000u64),
                amount: 5u64.into(),
                child: MessageId::from([28; 32]),
                status,
                returned_value,
            };
            program
                .redemptions
                .borrow_mut()
                .insert(operation, original.clone());
            program.outstanding_redemptions.set(1);
            let mut escrow = program.native_escrow();
            assert!(catch_unwind(AssertUnwindSafe(|| {
                escrow.hold_redemption(operation, original.clone(), false);
            }))
            .is_err());
            assert_eq!(escrow.held_redemption(operation), None);
            assert_eq!(escrow.blocking_redemptions(), 1);
            assert_eq!(
                escrow.hold_redemption(operation, original.clone(), true),
                original
            );
            assert_eq!(
                escrow.hold_redemption(operation, original.clone(), false),
                original
            );
            assert_eq!(escrow.blocking_redemptions(), 0);
            assert_eq!(escrow.outstanding_redemptions(), 1);
        }
    }

    #[test]
    fn partial_ambiguous_hold_needs_no_irreversible_consent() {
        let manager = ActorId::from(1_000u64);
        Syscall::with_message_source(manager);
        let program = Program::new(Mainnet::No);
        program.pause.pause();
        let operation = H256([29; 32]);
        let original = Redemption {
            from: manager,
            to: ActorId::from(2_000u64),
            amount: 5u64.into(),
            child: MessageId::from([30; 32]),
            status: PayoutStatus::Ambiguous,
            returned_value: 2,
        };
        program
            .redemptions
            .borrow_mut()
            .insert(operation, original.clone());
        program.outstanding_redemptions.set(1);
        assert_eq!(
            program
                .native_escrow()
                .hold_redemption(operation, original.clone(), false),
            original
        );
        assert_eq!(program.native_escrow().blocking_redemptions(), 0);
        assert_eq!(program.native_escrow().outstanding_redemptions(), 1);
    }

    #[test]
    fn payout_reply_retains_actual_backing_without_minting() {
        for success in [false, true] {
            for returned_value in [0, 2, 5, 6] {
                let manager = ActorId::from(1_000u64);
                let recipient = ActorId::from(2_000u64);
                Syscall::with_message_source(manager);
                let program = Program::new(Mainnet::No);
                let operation = H256([31; 32]);
                let child = MessageId::from([32; 32]);
                program.redemptions.borrow_mut().insert(
                    operation,
                    Redemption {
                        from: manager,
                        to: recipient,
                        amount: 5u64.into(),
                        child,
                        status: PayoutStatus::Queued,
                        returned_value: 0,
                    },
                );
                program
                    .payout_children
                    .borrow_mut()
                    .insert(child, operation);
                program.outstanding_redemptions.set(1);
                Syscall::with_message_source(recipient);
                Syscall::with_reply_to(Ok(child));
                Syscall::with_reply_code(Ok(if success {
                    ReplyCode::Success(SuccessReplyReason::Auto)
                } else {
                    ReplyCode::Error(ErrorReplyReason::Execution(
                        SimpleExecutionError::UserspacePanic,
                    ))
                }));
                Syscall::with_message_value(returned_value);
                assert!(program.native_escrow().handle_payout_reply());
                let result = program.native_escrow().redemption(operation).unwrap();
                let expected = if success && returned_value == 0 {
                    PayoutStatus::Delivered
                } else if returned_value >= 5 {
                    PayoutStatus::Queued
                } else {
                    PayoutStatus::Ambiguous
                };
                assert_eq!(result.status, expected);
                assert_eq!(result.returned_value, returned_value);
                assert_eq!(result.child, child);
                assert_eq!(program.vft2().total_supply().unwrap(), U256::zero());
                assert_eq!(
                    program.native_escrow().outstanding_redemptions(),
                    u64::from(expected != PayoutStatus::Delivered)
                );
            }
        }
    }

    #[test]
    fn held_late_delivery_completes_counters_but_keeps_saved_snapshot() {
        let manager = ActorId::from(1_000u64);
        let recipient = ActorId::from(2_000u64);
        Syscall::with_message_source(manager);
        let program = Program::new(Mainnet::No);
        program.pause.pause();
        let operation = H256([33; 32]);
        let child = MessageId::from([34; 32]);
        let original = Redemption {
            from: manager,
            to: recipient,
            amount: 5u64.into(),
            child,
            status: PayoutStatus::Queued,
            returned_value: 0,
        };
        program
            .redemptions
            .borrow_mut()
            .insert(operation, original.clone());
        program
            .payout_children
            .borrow_mut()
            .insert(child, operation);
        program.outstanding_redemptions.set(1);
        let mut escrow = program.native_escrow();
        escrow.hold_redemption(operation, original.clone(), true);
        Syscall::with_message_source(recipient);
        Syscall::with_reply_to(Ok(child));
        Syscall::with_reply_code(Ok(ReplyCode::Success(SuccessReplyReason::Auto)));
        Syscall::with_message_value(0);
        assert!(escrow.handle_payout_reply());
        assert_eq!(
            escrow.redemption(operation).unwrap().status,
            PayoutStatus::Delivered
        );
        assert_eq!(escrow.outstanding_redemptions(), 0);
        assert_eq!(escrow.blocking_redemptions(), 0);
        assert_eq!(escrow.held_redemption(operation), Some(original.clone()));
        assert!(escrow.handle_payout_reply());
        Syscall::with_message_source(manager);
        assert_eq!(
            escrow.hold_redemption(operation, original.clone(), false),
            original
        );
        assert_eq!(escrow.outstanding_redemptions(), 0);
    }

    #[test]
    fn oversized_native_amount_rejects_before_burning_or_dispatching() {
        let manager = ActorId::from(1_000u64);
        Syscall::with_message_source(manager);
        let program = Program::new(Mainnet::No);
        *program.escrow_manager.borrow_mut() = Some(manager);
        let operation = H256([35; 32]);
        let error = catch_unwind(AssertUnwindSafe(|| {
            program.native_escrow().redeem_escrow(
                operation,
                manager,
                ActorId::from(2_000u64),
                U256::one() << 80,
            );
        }))
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<&str>(),
            Some(&"Native amount overflow")
        );
        assert_eq!(program.vft2().total_supply().unwrap(), U256::zero());
        assert_eq!(program.native_escrow().redemption(operation), None);
        assert_eq!(program.native_escrow().outstanding_redemptions(), 0);
        assert!(program.payout_children.borrow().is_empty());
    }
}
