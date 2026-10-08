use super::{
    message_sender::{MessageSender, MessageSenderIo},
    proof_composer::{ProofComposer, ProofComposerIo},
    storage::{InboundRuntimeIdentity, JSONStorage, ManualInboundIdentity, Storage},
    tx_manager::{TransactionManager, TxStatus},
};
use crate::message_relayer::common::{
    gear::{
        block_listener::BlockListener as GearBlockListener,
        checkpoints_extractor::CheckpointsExtractor,
    },
    EthereumSlotNumber, TxHashWithSlot,
};
use anyhow::{Context, Result as AnyResult};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{PollingEthApi, TxHash};
use ethereum_common::SECONDS_PER_SLOT;
use gear_common::api_provider::ApiProviderConnection;
use primitive_types::H256;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Arc,
};
use subxt::{
    config::{substrate::BlakeTwo256, Hasher},
    utils::AccountId32,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

#[allow(clippy::too_many_arguments)]
pub async fn relay(
    mut provider_connection: ApiProviderConnection,
    gear_suri: String,
    eth_api: PollingEthApi,
    beacon_client: BeaconClient,
    checkpoint_light_client_address: H256,
    historical_proxy_address: H256,
    receiver_address: H256,
    receiver_route: Vec<u8>,
    tx_hash: TxHash,
    storage_path: &Path,
) -> AnyResult<()> {
    let _journal_lock = lock_journal(storage_path)?;
    let ethereum_genesis_hash = eth_api.get_block(0).await?.header.hash;
    eth_api
        .enable_finality_archive(
            &storage_path.join("ethereum-finality"),
            ethereum_genesis_hash,
        )
        .await?;
    let original = eth_api
        .get_finalized_receipt(tx_hash)
        .await?
        .context("HOLD: manual Ethereum transaction has no canonical finalized receipt")?;
    let block = eth_api.get_block(original.included_block_number).await?;
    anyhow::ensure!(
        block.header.hash == original.included_block_hash,
        "HOLD: original manual Ethereum inclusion changed"
    );
    let genesis_time = beacon_client
        .get_genesis()
        .await
        .context("Failed to fetch chain genesis")?
        .data
        .genesis_time;
    let elapsed = block
        .header
        .timestamp
        .checked_sub(genesis_time)
        .context("HOLD: original Ethereum timestamp precedes Beacon genesis")?;
    anyhow::ensure!(
        elapsed % SECONDS_PER_SLOT == 0,
        "HOLD: original Ethereum timestamp does not identify a Beacon slot"
    );
    let slot_number = EthereumSlotNumber(elapsed / SECONDS_PER_SLOT);
    let target = ManualInboundIdentity {
        tx_hash,
        slot_number,
        ethereum_block_hash: original.included_block_hash.0.into(),
        transaction_index: original
            .receipt
            .transaction_index
            .context("HOLD: original manual Ethereum receipt has no transaction index")?,
        receiver_route: receiver_route.clone(),
    };
    let client = provider_connection
        .gclient_client(&gear_suri)
        .context("Failed to create gclient")?;
    let gear_genesis_hash = crate::rpc::retry_gear(
        &mut provider_connection,
        "manual inbound Gear genesis",
        |api| async move { api.block_number_to_hash(0).await },
    )
    .await?;
    let runtime = InboundRuntimeIdentity {
        ethereum_chain_id: eth_api.chain_id().await?,
        ethereum_genesis_hash: ethereum_genesis_hash.0.into(),
        ethereum_start_block: original.included_block_number,
        erc20_manager_address: None,
        bridging_payment_address: None,
        gear_genesis_hash,
        vft_manager_address: receiver_address,
        checkpoint_light_client_address,
        historical_proxy_address,
        gear_sender: client.account_id().clone().into(),
    };
    let storage = Arc::new(JSONStorage::new(storage_path));
    let tx_manager = restore_manager(storage.clone(), &runtime, &target).await?;

    let gear_block_listener = GearBlockListener::new(
        provider_connection.clone(),
        Arc::new(crate::message_relayer::common::gear::block_storage::NoStorage),
    );
    let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);
    let latest_checkpoint =
        super::get_latest_checkpoint(checkpoint_light_client_address, client).await;
    let message_sender = MessageSender::new(
        receiver_address,
        receiver_route,
        historical_proxy_address,
        provider_connection.clone(),
        gear_suri.clone(),
        None,
    );
    let proof_composer = ProofComposer::new(
        provider_connection,
        beacon_client,
        eth_api,
        historical_proxy_address,
        gear_suri,
    );
    let [gear_blocks] = gear_block_listener.run().await;
    let checkpoints = checkpoints_extractor
        .run(gear_blocks, latest_checkpoint)
        .await;
    let (events_sender, mut events_receiver) = unbounded_channel();
    events_sender.send(TxHashWithSlot {
        tx_hash,
        slot_number,
    })?;
    let mut sender_io = message_sender.run();
    let mut composer_io = proof_composer.run(checkpoints);
    run_manual(
        &tx_manager,
        &mut events_receiver,
        &mut composer_io,
        &mut sender_io,
    )
    .await
}

fn lock_journal(path: &Path) -> AnyResult<fs::File> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir() && metadata.permissions().mode() & 0o777 == 0o700,
        "HOLD: manual inbound journal must be a private 0700 directory, not a symlink"
    );
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.join("manual.lock"))?;
    anyhow::ensure!(
        lock.metadata()?.is_file() && lock.metadata()?.permissions().mode() & 0o777 == 0o600,
        "HOLD: manual inbound journal lock must be a private regular file"
    );
    lock.try_lock()
        .context("HOLD: another manual inbound relay owns this journal")?;
    Ok(lock)
}

async fn restore_manager(
    storage: Arc<JSONStorage>,
    runtime: &InboundRuntimeIdentity,
    target: &ManualInboundIdentity,
) -> AnyResult<TransactionManager> {
    storage
        .bind_manual_identity(runtime.clone(), target.clone())
        .await?;
    let manager = TransactionManager::new(storage.clone());
    storage.load(&manager).await?;
    let active = manager.transactions.read().await;
    let completed = manager.completed.read().await;
    let failed = manager.failed.read().await;
    anyhow::ensure!(
        active.len() + completed.len() <= 1,
        "HOLD: manual inbound journal contains multiple operations"
    );
    anyhow::ensure!(
        failed
            .keys()
            .all(|id| active.contains_key(id) || completed.contains_key(id)),
        "HOLD: manual inbound journal has failed work without its original transaction"
    );
    for tx in active.values().chain(completed.values()) {
        anyhow::ensure!(
            tx.tx.tx_hash == target.tx_hash && tx.tx.slot_number == target.slot_number,
            "HOLD: manual inbound journal belongs to another original transaction/slot"
        );
        anyhow::ensure!(
            !matches!(tx.status, TxStatus::Failed { .. }),
            "HOLD: manual inbound transaction failed; preserve the original journal"
        );
        if let Some(receipt) = &tx.receipt {
            let event = tx.receipt_event()?;
            let payload = &event.proof_block.block.body.execution_payload;
            anyhow::ensure!(
                event.transaction_index == target.transaction_index
                    && payload.block_number == runtime.ethereum_start_block
                    && H256::from(payload.block_hash.0 .0) == target.ethereum_block_hash,
                "HOLD: manual inbound receipt differs from the original inclusion/index"
            );
            let payload_hash = H256::from(alloy_primitives::keccak256(&receipt.payload).0);
            for signed in receipt.signed_submission.iter().chain(
                receipt
                    .submission_attempts
                    .iter()
                    .map(|attempt| &attempt.signed_submission),
            ) {
                signed.validate_native_reconciliation()?;
                anyhow::ensure!(
                    signed.chain_genesis_hash.parse::<H256>().ok()
                        == Some(runtime.gear_genesis_hash)
                        && signed.manager.parse::<H256>().ok() == Some(runtime.vft_manager_address)
                        && signed.historical_proxy.parse::<H256>().ok()
                            == Some(runtime.historical_proxy_address)
                        && signed
                            .sender
                            .parse::<AccountId32>()
                            .ok()
                            .map(|account| account.0)
                            == Some(runtime.gear_sender)
                        && signed.receipt_key == receipt.receipt_key
                        && signed.payload_hash.parse::<H256>().ok() == Some(payload_hash)
                        && !signed.raw_extrinsic.is_empty()
                        && signed.extrinsic_hash.parse::<H256>().ok()
                            == Some(H256::from(BlakeTwo256.hash(&signed.raw_extrinsic).0)),
                    "HOLD: original manual signed submission identity or exact bytes changed"
                );
            }
        }
    }
    drop(failed);
    drop(completed);
    drop(active);
    Ok(manager)
}

async fn run_manual(
    manager: &TransactionManager,
    events: &mut UnboundedReceiver<TxHashWithSlot>,
    composer: &mut ProofComposerIo,
    sender: &mut MessageSenderIo,
) -> AnyResult<()> {
    loop {
        let running = manager.process(events, composer, sender).await?;
        if !manager.completed.read().await.is_empty() {
            return Ok(());
        }
        let transactions = manager.transactions.read().await;
        let failed = manager.failed.read().await;
        for tx in transactions.values() {
            if matches!(
                tx.status,
                TxStatus::Failed { .. } | TxStatus::PreparingSubmission
            ) && failed.contains_key(&tx.uuid)
            {
                anyhow::bail!(
                    "HOLD: manual inbound relay failed: {}; preserve its original journal",
                    failed[&tx.uuid]
                );
            }
        }
        anyhow::ensure!(running, "HOLD: manual inbound handlers stopped before completion; preserve the original journal");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::{
            message_sender::{MessageStatus, Request, Response, SignedSubmission},
            tx_manager::{ReceiptEvidence, Transaction},
        },
        *,
    };
    use sails_rs::{calls::ActionIo, Encode};
    use serde::Deserialize;
    use std::io::Read;

    fn original_receipt() -> (TxHashWithSlot, eth_events_electra_client::EthToVaraEvent) {
        use alloy_rlp::Encodable;
        use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof};
        use ethereum_common::{
            beacon::electra::Block,
            utils::{self, BeaconBlockHeaderResponse, BeaconBlockResponse},
        };
        #[derive(Deserialize)]
        struct Receipts {
            result: Vec<alloy::rpc::types::TransactionReceipt>,
        }
        #[derive(Deserialize)]
        struct Fixture {
            tx_hash: TxHash,
            tx_index: u64,
            slot_number: u64,
            receipts: Receipts,
            block: BeaconBlockResponse<Block>,
            headers: Vec<BeaconBlockHeaderResponse>,
        }
        let bytes: &[u8] = include_bytes!("../../../../tests/src/relayer/transactions.json.zst");
        let mut decoder = ruzstd::StreamingDecoder::new(bytes).unwrap();
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        let fixtures: Vec<Fixture> = serde_json::from_slice(&decoded).unwrap();
        let fixture = fixtures
            .into_iter()
            .min_by_key(|fixture| fixture.tx_hash)
            .unwrap();
        let receipts = fixture
            .receipts
            .result
            .iter()
            .map(|receipt| {
                (
                    receipt.transaction_index.unwrap(),
                    utils::map_receipt_envelope(receipt.as_ref()),
                )
            })
            .collect::<Vec<_>>();
        let proof = utils::generate_merkle_proof(fixture.tx_index, &receipts).unwrap();
        let mut receipt_rlp = Vec::new();
        Encodable::encode(&proof.receipt, &mut receipt_rlp);
        let block = fixture.block.data.message;
        let event = eth_events_electra_client::EthToVaraEvent {
            proof_block: BlockInclusionProof {
                block: BlockGenericForBlockBody {
                    slot: block.slot,
                    proposer_index: block.proposer_index,
                    parent_root: block.parent_root,
                    state_root: block.state_root,
                    body: block.body.into(),
                },
                headers: fixture
                    .headers
                    .into_iter()
                    .map(|header| header.data.header.message)
                    .collect(),
            },
            proof: proof.proof,
            transaction_index: fixture.tx_index,
            receipt_rlp,
        };
        (
            TxHashWithSlot {
                slot_number: EthereumSlotNumber(fixture.slot_number),
                tx_hash: fixture.tx_hash,
            },
            event,
        )
    }

    #[tokio::test]
    async fn manual_restart_holds_changed_targets_and_reconciles_original_signed_attempt() {
        let path = std::env::temp_dir().join(format!("manual-inbound-{}", uuid::Uuid::now_v7()));
        let lock = lock_journal(&path).unwrap();
        assert!(
            lock_journal(&path).is_err(),
            "a second owner must not reach signing"
        );
        let (original, event) = original_receipt();
        let execution = &event.proof_block.block.body.execution_payload;
        let runtime = InboundRuntimeIdentity {
            ethereum_chain_id: 560048,
            ethereum_genesis_hash: H256::repeat_byte(1),
            ethereum_start_block: execution.block_number,
            erc20_manager_address: None,
            bridging_payment_address: None,
            gear_genesis_hash: H256::repeat_byte(3),
            vft_manager_address: H256::repeat_byte(4),
            checkpoint_light_client_address: H256::repeat_byte(5),
            historical_proxy_address: H256::repeat_byte(6),
            gear_sender: [7; 32],
        };
        let target = ManualInboundIdentity {
            tx_hash: original.tx_hash,
            slot_number: original.slot_number,
            ethereum_block_hash: H256::from(execution.block_hash.0 .0),
            transaction_index: event.transaction_index,
            receiver_route: vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec(),
        };
        let storage = Arc::new(JSONStorage::new(&path));
        let manager = restore_manager(storage.clone(), &runtime, &target)
            .await
            .unwrap();
        let intent = tokio::fs::read(path.join("state.json")).await.unwrap();
        let mut unbound: serde_json::Value = serde_json::from_slice(&intent).unwrap();
        unbound.as_object_mut().unwrap().remove("runtime_identity");
        let unbound = serde_json::to_vec(&unbound).unwrap();
        tokio::fs::write(path.join("state.json"), &unbound)
            .await
            .unwrap();
        assert!(
            restore_manager(Arc::new(JSONStorage::new(&path)), &runtime, &target)
                .await
                .is_err(),
            "a durable pre-handoff intent cannot adopt a lost runtime identity"
        );
        assert_eq!(
            tokio::fs::read(path.join("state.json")).await.unwrap(),
            unbound
        );
        tokio::fs::write(path.join("state.json"), &intent)
            .await
            .unwrap();
        let signed = SignedSubmission {
            chain_genesis_hash: format!("{:#x}", runtime.gear_genesis_hash),
            manager: format!("{:#x}", runtime.vft_manager_address),
            historical_proxy: format!("{:#x}", runtime.historical_proxy_address),
            sender: AccountId32::from(runtime.gear_sender).to_string(),
            receipt_key: (target.slot_number.0, target.transaction_index),
            payload_hash: format!("{:#x}", alloy_primitives::keccak256(event.encode())),
            nonce: 53,
            extrinsic_hash: format!("{:#x}", BlakeTwo256.hash(&[1, 2, 3, 4])),
            raw_extrinsic: vec![1, 2, 3, 4],
            prepared_finalized_number: 50,
            prepared_finalized_hash: "0x06".into(),
            scanned_finalized_number: 50,
            scanned_finalized_hash: "0x06".into(),
            reply_scanned_finalized_number: 50,
            reply_scanned_finalized_hash: "0x06".into(),
            inclusion_block_number: None,
            inclusion_block_hash: None,
            message_id: None,
            finalized_dispatch_error: None,
            finalized_reply: None,
            native_reconciliation: None,
        };
        let mut tx = Transaction::new(original.clone(), TxStatus::PreparedSubmission);
        let uuid = tx.uuid;
        tx.receipt = Some(ReceiptEvidence {
            payload: event.encode(),
            receipt_key: signed.receipt_key,
            composed_at_ms: 10,
            handed_off_at_ms: None,
            initial_response: None,
            signed_submission: Some(signed.clone()),
            submission_attempts: Vec::new(),
        });
        manager.add_transaction(tx).await;
        storage.save(&manager).await.unwrap();
        let committed = tokio::fs::read(path.join("state.json")).await.unwrap();
        drop(manager);
        drop(storage);
        for change in [
            "transaction",
            "slot",
            "block",
            "index",
            "route",
            "start",
            "eth-chain",
            "eth-genesis",
            "gear-genesis",
            "receiver",
            "checkpoint",
            "proxy",
            "sender",
        ] {
            let mut changed_runtime = runtime.clone();
            let mut changed_target = target.clone();
            match change {
                "transaction" => changed_target.tx_hash = TxHash::from([9; 32]),
                "slot" => changed_target.slot_number.0 += 1,
                "block" => changed_target.ethereum_block_hash = H256::repeat_byte(9),
                "index" => changed_target.transaction_index += 1,
                "route" => changed_target.receiver_route.push(9),
                "start" => changed_runtime.ethereum_start_block += 1,
                "eth-chain" => changed_runtime.ethereum_chain_id += 1,
                "eth-genesis" => changed_runtime.ethereum_genesis_hash = H256::repeat_byte(9),
                "gear-genesis" => changed_runtime.gear_genesis_hash = H256::repeat_byte(9),
                "receiver" => changed_runtime.vft_manager_address = H256::repeat_byte(9),
                "checkpoint" => {
                    changed_runtime.checkpoint_light_client_address = H256::repeat_byte(9)
                }
                "proxy" => changed_runtime.historical_proxy_address = H256::repeat_byte(9),
                "sender" => changed_runtime.gear_sender = [9; 32],
                _ => unreachable!(),
            }
            let result = restore_manager(
                Arc::new(JSONStorage::new(&path)),
                &changed_runtime,
                &changed_target,
            )
            .await;
            assert!(
                result.is_err(),
                "{change} must HOLD before any handler starts"
            );
            assert_eq!(
                tokio::fs::read(path.join("state.json")).await.unwrap(),
                committed,
                "{change} must preserve the original evidence"
            );
        }
        for corruption in [
            "mismatched-key",
            "signed-unsigned-collision",
            "active-completed-collision",
        ] {
            let mut malformed: serde_json::Value = serde_json::from_slice(&committed).unwrap();
            let original = malformed["transactions"][uuid.to_string()].clone();
            let foreign_key = "ffffffff-ffff-ffff-ffff-ffffffffffff";
            match corruption {
                "mismatched-key" => {
                    malformed["transactions"]
                        .as_object_mut()
                        .unwrap()
                        .remove(&uuid.to_string());
                    malformed["transactions"][foreign_key] = original;
                }
                "signed-unsigned-collision" => {
                    let mut unsigned = original;
                    unsigned["status"] = serde_json::json!("SubmitMessage");
                    unsigned["receipt"]["signed_submission"] = serde_json::Value::Null;
                    malformed["transactions"][foreign_key] = unsigned;
                }
                "active-completed-collision" => {
                    let mut completed = original;
                    completed["status"] = serde_json::json!("Completed");
                    malformed["completed"][uuid.to_string()] = completed;
                }
                _ => unreachable!(),
            }
            let malformed = serde_json::to_vec(&malformed).unwrap();
            tokio::fs::write(path.join("state.json"), &malformed)
                .await
                .unwrap();
            assert!(
                restore_manager(Arc::new(JSONStorage::new(&path)), &runtime, &target)
                    .await
                    .is_err(),
                "{corruption} must hold before returning a manager that can request signing"
            );
            assert_eq!(
                tokio::fs::read(path.join("state.json")).await.unwrap(),
                malformed
            );
        }
        tokio::fs::write(path.join("state.json"), &committed)
            .await
            .unwrap();
        let mut cached: serde_json::Value = serde_json::from_slice(&committed).unwrap();
        cached["transactions"][uuid.to_string()]["status"] = serde_json::json!("Completed");
        tokio::fs::write(
            path.join("state.json"),
            serde_json::to_vec(&cached).unwrap(),
        )
        .await
        .unwrap();
        let held = restore_manager(Arc::new(JSONStorage::new(&path)), &runtime, &target)
            .await
            .unwrap();
        assert!(held.completed.read().await.is_empty());
        assert!(matches!(
            held.transactions.read().await[&uuid].status,
            TxStatus::NeedsReconciliation { .. }
        ));
        assert_eq!(
            held.transactions.read().await[&uuid]
                .receipt
                .as_ref()
                .unwrap()
                .signed_submission
                .as_ref(),
            Some(&signed)
        );
        tokio::fs::write(path.join("state.json"), &committed)
            .await
            .unwrap();
        let mut corrupted: serde_json::Value = serde_json::from_slice(&committed).unwrap();
        corrupted["transactions"][uuid.to_string()]["status"] = serde_json::json!("Completed");
        corrupted["transactions"][uuid.to_string()]["receipt"]["signed_submission"]
            ["raw_extrinsic"][0] = serde_json::json!(99);
        let corrupted = serde_json::to_vec(&corrupted).unwrap();
        tokio::fs::write(path.join("state.json"), &corrupted)
            .await
            .unwrap();
        assert!(
            restore_manager(Arc::new(JSONStorage::new(&path)), &runtime, &target)
                .await
                .is_err(),
            "a cached completion cannot bypass original signed-byte verification"
        );
        assert_eq!(
            tokio::fs::read(path.join("state.json")).await.unwrap(),
            corrupted
        );
        tokio::fs::write(path.join("state.json"), &committed)
            .await
            .unwrap();
        let automatic_storage = JSONStorage::new(&path);
        let mut automatic_runtime = runtime.clone();
        automatic_runtime.erc20_manager_address = Some(primitive_types::H160::repeat_byte(2));
        assert!(automatic_storage
            .bind_runtime_identity(automatic_runtime)
            .await
            .is_err());
        let manager = restore_manager(Arc::new(JSONStorage::new(&path)), &runtime, &target)
            .await
            .unwrap();
        let (events, mut received_events) = unbounded_channel();
        let (requests, mut sent) = unbounded_channel();
        let (responses, received) = unbounded_channel();
        let mut sender = MessageSenderIo::new(requests, received);
        let (proof_requests, mut proofs_sent) = unbounded_channel();
        let (_proof_responses, received) = unbounded_channel();
        let mut composer = ProofComposerIo::new(proof_requests, received);
        events.send(original.clone()).unwrap();
        assert!(manager
            .process(&mut received_events, &mut composer, &mut sender)
            .await
            .unwrap());
        match sent.try_recv().unwrap() {
            Request::SubmitPrepared {
                tx_uuid,
                tx_hash,
                prepared,
                ..
            } => {
                assert_eq!(tx_uuid, uuid);
                assert_eq!(tx_hash, original.tx_hash);
                assert_eq!(prepared.1, signed);
            }
            other => panic!(
                "restart requested new work rather than the original signed attempt: {other:?}"
            ),
        }
        assert!(matches!(
            proofs_sent.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        responses
            .send(Response {
                tx_uuid: uuid,
                status: MessageStatus::Failure("unsupported Ethereum event".into()),
                submission: None,
                signed_submission: Some(signed.clone()),
            })
            .unwrap();
        events.send(original).unwrap();
        let error = run_manual(&manager, &mut received_events, &mut composer, &mut sender)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unsupported Ethereum event"));
        let tx = manager.transactions.read().await[&uuid].clone();
        assert!(matches!(tx.status, TxStatus::Failed { .. }));
        assert_eq!(
            tx.receipt.unwrap().submission_attempts[0].signed_submission,
            signed
        );
        assert!(matches!(
            sent.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        drop(lock);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
}
