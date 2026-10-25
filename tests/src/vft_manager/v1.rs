//! Exact 83eafd9e escrow ABI: no held-liability routes are available in this fixture.
use super::*;
use vft_manager_client::{CutoverPhase, ReceiptDepositOutcome, ReceiptStatus};
use vft_vara_v1_client::{traits::NativeEscrow as _, PayoutStatus};

fn binding(remoting: &GTestRemoting, native: ActorId) -> NativeWrapperConfig {
    NativeWrapperConfig {
        actor_id: native,
        ethereum_token: ERC20_TOKEN_GEAR_SUPPLY,
        code_id: H256::from(
            remoting
                .system()
                .submit_code(vft_vara_v1::WASM_BINARY)
                .into_bytes(),
        ),
        mode: NativeWrapperMode::EscrowV1,
    }
}

async fn fixture() -> Fixture {
    let fixture = setup_with_wrapper(Some(ReplyBehavior::Queued), 100, true).await;
    let Fixture {
        ref remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        ..
    } = fixture;
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(binding(remoting, native)))
        .send_recv(manager)
        .await
        .unwrap();
    let mut token_admin = VftAdminC::new(remoting.clone());
    token_admin.pause().send_recv(native).await.unwrap();
    vft_vara_v1_client::NativeEscrow::new(remoting.clone())
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    token_admin
        .set_burner(manager)
        .send_recv(native)
        .await
        .unwrap();
    token_admin.resume().send_recv(native).await.unwrap();
    admin.unpause().send_recv(manager).await.unwrap();
    fixture
}

async fn fund(remoting: &GTestRemoting, native: ActorId, manager: ActorId, raw: u128) {
    use vft_vara_v1_client::traits::VftNativeExchange as _;
    vft_vara_v1_client::VftNativeExchange::new(remoting.clone())
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    assert!(VftC::new(remoting.clone())
        .transfer(manager, U256::from(raw))
        .send_recv(native)
        .await
        .unwrap());
}

#[tokio::test]
async fn vft_manager_v1_paused_error_exact_retry_predicate_and_normal_delivery() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        ..
    } = fixture().await;
    let raw = 2_000_000_000_000u128;
    let recipient: ActorId = 110_001.into();
    remoting.system().mint_to(recipient, 100_000_000_000_000);
    fund(&remoting, native, manager, raw).await;
    let before = remoting.system().balance_of(recipient);
    let mut wrapper_admin = VftAdminC::new(remoting.clone());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, U256::from(raw));
    let mut admin = VftManagerC::new(remoting.clone());
    let (reply, events) = submit_receipt_events(&remoting, manager, 70, 0, receipt.clone());
    assert!(matches!(reply, Err(Error::ReplyFailure(_))));
    assert!(events.is_empty());
    let original = receipt_deposits(&admin, manager, 70, 0).await.remove(0);
    assert_eq!(original.outcome, ReceiptDepositOutcome::Retryable);
    let child = original.child.unwrap();
    assert!(admin
        .receipt_retryable_native(70, 0, original.log_index, original.operation_id, child)
        .recv(manager)
        .await
        .unwrap());
    for (slot, index, log, operation, child) in [
        (71, 0, original.log_index, original.operation_id, child),
        (70, 1, original.log_index, original.operation_id, child),
        (70, 0, original.log_index + 1, original.operation_id, child),
        (70, 0, original.log_index, H256([99; 32]), child),
        (
            70,
            0,
            original.log_index,
            original.operation_id,
            MessageId::from([99; 32]),
        ),
    ] {
        assert!(!admin
            .receipt_retryable_native(slot, index, log, operation, child)
            .recv(manager)
            .await
            .unwrap());
    }
    let escrow = vft_vara_v1_client::NativeEscrow::new(remoting.clone());
    assert_eq!(
        escrow
            .redemption(original.operation_id)
            .recv(native)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_eq!(
        balance_of(&remoting, native, manager).await,
        U256::from(raw)
    );
    wrapper_admin.resume().send_recv(native).await.unwrap();
    assert_eq!(
        submit_receipt_events(&remoting, manager, 70, 0, receipt.clone()).0,
        Err(Error::NativeSettlementPending)
    );
    let retried = receipt_deposits(&admin, manager, 70, 0).await.remove(0);
    assert_eq!(retried.operation_id, original.operation_id);
    assert_ne!(retried.child, original.child);
    assert!(!admin
        .receipt_retryable_native(70, 0, original.log_index, original.operation_id, child)
        .recv(manager)
        .await
        .unwrap());
    let queued = escrow
        .redemption(original.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (queued.from, queued.to, queued.amount),
        (manager, recipient, U256::from(raw))
    );
    assert_eq!(queued.status, PayoutStatus::Queued);
    assert_eq!(queued.returned_value, 0);
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_eq!(
        submit_receipt_events(&remoting, manager, 70, 0, receipt.clone()).0,
        Err(Error::NativeSettlementPending)
    );
    assert_eq!(
        escrow
            .redemption(original.operation_id)
            .recv(native)
            .await
            .unwrap(),
        Some(queued.clone())
    );
    admin.pause().send_recv(manager).await.unwrap();
    assert_eq!(
        resolve_receipt_events(&remoting, manager, 70, 0, &retried).0,
        Err(Error::NativeHoldUnsupported)
    );
    let mailbox = remoting.system().get_mailbox(recipient);
    let payout = Log::builder().source(native).dest(recipient);
    mailbox.claim_value(payout.clone()).unwrap();
    remoting.system().run_next_block();
    for _ in 0..2 {
        assert_eq!(
            admin
                .reconcile_receipt(70, 0)
                .send_recv(manager)
                .await
                .unwrap(),
            Ok(ReceiptStatus::Processed)
        );
    }
    let delivered = escrow
        .redemption(original.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.child, queued.child);
    assert_eq!(delivered.status, PayoutStatus::Delivered);
    assert_eq!(delivered.returned_value, 0);
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_eq!(remoting.system().balance_of(recipient), before + raw);
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    assert_eq!(balance_of(&remoting, native, manager).await, U256::zero());
    assert!(mailbox.claim_value(payout).is_err());
    admin.unpause().send_recv(manager).await.unwrap();
    assert_eq!(
        submit_receipt_events(&remoting, manager, 70, 0, receipt).0,
        Err(Error::AlreadyProcessed)
    );
}

#[tokio::test]
async fn vft_manager_v1_exact_full_original_refunds_once_and_zero_outstanding_handoff() {
    let Fixture {
        remoting,
        vft_manager_program_id: source,
        gear_supply_vft: native,
        eth_supply_vft: recipient,
    } = fixture().await;
    let raw = 2_000_000_000_000u128;
    let amount = U256::from(raw);
    fund(&remoting, native, source, raw * 2).await;
    let mut token = VftC::new(remoting.clone());
    let spender: ActorId = 110_002.into();
    assert!(token
        .approve(spender, U256::from(17u64))
        .send_recv(native)
        .await
        .unwrap());
    let before_backing = remoting.system().balance_of(native);
    let mut admin = VftManagerC::new(remoting.clone());
    let token_inventory = admin.vara_to_eth_addresses().recv(source).await.unwrap();
    let eth_supply = token.total_supply().recv(recipient).await.unwrap();
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, amount);
    assert_eq!(
        submit_receipt_events(&remoting, source, 71, 0, receipt.clone()).0,
        Err(Error::NativeSettlementPending)
    );
    remoting.system().run_next_block();
    let row = receipt_deposits(&admin, source, 71, 0).await.remove(0);
    let escrow = vft_vara_v1_client::NativeEscrow::new(remoting.clone());
    let returned = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned.status, PayoutStatus::Queued);
    assert_eq!(returned.returned_value, raw);
    assert_eq!(
        (returned.from, returned.to, returned.amount),
        (source, recipient, amount)
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    let mut wrapper_admin = VftAdminC::new(remoting.clone());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    admin.pause().send_recv(source).await.unwrap();
    assert_eq!(
        resolve_receipt_events(&remoting, source, 71, 0, &row).0,
        Err(Error::NativeHoldUnsupported)
    );
    for _ in 0..2 {
        assert_eq!(
            admin
                .reconcile_receipt(71, 0)
                .send_recv(source)
                .await
                .unwrap(),
            Ok(ReceiptStatus::Processed)
        );
    }
    let refunded = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        refunded,
        vft_vara_v1_client::Redemption {
            status: PayoutStatus::Refunded,
            ..returned
        }
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    let completed = receipt_deposits(&admin, source, 71, 0).await.remove(0);
    assert_eq!(completed.outcome, ReceiptDepositOutcome::Refunded);
    assert_eq!(completed.operation_id, row.operation_id);
    assert_eq!(completed.child, row.child);
    assert_eq!(balance_of(&remoting, native, recipient).await, amount);
    assert_eq!(balance_of(&remoting, native, source).await, amount);
    assert_eq!(token.total_supply().recv(native).await.unwrap(), amount * 2);
    assert_eq!(remoting.system().balance_of(native), before_backing);
    assert!(wrapper_admin.is_paused().recv(native).await.unwrap());
    // Immutable v1 has no BlockingRedemptions/HeldRedemption/HoldRedemption.
    // Successful reconciliation and handoff therefore exercise only its real ABI.
    let destination = upgrade_destination(&remoting, source).await;
    configure_cutover_destination(&remoting, source, destination).await;
    let approved = binding(&remoting, native);
    assert_eq!(
        admin.native_wrapper().recv(source).await.unwrap(),
        Some(approved.clone())
    );
    assert_eq!(
        admin.native_wrapper().recv(destination).await.unwrap(),
        Some(approved.clone())
    );
    let allowance = token
        .allowance(REMOTING_ACTOR_ID.into(), spender)
        .recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    admin
        .begin_cutover(destination, 72)
        .send_recv(source)
        .await
        .unwrap();
    admin
        .prepare_cutover(source, 72)
        .send_recv(destination)
        .await
        .unwrap();
    wrapper_admin.pause().send_recv(native).await.unwrap();
    vft_vara_v1_client::NativeEscrow::new(remoting.clone())
        .configure_manager(destination)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    assert!(admin.upgrade(destination).send_recv(source).await.is_err());
    assert_eq!(balance_of(&remoting, native, source).await, amount);
    wrapper_admin.pause().send_recv(native).await.unwrap();
    vft_vara_v1_client::NativeEscrow::new(remoting.clone())
        .configure_manager(source)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    admin.upgrade(destination).send_recv(source).await.unwrap();
    assert_eq!(
        admin.cutover_status().recv(source).await.unwrap().phase,
        CutoverPhase::Retired
    );
    assert!(admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .is_err());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    vft_vara_v1_client::NativeEscrow::new(remoting.clone())
        .configure_manager(destination)
        .send_recv(native)
        .await
        .unwrap();
    assert!(admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .is_err());
    wrapper_admin
        .set_burner(destination)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    wrapper_admin
        .set_minter(destination)
        .send_recv(recipient)
        .await
        .unwrap();
    wrapper_admin
        .set_burner(destination)
        .send_recv(recipient)
        .await
        .unwrap();
    admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .unwrap();
    assert_eq!(
        admin
            .cutover_status()
            .recv(destination)
            .await
            .unwrap()
            .phase,
        CutoverPhase::Ready
    );
    assert_eq!(
        admin.native_wrapper().recv(destination).await.unwrap(),
        Some(approved)
    );
    assert!(remoting.system().is_active_program(native));
    assert_eq!(
        escrow.manager().recv(native).await.unwrap(),
        Some(destination)
    );
    assert_eq!(
        wrapper_admin.burner().recv(native).await.unwrap(),
        destination
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_eq!(
        escrow
            .redemption(row.operation_id)
            .recv(native)
            .await
            .unwrap(),
        Some(refunded)
    );
    assert_eq!(
        receipt_deposits(&admin, source, 71, 0).await,
        vec![completed]
    );
    assert_eq!(balance_of(&remoting, native, source).await, U256::zero());
    assert_eq!(balance_of(&remoting, native, destination).await, amount);
    assert_eq!(balance_of(&remoting, native, recipient).await, amount);
    assert_eq!(token.total_supply().recv(native).await.unwrap(), amount * 2);
    assert_eq!(
        token
            .allowance(REMOTING_ACTOR_ID.into(), spender)
            .recv(native)
            .await
            .unwrap(),
        allowance
    );
    assert_eq!(remoting.system().balance_of(native), before_backing);
    assert_eq!(
        token.total_supply().recv(recipient).await.unwrap(),
        eth_supply
    );
    assert_eq!(
        admin.vara_to_eth_addresses().recv(source).await.unwrap(),
        token_inventory
    );
    assert_eq!(
        admin
            .vara_to_eth_addresses()
            .recv(destination)
            .await
            .unwrap(),
        token_inventory
    );
    assert!(remoting.system().is_active_program(recipient));
}

#[tokio::test]
async fn vft_manager_v1_queued_partial_exact_success_excess_ambiguous_block_cutover() {
    let raw = 2_000_000_000_000u128;
    // Success with value is ambiguous in immutable v1, even when exactly full.
    for returned in [None, Some(raw / 2), Some(raw), Some(raw + 1)] {
        let Fixture {
            remoting,
            vft_manager_program_id: source,
            gear_supply_vft: native,
            ..
        } = fixture().await;
        let recipient: ActorId = 110_003.into();
        remoting.system().mint_to(recipient, 100_000_000_000_000);
        fund(&remoting, native, source, raw * 2).await;
        let mut admin = VftManagerC::new(remoting.clone());
        let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, U256::from(raw));
        assert_eq!(
            submit_receipt_events(&remoting, source, 73, 0, receipt).0,
            Err(Error::NativeSettlementPending)
        );
        let row = receipt_deposits(&admin, source, 73, 0).await.remove(0);
        let escrow = vft_vara_v1_client::NativeEscrow::new(remoting.clone());
        let queued = escrow
            .redemption(row.operation_id)
            .recv(native)
            .await
            .unwrap()
            .unwrap();
        if let Some(value) = returned {
            remoting
                .system()
                .get_mailbox(recipient)
                .reply_bytes(
                    Log::builder().source(native).dest(recipient),
                    Vec::new(),
                    value,
                )
                .unwrap();
            remoting.system().run_next_block();
        }
        let original = escrow
            .redemption(row.operation_id)
            .recv(native)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original.child, queued.child);
        assert_eq!(original.returned_value, returned.unwrap_or(0));
        assert_eq!(
            original.status,
            if returned.is_some() {
                PayoutStatus::Ambiguous
            } else {
                PayoutStatus::Queued
            }
        );
        admin.pause().send_recv(source).await.unwrap();
        assert_eq!(
            resolve_receipt_events(&remoting, source, 73, 0, &row).0,
            Err(Error::NativeHoldUnsupported)
        );
        for _ in 0..2 {
            let _ = admin
                .reconcile_receipt(73, 0)
                .send_recv(source)
                .await
                .unwrap();
            assert_eq!(
                receipt_status(&admin, source, 73, 0).await,
                ReceiptStatus::Reserved
            );
            assert_eq!(
                escrow
                    .redemption(row.operation_id)
                    .recv(native)
                    .await
                    .unwrap(),
                Some(original.clone())
            );
        }
        let destination = upgrade_destination(&remoting, source).await;
        configure_cutover_destination(&remoting, source, destination).await;
        admin
            .begin_cutover(destination, 74)
            .send_recv(source)
            .await
            .unwrap();
        admin
            .prepare_cutover(source, 74)
            .send_recv(destination)
            .await
            .unwrap();
        assert!(admin.upgrade(destination).send_recv(source).await.is_err());
        assert!(admin
            .finalize_cutover()
            .send_recv(destination)
            .await
            .is_err());
        let mut token_admin = VftAdminC::new(remoting.clone());
        token_admin.pause().send_recv(native).await.unwrap();
        assert!(vft_vara_v1_client::NativeEscrow::new(remoting.clone())
            .configure_manager(destination)
            .send_recv(native)
            .await
            .is_err());
        assert_eq!(
            escrow.outstanding_redemptions().recv(native).await.unwrap(),
            1
        );
        assert_eq!(
            escrow
                .redemption(row.operation_id)
                .recv(native)
                .await
                .unwrap(),
            Some(original)
        );
        assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
        assert_eq!(balance_of(&remoting, native, source).await, U256::from(raw));
        assert_eq!(
            balance_of(&remoting, native, destination).await,
            U256::zero()
        );
        assert_eq!(
            VftC::new(remoting.clone())
                .total_supply()
                .recv(native)
                .await
                .unwrap(),
            U256::from(raw)
        );
        assert_eq!(
            receipt_status(&admin, source, 73, 0).await,
            ReceiptStatus::Reserved
        );
    }
}

#[tokio::test]
async fn vft_manager_v1_capability_freeze_and_original_fingerprint() {
    use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
    let Fixture {
        remoting,
        vft_manager_program_id: source,
        gear_supply_vft: native,
        ..
    } = fixture().await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(source).await.unwrap();
    let approved = binding(&remoting, native);
    let mut zero_code = approved.clone();
    zero_code.code_id = H256::zero();
    assert!(admin
        .configure_native_wrapper(Some(zero_code))
        .send_recv(source)
        .await
        .is_err());
    assert!(
        VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
            .configure_native_wrapper(Some(approved.clone()))
            .send_recv(source)
            .await
            .is_err()
    );
    for (actor, token) in [
        (ActorId::zero(), ERC20_TOKEN_GEAR_SUPPLY),
        (native, H160::zero()),
        (native, H160([99; 20])),
    ] {
        let mut invalid = approved.clone();
        invalid.actor_id = actor;
        invalid.ethereum_token = token;
        assert!(admin
            .configure_native_wrapper(Some(invalid))
            .send_recv(source)
            .await
            .is_err());
    }
    assert!(admin
        .map_vara_to_eth_address(native, H160([99; 20]), TokenSupply::Gear)
        .send_recv(source)
        .await
        .is_err());
    assert!(admin
        .map_vara_to_eth_address(
            ActorId::from(110_004u64),
            ERC20_TOKEN_GEAR_SUPPLY,
            TokenSupply::Gear
        )
        .send_recv(source)
        .await
        .is_err());
    assert!(admin
        .map_vara_to_eth_address(native, ERC20_TOKEN_GEAR_SUPPLY, TokenSupply::Ethereum)
        .send_recv(source)
        .await
        .is_err());
    admin.unpause().send_recv(source).await.unwrap();
    let recipient: ActorId = 110_005.into();
    // Permanent no-effect rejection is still a supported admin resolution in v1.
    assert_eq!(
        submit_receipt_events(
            &remoting,
            source,
            75,
            0,
            deposit_receipt(ActorId::zero(), ERC20_TOKEN_GEAR_SUPPLY, U256::one())
        )
        .0,
        Err(Error::ReceiptResolved)
    );
    let rejected = receipt_deposits(&admin, source, 75, 0).await.remove(0);
    admin.pause().send_recv(source).await.unwrap();
    assert_eq!(
        resolve_receipt_events(&remoting, source, 75, 0, &rejected).0,
        Ok(ReceiptDepositOutcome::Rejected)
    );
    admin
        .configure_native_wrapper(Some(approved.clone()))
        .send_recv(source)
        .await
        .unwrap();
    for field in 0..5 {
        let mut changed = approved.clone();
        match field {
            0 => changed.actor_id = recipient,
            1 => changed.ethereum_token = H160([98; 20]),
            2 => changed.code_id = H256([98; 32]),
            3 => changed.mode = NativeWrapperMode::HeldEscrowV2,
            _ => (),
        }
        let replacement = if field == 4 { None } else { Some(changed) };
        assert!(admin
            .configure_native_wrapper(replacement)
            .send_recv(source)
            .await
            .is_err());
        assert_eq!(
            admin.native_wrapper().recv(source).await.unwrap(),
            Some(approved.clone())
        );
    }
    let destination = upgrade_destination(&remoting, source).await;
    configure_cutover_destination(&remoting, source, destination).await;
    let mut mismatched = approved.clone();
    mismatched.mode = NativeWrapperMode::HeldEscrowV2;
    admin
        .configure_native_wrapper(Some(mismatched))
        .send_recv(destination)
        .await
        .unwrap();
    let mut mappings = admin.vara_to_eth_addresses().recv(source).await.unwrap();
    mappings.sort_unstable_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
    // This is the original immutable source's exact SCALE tuple, not the new config.
    let old_bytes = (
        *b"vft-manager/cutover-config/v1",
        ActorId::from(BRIDGE_BUILTIN_ID),
        ActorId::from(HISTORICAL_PROXY_ID),
        Some(ERC20_MANAGER_ADDRESS),
        Some(native),
        admin.get_config().recv(source).await.unwrap(),
        &mappings,
    )
        .encode();
    assert_eq!(
        &old_bytes[..b"vft-manager/cutover-config/v1".len()],
        b"vft-manager/cutover-config/v1"
    );
    let expected = KeccakHasher::hash(&old_bytes);
    admin
        .begin_cutover(destination, 76)
        .send_recv(source)
        .await
        .unwrap();
    assert_eq!(
        admin
            .cutover_status()
            .recv(source)
            .await
            .unwrap()
            .config_fingerprint,
        Some(expected)
    );
    assert!(admin
        .prepare_cutover(source, 76)
        .send_recv(destination)
        .await
        .is_err());
    admin
        .configure_native_wrapper(Some(approved))
        .send_recv(destination)
        .await
        .unwrap();
    admin
        .prepare_cutover(source, 76)
        .send_recv(destination)
        .await
        .unwrap();
    assert_eq!(
        admin
            .cutover_status()
            .recv(destination)
            .await
            .unwrap()
            .config_fingerprint,
        Some(expected)
    );
}

#[test]
fn vft_manager_v1_fixture_provenance_is_exact() {
    use sp_core::hashing::sha2_256;
    assert_eq!(
        sha2_256(include_bytes!("../../fixtures/vft-vara-v1/app/src/lib.rs")),
        hex_literal::hex!("084fef163e4188c0c2c4529c02989f0853c791ecd19930866f99525cc15baf0c")
    );
    let idl = include_bytes!("../../../api/gear/vft_vara_v1.idl");
    assert_eq!(
        sha2_256(idl),
        hex_literal::hex!("afd6509bb15544b1302dcd4c5659bd77ef07c7dfb98a292f0c4a99b59c73fca9")
    );
    let idl = std::str::from_utf8(idl).unwrap();
    assert!(idl.contains("query OutstandingRedemptions"));
    for unavailable in ["BlockingRedemptions", "HeldRedemption", "HoldRedemption"] {
        assert!(!idl.contains(unavailable));
    }
    assert_ne!(vft_vara_v1::WASM_BINARY, vft_vara::WASM_BINARY);
    assert!(vft_vara_v1::WASM_BINARY.starts_with(b"\0asm"));
    println!(
        "vft-vara-v1 wasm SHA256 {}",
        hex::encode(sha2_256(vft_vara_v1::WASM_BINARY))
    );
}

#[test]
fn vft_manager_v1_fingerprint_scale_golden() {
    let config = Config {
        gas_for_token_ops: 1,
        gas_for_reply_deposit: 2,
        gas_to_send_request_to_builtin: 3,
        gas_for_swap_token_maps: 4,
        reply_timeout: 5,
        fee_bridge: 6,
        fee_incoming: 7,
    };
    let wrapper = ActorId::from([4; 32]);
    let token = H160([3; 20]);
    let original_tuple = (
        *b"vft-manager/cutover-config/v1",
        ActorId::from([1; 32]),
        ActorId::from([2; 32]),
        Some(token),
        Some(wrapper),
        config,
        vec![(wrapper, token, TokenSupply::Gear)],
    );
    assert_eq!(
        original_tuple.encode(),
        hex::decode(include_str!("../../fixtures/vft-vara-v1/cutover-v1.hex").trim()).unwrap()
    );
}
