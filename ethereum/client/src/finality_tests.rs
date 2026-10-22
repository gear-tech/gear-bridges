//! Independent transports and real signed transaction/receipt roots; no proof bypasses.
use super::{tests::header_chain as chain, *};
use beacon_finality::test_fixtures::{single_account_proof, LocalBeaconFixture};
use alloy::{
    consensus::{
        proofs::{calculate_receipt_root, calculate_transaction_root},
        SignableTransaction, TxEip1559, TxEnvelope,
    },
    eips::Encodable2718,
    network::TxSigner,
    primitives::TxKind,
    rpc::types::BlockTransactions,
    sol_types::SolCall,
};
use serde_json::json;
use std::collections::BTreeSet;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
struct State {
    chain: Vec<Block>,
    tip: usize,
    signed_tip: usize,
    chain_id: u64,
    missing: BTreeSet<u64>,
    unavailable: bool,
    http_failure: Option<(&'static str, u16)>,
    rpc_failure: Option<&'static str>,
    receipt: Option<TransactionReceipt>,
    receipts: Vec<TransactionReceipt>,
    nonce: u64,
    account_proofs: BTreeMap<B256, alloy::rpc::types::EIP1186AccountProofResponse>,
    processed: bool,
    root: B256,
    requests: Vec<Value>,
}

struct Fixture {
    url: String,
    state: Arc<std::sync::Mutex<State>>,
    server: tokio::task::JoinHandle<()>,
    beacon: tokio::sync::Mutex<Option<LocalBeaconFixture>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(chain: Vec<Block>, tip: usize) -> AnyResult<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(62))?;
        let (state_root, proof) = single_account_proof(signer.address(), 7);
        let account_proofs = chain.iter()
            .filter(|block| block.header.state_root == state_root)
            .map(|block| (block.header.hash, proof.clone()))
            .collect();
        let state = Arc::new(std::sync::Mutex::new(State {
            chain,
            tip,
            signed_tip: tip,
            chain_id: 31337,
            missing: BTreeSet::new(),
            unavailable: false,
            http_failure: None,
            rpc_failure: None,
            receipt: None,
            receipts: vec![],
            nonce: 7,
            account_proofs,
            processed: true,
            root: B256::repeat_byte(67),
            requests: vec![],
        }));
        let shared = state.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let shared = shared.clone();
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let request = loop {
                        let mut chunk = [0; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..count]);
                        let Some(end) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                        else {
                            continue;
                        };
                        let headers = std::str::from_utf8(&buffer[..end]).unwrap();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, value)| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if buffer.len() >= end + 4 + length {
                            break serde_json::from_slice::<Value>(
                                &buffer[end + 4..end + 4 + length],
                            )
                            .unwrap();
                        }
                    };
                    // Inclusion faults target the full-body fetch, not Beacon's header binding.
                    let inclusion_request = request["method"] != "eth_getBlockByHash"
                        || request["params"][1] == true;
                    let http_failure = {
                        let mut state = shared.lock().unwrap();
                        state.requests.push(request.clone());
                        state
                            .http_failure
                            .filter(|(method, _)| request["method"] == *method && inclusion_request)
                    };
                    if let Some((_, status)) = http_failure {
                        let _ = socket.write_all(format!(
                            "HTTP/1.1 {status} Fixture failure\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        ).as_bytes()).await;
                        return;
                    }
                    let response = {
                        let state = shared.lock().unwrap();
                        if state.unavailable || (state.rpc_failure == request["method"].as_str() && inclusion_request) {
                            json!({"jsonrpc":"2.0", "id":request["id"], "error":{"code":-32000,"message":"fixture unavailable"}})
                        } else {
                            let result = match request["method"].as_str().unwrap() {
                                "eth_chainId" => json!(format!("0x{:x}", state.chain_id)),
                                "eth_getBlockByNumber" => {
                                    let number = if request["params"][0] == "finalized" { state.tip as u64 }
                                    else { u64::from_str_radix(request["params"][0].as_str().unwrap().trim_start_matches("0x"), 16).unwrap() };
                                    if state.missing.contains(&number) { Value::Null }
                                    else { serde_json::to_value(state.chain.get(number as usize)).unwrap() }
                                }
                                "eth_getBlockByHash" => {
                                    let hash: B256 = serde_json::from_value(request["params"][0].clone()).unwrap();
                                    serde_json::to_value(state.chain.iter().find(|block|
                                        block.header.hash == hash && !state.missing.contains(&block.header.number))).unwrap()
                                }
                                "eth_getTransactionReceipt" => serde_json::to_value(&state.receipt).unwrap(),
                                "eth_getBlockReceipts" => serde_json::to_value(&state.receipts).unwrap(),
                                "eth_getTransactionCount" => json!(format!("0x{:x}", state.nonce)),
                                "eth_getProof" => {
                                    let hash: B256 = serde_json::from_value(request["params"][2]["blockHash"].clone()).unwrap();
                                    serde_json::to_value(state.account_proofs.get(&hash)).unwrap()
                                }
                                "eth_call" => {
                                    let input = request["params"][0].get("input").or_else(|| request["params"][0].get("data")).unwrap().as_str().unwrap();
                                    if input.starts_with(&format!("0x{}", alloy::hex::encode(IMessageQueue::isProcessedCall::SELECTOR))) {
                                        json!(format!("0x{:064x}", u8::from(state.processed)))
                                    } else { json!(state.root) }
                                }
                                method => panic!("unexpected fixture method {method}"),
                            };
                            json!({"jsonrpc":"2.0", "id":request["id"], "result":result})
                        }
                    }.to_string();
                    let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await;
                });
            }
        });
        Ok(Self { url, state, server, beacon: tokio::sync::Mutex::new(None) })
    }

    async fn beacon_settings(&self) -> AnyResult<(BeaconFinalityConfig, std::path::PathBuf)> {
        let mut owned = self.beacon.lock().await;
        if owned.is_none() {
            let state = self.state.clone();
            let genesis = state.lock().unwrap().chain[0].header.hash;
            let bootstrap = std::sync::atomic::AtomicBool::new(true);
            *owned = Some(LocalBeaconFixture::new(31337, genesis, Arc::new(move || {
                let state = state.lock().unwrap();
                let height = if bootstrap.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    1
                } else {
                    state.signed_tip
                };
                state.chain[height].header.clone()
            })).await?);
        }
        let beacon = owned.as_ref().unwrap();
        Ok((beacon.config.clone(), beacon.directory.clone()))
    }

    async fn polling(&self, witness: &Self) -> AnyResult<PollingEthApi> {
        let genesis = self.state.lock().unwrap().chain[0].header.hash;
        let (config, directory) = self.beacon_settings().await?;
        PollingEthApi::new(&self.url).await?
            .with_beacon_finality(config, directory).await?
            .with_data_witness(&witness.url, 31337, genesis).await
    }

    async fn api(&self, witness: &Self) -> AnyResult<EthApi> {
        let genesis = self.state.lock().unwrap().chain[0].header.hash;
        let (config, directory) = self.beacon_settings().await?;
        EthApi::new(
            &self.url,
            &format!("{:#x}", Address::repeat_byte(9)),
            Some(&format!("{:#x}", B256::repeat_byte(62))),
            None,
            None,
        )
        .await?
        .with_beacon_finality(config, directory)
        .await?
        .with_data_witness(&witness.url, 31337, genesis)
        .await
    }
}

fn temp_archive(genesis: B256) -> super::tests::Cleanup {
    super::tests::Cleanup {
        genesis,
        directory: std::env::temp_dir().join(format!(
            "quorum-evidence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
    }
}

#[tokio::test]
async fn normal_lag_rechecks_original_pin_and_reconnect_retains_approval() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chain = chain(151, 100);
    let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
    let primary = Fixture::new(chain.clone(), 100).await?;
    let witness = Fixture::new(chain.clone(), 98).await?;
    let api = primary.api(&witness).await?;
    let view = api.verified_finalized_view().await?;
    assert_eq!(view.block_number(), 100);
    assert_eq!(view.checkpoint().schema_version(), 2);
    assert_eq!(view.checkpoint().common_header(), &chain[100].header.inner);
    assert!(view.checkpoint().beacon().is_some());
    assert_eq!(
        api.reconnect().await?.finality_identity()?,
        (31337, chain[0].header.hash)
    );
    let polling = primary.polling(&witness).await?;
    assert_eq!(
        polling.reconnect().await?.finality_identity()?,
        api.finality_identity()?
    );
    assert_eq!(polling.finalized_block().await?.header.number, 100);
    assert!(api
        .is_finalized_block(101, B256::repeat_byte(101).0.into())
        .await
        .is_err());
    assert!(api
        .get_finalized_receipt(B256::repeat_byte(1))
        .await?
        .is_none());
    witness.state.lock().unwrap().chain[100]
        .header
        .inner
        .extra_data = vec![2].into();
    assert!(view.recheck(api.raw_provider()).await.is_err());
    Ok(())
}

#[tokio::test]
async fn wrong_network_outage_and_private_forks_never_qualify_or_replace_fences() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chain = chain(152, 5);
    let cleanup = temp_archive(chain[0].header.hash);
    let primary = Fixture::new(chain.clone(), 4).await?;
    let witness = Fixture::new(chain.clone(), 4).await?;
    let api = primary.api(&witness).await?;
    let view = api
        .verified_finalized_view_with_archive(&cleanup.directory, chain[0].header.hash)
        .await?;
    let checkpoint = view.checkpoint().clone();
    let fence = std::fs::read(cleanup.directory.join("fence.json"))?;
    for fault in [
        "network",
        "genesis",
        "outage",
        "private",
        "malformed",
    ] {
        {
            let mut state = witness.state.lock().unwrap();
            match fault {
                "network" => state.chain_id += 1,
                "genesis" => state.chain[0] = super::finality_tests::chain(153, 0).remove(0),
                "outage" => state.unavailable = true,
                "private" => {
                    state.chain[4].header.inner.extra_data = vec![99].into();
                    state.chain[4].header.hash = state.chain[4].header.inner.hash_slow();
                }
                "malformed" => state.chain[4].header.hash = B256::ZERO,
                _ => unreachable!(),
            }
        }
        assert!(api.verified_finalized_view().await.is_err(), "{fault}");
        assert!(
            api.get_finalized_receipt(B256::ZERO).await.is_err(),
            "{fault}"
        );
        assert_eq!(
            std::fs::read(cleanup.directory.join("fence.json"))?,
            fence,
            "{fault}"
        );
        let mut state = witness.state.lock().unwrap();
        state.chain = chain.clone();
        state.tip = 4;
        state.chain_id = 31337;
        state.unavailable = false;
    }
    // Only authenticated headers bind authority; EL advertised tips are irrelevant.
    primary.state.lock().unwrap().signed_tip = 5;
    let mut fresh = api.verified_finalized_view().await?;
    fresh
        .verify_checkpoint(api.raw_provider(), &checkpoint)
        .await?;
    assert_eq!(fresh.block_number(), 5);
    let mut invalid = checkpoint.beacon().unwrap().clone();
    invalid.schema_version = 1;
    let invalid = FinalityCheckpoint::Beacon(invalid);
    assert!(fresh
        .verify_checkpoint(api.raw_provider(), &invalid)
        .await
        .is_err());
    let legacy = LegacyFinalityCheckpoint {
        schema_version: 1,
        policy: "independent-two-el-v1".into(),
        chain_id: 31337,
        genesis_hash: chain[0].header.hash,
        common_header: chain[2].header.inner.clone(),
        primary_finalized_header: chain[4].header.inner.clone(),
        witness_finalized_header: chain[3].header.inner.clone(),
    };
    let historical = FinalityCheckpoint::Legacy(legacy.clone());
    let original_bytes = serde_json::to_vec(&historical)?;
    fresh.verify_checkpoint(api.raw_provider(), &historical).await?;
    assert_eq!(serde_json::to_vec(&historical)?, original_bytes);
    let (_, directory) = primary.beacon_settings().await?;
    let original_file = std::fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().starts_with("legacy-"))
        .expect("legacy verification must retain its original bytes")
        .path();
    let retained_original: Bytes = serde_json::from_slice(&std::fs::read(&original_file)?)?;
    assert_eq!(retained_original.as_ref(), original_bytes.as_slice());
    let digest = original_file.file_stem().unwrap().to_str().unwrap().strip_prefix("legacy-").unwrap();
    let proof = &fresh.checkpoint().beacon().unwrap().beacon;
    let link = directory.join(format!("requalified-{digest}-{:x}.json", proof.proof_digest));
    let (_, linked): (B256, BeaconCheckpointProof) = serde_json::from_slice(&std::fs::read(link)?)?;
    assert_eq!(&linked, proof);
    for primary_tip in [true, false] {
        let mut forged = legacy.clone();
        let header = if primary_tip { &mut forged.primary_finalized_header } else { &mut forged.witness_finalized_header };
        header.extra_data = vec![0xfa].into();
        assert!(fresh.verify_checkpoint(api.raw_provider(), &FinalityCheckpoint::Legacy(forged)).await.is_err());
    }
    // Freeze the exact retained schema-2 pin while the global cache remains at height 5.
    fresh.pin_to_checkpoint(api.raw_provider(), &checkpoint).await?;
    assert_eq!(fresh.block_number(), 4);
    assert_eq!(fresh.block_hash(), chain[4].header.hash);
    assert_eq!(fresh.checkpoint(), &checkpoint);
    assert!(!fresh.contains(5, chain[5].header.hash));
    fresh.verify_blocks(api.raw_provider(), &BTreeMap::from([(1, chain[1].header.hash)])).await?;
    assert!(fresh.contains(1, chain[1].header.hash));
    assert_eq!(fresh.checkpoint(), &checkpoint);
    assert_eq!(api.verified_finalized_view().await?.block_number(), 5);
    let mut value = serde_json::to_value(&checkpoint)?;
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<FinalityCheckpoint>(value).is_err());
    Ok(())
}

#[tokio::test]
async fn nonce_proofs_requalify_original_headers_and_roots_require_exact_hash_agreement() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let (mut chain, _, raw) = receipt_chain(154, true).await?;
    let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(62))?;
    let (later_root, later_proof) = single_account_proof(signer.address(), 8);
    let mut later = super::finality_tests::chain(154, 2).pop().unwrap();
    later.header.inner.parent_hash = chain[1].header.hash;
    later.header.inner.state_root = later_root;
    later.header.hash = later.header.inner.hash_slow();
    chain.push(later);
    let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
    let primary = Fixture::new(chain.clone(), 1).await?;
    let witness = Fixture::new(chain.clone(), 1).await?;
    primary.state.lock().unwrap().account_proofs.insert(chain[2].header.hash, later_proof);
    let api = primary.api(&witness).await?;
    let saved = api.get_finalized_submission_snapshot(&raw).await?.unwrap();
    assert!(!saved.pinned_nonce_consumed());
    assert_eq!(saved.beacon().unwrap().account_nonce, 7);
    assert_eq!(saved.beacon().unwrap().checkpoint.schema_version(), 2);
    api.verify_finalized_submission_snapshot(&raw, &saved).await?;

    // Unauthenticated count/processed answers cannot change the MPT-proven nonce.
    primary.state.lock().unwrap().nonce = 99;
    witness.state.lock().unwrap().nonce = 6;
    witness.state.lock().unwrap().processed = false;
    assert!(!api.get_finalized_submission_snapshot(&raw).await?.unwrap().pinned_nonce_consumed());
    let original_proof = primary.state.lock().unwrap().account_proofs[&chain[1].header.hash].clone();
    let mut forged = original_proof.clone();
    forged.nonce = 99;
    primary.state.lock().unwrap().account_proofs.insert(chain[1].header.hash, forged);
    assert!(api.get_finalized_submission_snapshot(&raw).await.is_err());
    primary.state.lock().unwrap().account_proofs.remove(&chain[1].header.hash);
    assert!(api.get_finalized_submission_snapshot(&raw).await.is_err());
    primary.state.lock().unwrap().account_proofs.insert(chain[1].header.hash, original_proof);
    let mut forged = saved.beacon().unwrap().clone();
    forged.account_nonce = 99;
    assert!(api.verify_finalized_submission_snapshot(&raw, &FinalizedSubmissionSnapshot::Beacon(forged)).await.is_err());
    let mut forged = saved.beacon().unwrap().clone();
    forged.account_proof.account_proof.clear();
    assert!(api.verify_finalized_submission_snapshot(&raw, &FinalizedSubmissionSnapshot::Beacon(forged)).await.is_err());

    primary.state.lock().unwrap().signed_tip = 2;
    assert!(api.get_finalized_submission_snapshot(&raw).await?.unwrap().pinned_nonce_consumed());
    let restored: FinalizedSubmissionSnapshot = serde_json::from_slice(&serde_json::to_vec(&saved)?)?;
    drop(api);
    let reloaded = primary.api(&witness).await?;
    let api = reloaded.clone();
    let retained = reloaded.verify_finalized_submission_snapshot(&raw, &restored).await?;
    assert_eq!(retained.block_hash(), chain[1].header.hash);
    assert!(!retained.pinned_nonce_consumed());
    assert_eq!(retained.beacon().unwrap().checkpoint, saved.beacon().unwrap().checkpoint);
    for message_processed in [false, true] {
        let legacy: FinalizedSubmissionSnapshot = serde_json::from_value(json!({
            "block_number":1, "block_hash":chain[1].header.hash,
            "pinned_nonce_consumed":false, "message_processed":message_processed,
        }))?;
        let upgraded = reloaded.verify_finalized_submission_snapshot(&raw, &legacy).await?;
        assert!(upgraded.beacon().is_some());
        assert_eq!(upgraded.block_number(), 1);
        assert_eq!(upgraded.block_hash(), chain[1].header.hash);
        assert!(!upgraded.pinned_nonce_consumed());
        assert_eq!(upgraded.beacon().unwrap().account_nonce, 7);
    }
    let legacy = FinalizedSubmissionSnapshot::Legacy(LegacySubmissionSnapshot {
        block_number: 1, block_hash: chain[1].header.hash,
        pinned_nonce_consumed: true, message_processed: true,
    });
    assert!(reloaded.verify_finalized_submission_snapshot(&raw, &legacy).await.is_err());
    assert_eq!(api.read_finalized_merkle_root(1).await?, Some([67; 32]));
    witness.state.lock().unwrap().root = B256::ZERO;
    assert!(api.read_finalized_merkle_root(1).await.is_err());
    assert!(api.read_finalized_merkle_root_at(1, 2, chain[2].header.hash).await.is_err());
    assert!(api.max_block_number().await.is_err());
    assert!(api.max_block_distance().await.is_err());
    assert!(api.process_admin_message_delay().await.is_err());
    assert!(api.process_pauser_message_delay().await.is_err());
    assert!(api.process_user_message_delay().await.is_err());
    for fixture in [&primary, &witness] {
        for request in &fixture.state.lock().unwrap().requests {
            assert_ne!(request["method"], "eth_getTransactionCount");
            if request["method"] == "eth_call" {
                let input = request["params"][0].get("input").or_else(|| request["params"][0].get("data")).unwrap().as_str().unwrap();
                assert!(!input.starts_with(&format!("0x{}", alloy::hex::encode(IMessageQueue::isProcessedCall::SELECTOR))));
                assert_eq!(request["params"][1], json!({"blockHash":chain[2].header.hash, "requireCanonical":true}));
            }
            if request["method"] == "eth_getProof" {
                assert_eq!(request["params"][2]["requireCanonical"], true);
                assert!([json!(chain[1].header.hash), json!(chain[2].header.hash)].contains(&request["params"][2]["blockHash"]));
            }
        }
    }
    Ok(())
}

async fn receipt_chain(
    seed: u8,
    status: bool,
) -> AnyResult<(Vec<Block>, TransactionReceipt, Vec<u8>)> {
    let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(62))?;
    let mut transaction = TxEip1559 {
        chain_id: 31337,
        nonce: 7,
        gas_limit: 100_000,
        max_fee_per_gas: 10,
        max_priority_fee_per_gas: 1,
        to: TxKind::Call(Address::repeat_byte(9)),
        ..Default::default()
    };
    let signature = signer.sign_transaction(&mut transaction).await?;
    let envelope: TxEnvelope = transaction.into_signed(signature).into();
    let raw = envelope.encoded_2718();
    let hash = alloy::primitives::keccak256(&raw);
    let mut chain = chain(seed, 1);
    let consensus = alloy::consensus::ReceiptEnvelope::Eip1559(
        alloy::consensus::Receipt::<alloy::primitives::Log> {
            status: status.into(), cumulative_gas_used: 21_000,
            logs: if status { vec![serde_json::from_value(json!({
                "address":Address::repeat_byte(9), "topics":[B256::repeat_byte(42)], "data":"0x0102"
            }))?] } else { vec![] },
        }.with_bloom(),
    );
    chain[1].header.inner.transactions_root =
        calculate_transaction_root(std::slice::from_ref(&envelope));
    chain[1].header.inner.receipts_root = calculate_receipt_root(std::slice::from_ref(&consensus));
    chain[1].header.inner.state_root = single_account_proof(signer.address(), 7).0;
    chain[1].header.hash = chain[1].header.inner.hash_slow();
    let mut value = serde_json::to_value(consensus)?;
    let coordinates = json!({
        "transactionHash":hash, "transactionIndex":"0x0", "blockNumber":"0x1",
        "blockHash":chain[1].header.hash,
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(coordinates.as_object().unwrap().clone());
    value.as_object_mut().unwrap().extend(
        json!({
            "gasUsed":"0x5208", "effectiveGasPrice":"0x1", "from":signer.address(),
            "to":Address::repeat_byte(9), "contractAddress":null,
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    for (index, log) in value["logs"].as_array_mut().unwrap().iter_mut().enumerate() {
        log.as_object_mut()
            .unwrap()
            .extend(coordinates.as_object().unwrap().clone());
        log["logIndex"] = json!(format!("0x{index:x}"));
        log["removed"] = json!(false);
    }
    let receipt = serde_json::from_value(value)?;
    let mut transaction = serde_json::to_value(envelope)?;
    transaction["hash"] = json!(hash);
    transaction["from"] = json!(signer.address());
    transaction["blockNumber"] = json!("0x1");
    transaction["blockHash"] = json!(chain[1].header.hash);
    transaction["transactionIndex"] = json!("0x0");
    chain[1].transactions = BlockTransactions::Full(vec![serde_json::from_value(transaction)?]);
    Ok((chain, receipt, raw))
}

#[tokio::test]
async fn original_receipts_require_both_tries_signed_identity_and_exact_locator() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    for (seed, status) in [(155, true), (156, false)] {
        let (chain, receipt, raw) = receipt_chain(seed, status).await?;
        let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
        let primary = Fixture::new(chain.clone(), 1).await?;
        let witness = Fixture::new(chain.clone(), 1).await?;
        {
            let mut state = primary.state.lock().unwrap();
            state.receipt = Some(receipt.clone());
            state.receipts = vec![receipt.clone()];
        }
        let api = primary.api(&witness).await?;
        let evidence = api
            .get_finalized_receipt(receipt.transaction_hash)
            .await?
            .unwrap();
        assert_eq!(evidence.inclusion.raw_transaction, raw);
        assert_eq!(evidence.inclusion.receipt.status(), status);
        let mut view = api.verified_finalized_view().await?;
        view.verify_checkpoint(api.raw_provider(), &evidence.checkpoint)
            .await?;
        evidence
            .inclusion
            .verify_original_inclusion(&view, &raw, receipt.transaction_hash)?;
        let restored: inclusion::RecordedReceiptProof =
            serde_json::from_slice(&serde_json::to_vec(&evidence.inclusion)?)?;
        restored.verify_original_inclusion(&view, &raw, receipt.transaction_hash)?;
        assert!(restored
            .verify_original_inclusion(&view, &[1], receipt.transaction_hash)
            .is_err());
        assert!(restored
            .verify_original_inclusion(&view, &raw, B256::repeat_byte(99))
            .is_err());
        for transaction in [true, false] {
            let mut tampered = restored.clone();
            if transaction {
                tampered.transaction_proof.clear();
            } else {
                tampered.receipt_proof.clear();
            }
            assert!(tampered
                .verify_inclusion(&view, receipt.transaction_hash)
                .is_err());
            assert!(tampered
                .verify_original_inclusion(&view, &raw, receipt.transaction_hash)
                .is_err());
        }
        let mut trailing = restored.clone();
        trailing.raw_transaction.push(0);
        assert!(validate_receipt_locator(&receipt, &trailing, receipt.transaction_hash).is_err());
        assert!(
            matches!(
                api.get_tx_status(receipt.transaction_hash).await?,
                TxStatus::Finalized
            ) == status
        );
        for field in [
            "status",
            "transactionHash",
            "transactionIndex",
            "from",
            "to",
            "logsBloom",
        ] {
            let mut value = serde_json::to_value(&receipt)?;
            value[field] = match field {
                "status" => json!(if status { "0x0" } else { "0x1" }),
                "transactionHash" => json!(B256::repeat_byte(99)),
                "transactionIndex" => json!("0x1"),
                "from" | "to" => json!(Address::repeat_byte(98)),
                "logsBloom" => json!(format!("0x{}", "ff".repeat(256))),
                _ => unreachable!(),
            };
            primary.state.lock().unwrap().receipt = Some(serde_json::from_value(value)?);
            assert!(
                api.get_finalized_receipt(receipt.transaction_hash)
                    .await
                    .is_err(),
                "{field}"
            );
        }
        if status {
            for (field, bad) in [
                ("removed", json!(true)),
                ("logIndex", json!("0x1")),
                ("transactionIndex", json!("0x1")),
                ("blockNumber", json!("0x0")),
                ("blockHash", json!(B256::ZERO)),
                ("transactionHash", json!(B256::ZERO)),
                ("data", json!("0x03")),
                ("topics", json!([B256::ZERO])),
            ] {
                let mut changed = serde_json::to_value(&receipt)?;
                changed["logs"][0][field] = bad;
                primary.state.lock().unwrap().receipt = Some(serde_json::from_value(changed)?);
                assert!(
                    api.get_tx_status(receipt.transaction_hash).await.is_err(),
                    "{field}"
                );
            }
        }
        let mut changed = serde_json::to_value(&receipt)?;
        changed["status"] = json!(if status { "0x0" } else { "0x1" });
        primary.state.lock().unwrap().receipts = vec![serde_json::from_value(changed)?];
        assert!(api.get_tx_status(receipt.transaction_hash).await.is_err());
        primary.state.lock().unwrap().receipt = Some(receipt.clone());
        primary.state.lock().unwrap().receipts.clear();
        assert!(api.get_tx_status(receipt.transaction_hash).await.is_err());
        primary.state.lock().unwrap().receipts = vec![receipt.clone()];
        primary.state.lock().unwrap().chain[1].transactions = BlockTransactions::Full(vec![]);
        assert!(api.get_tx_status(receipt.transaction_hash).await.is_err());
    }
    Ok(())
}

#[tokio::test]
async fn legacy_archives_all_requalify_before_checkpoint_promotion_and_restart() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chain = chain(157, 8);
    let cleanup = temp_archive(chain[0].header.hash);
    let genesis = chain[0].header.hash;
    let first = cleanup.directory.join("first");
    let second = cleanup.directory.join("second");
    finality_archive::HeaderArchive::open(&first)?.witness(
        genesis,
        &chain[2].header.inner,
        None,
    )?;
    finality_archive::HeaderArchive::open(&second)?.witness(
        genesis,
        &chain[4].header.inner,
        None,
    )?;
    register_archive(&first, genesis)?;
    register_archive(&second, genesis)?;
    register_archive(&first.join("."), genesis)?;
    assert_eq!(archives_for(genesis).len(), 2);
    assert!(register_archive(&first, B256::ZERO).is_err());
    let primary = Fixture::new(chain.clone(), 6).await?;
    let witness = Fixture::new(chain.clone(), 6).await?;
    let api = primary.api(&witness).await?;
    api.enable_finality_archive(&first, genesis).await?;
    api.enable_finality_archive(&second, genesis).await?;
    api.verified_finalized_view().await?;
    for directory in [&first, &second] {
        let fence: Value = serde_json::from_slice(&std::fs::read(directory.join("fence.json"))?)?;
        assert_eq!(fence["quorum_checkpoint"]["schema_version"], 2);
        assert_eq!(fence["tips"].as_array().unwrap().len(), 1);
    }
    let old = std::fs::read(second.join("fence.json"))?;
    drop(api);
    FINALIZED_ANCESTRY
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .remove(&genesis);
    FINALITY_ARCHIVES
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .remove(&genesis);
    register_archive(&first, genesis)?;
    register_archive(&second, genesis)?;
    let api = primary.api(&witness).await?;
    api.enable_finality_archive(&first, genesis).await?;
    api.enable_finality_archive(&second, genesis).await?;
    primary.state.lock().unwrap().signed_tip = 5;
    // A stale signed update cannot roll back the retained authenticated head.
    assert_eq!(api.verified_finalized_view().await?.block_number(), 6);
    assert_eq!(std::fs::read(second.join("fence.json"))?, old);
    primary.state.lock().unwrap().signed_tip = 8;
    primary.state.lock().unwrap().missing.insert(7);
    assert!(api.verified_finalized_view().await.is_err());
    let partial: Value = serde_json::from_slice(&std::fs::read(first.join("fence.json"))?)?;
    assert_eq!(partial["tips"].as_array().unwrap().len(), 2);
    primary.state.lock().unwrap().missing.clear();
    api.verified_finalized_view().await?;
    std::fs::write(second.join("fence.json"), b"corrupt")?;
    assert!(api.verified_finalized_view().await.is_err());
    assert_eq!(std::fs::read(second.join("fence.json"))?, b"corrupt");
    Ok(())
}

#[tokio::test]
async fn independent_beacon_owners_never_mutate_or_trust_peer_header_fences() -> AnyResult<()> {
    fn retained_bytes(directory: &std::path::Path) -> AnyResult<BTreeMap<std::path::PathBuf, Vec<u8>>> {
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                files.extend(retained_bytes(&path)?);
            } else {
                files.insert(path.clone(), std::fs::read(path)?);
            }
        }
        Ok(files)
    }

    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chain = chain(173, 8);
    let genesis = chain[0].header.hash;
    let cleanup = temp_archive(genesis);
    let primary = Fixture::new(chain.clone(), 6).await?;
    let witness = Fixture::new(chain.clone(), 6).await?;
    let original = primary.polling(&witness).await?;
    let checkpoint = original.verified_finalized_view().await?.checkpoint().clone();
    let (config, original_directory) = primary.beacon_settings().await?;
    let before = retained_bytes(&original_directory)?;
    let peer_directory = cleanup.directory.join("peer");
    let peer_config = config.with_proof_sources(vec![original_directory.clone()])?;
    let peer = PollingEthApi::new(&primary.url).await?
        .with_beacon_finality(peer_config.clone(), &peer_directory).await?;
    assert!(peer.enable_finality_archive(&original_directory.join("headers"), genesis).await.is_err());
    let mut peer_view = peer.verified_finalized_view().await?;
    peer_view.verify_checkpoint(&*peer, &checkpoint).await?;
    peer_view.verify_blocks(&*peer, &BTreeMap::from([(2, chain[2].header.hash)])).await?;
    assert_eq!(retained_bytes(&original_directory)?, before);
    drop(peer_view);
    drop(peer);
    let peer = PollingEthApi::new(&primary.url).await?
        .with_beacon_finality(peer_config, &peer_directory).await?;
    peer.verified_finalized_view().await?.verify_checkpoint(&*peer, &checkpoint).await?;
    assert_eq!(retained_bytes(&original_directory)?, before);
    std::fs::write(peer_directory.join("headers/fence.json"), b"corrupt peer fence")?;
    assert_eq!(original.verified_finalized_view().await?.checkpoint(), &checkpoint);
    assert_eq!(std::fs::read(peer_directory.join("headers/fence.json"))?, b"corrupt peer fence");
    Ok(())
}

#[tokio::test]
async fn durable_history_has_no_age_cutoff_and_preserves_corrupt_hints() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let height = 131_850u64;
    let chain = chain(158, height);
    let genesis = chain[0].header.hash;
    let cleanup = temp_archive(genesis);
    register_archive(&cleanup.directory, genesis)?;
    let archives = archives_for(genesis);
    for chunk in chain[1..].rchunks(512) {
        archives[0].save(
            &chunk
                .iter()
                .rev()
                .map(|block| block.header.inner.clone())
                .collect::<Vec<_>>(),
        )?;
    }
    let primary = Fixture::new(chain.clone(), height as usize).await?;
    let witness = Fixture::new(chain.clone(), height as usize).await?;
    let api = primary.api(&witness).await?;
    api.enable_finality_archive(&cleanup.directory, genesis).await?;
    let mut view = api.verified_finalized_view().await?;
    let targets = (0..8_430)
        .map(|index| (index * 15, chain[(index * 15) as usize].header.hash))
        .collect();
    view.verify_blocks(api.raw_provider(), &targets).await?;
    assert!(view.contains(0, genesis));
    let damaged = std::fs::read_dir(&cleanup.directory)?
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("headers-{height}-"))
        })
        .unwrap()
        .path();
    std::fs::write(&damaged, b"corrupt")?;
    let mut fresh = api.verified_finalized_view().await?;
    fresh
        .verify_blocks(
            api.raw_provider(),
            &BTreeMap::from([(height - 512, chain[(height - 512) as usize].header.hash)]),
        )
        .await?;
    assert_eq!(std::fs::read(damaged)?, b"corrupt");
    Ok(())
}

#[tokio::test]
async fn all_unconfigured_finality_boundaries_preserve_the_typed_hold() -> AnyResult<()> {
    let (chain, _, raw) = receipt_chain(159, true).await?;
    let fixture = Fixture::new(chain.clone(), 1).await?;
    let api = EthApi::new(
        &fixture.url,
        &format!("{:#x}", Address::repeat_byte(9)),
        Some(&format!("{:#x}", B256::repeat_byte(62))),
        None,
        None,
    )
    .await?;
    let polling = PollingEthApi::new(&fixture.url).await?;
    assert!(api
        .clone()
        .with_data_witness(
            &format!("{}/#same", fixture.url),
            31337,
            chain[0].header.hash
        )
        .await
        .is_err());
    assert!(api
        .clone()
        .with_data_witness("http://127.0.0.1:1", 1, chain[0].header.hash)
        .await
        .is_err());
    for result in [
        polling.get_finalized_receipt(B256::ZERO).await.map(|_| ()),
        polling.finalized_block().await.map(|_| ()),
        polling
            .is_finalized_block(0, chain[0].header.hash.0.into())
            .await
            .map(|_| ()),
        api.finalized_block_number().await.map(|_| ()),
        api.verify_finalized_submission_snapshot(
            &raw,
            &FinalizedSubmissionSnapshot::Legacy(LegacySubmissionSnapshot {
                block_number: 1,
                block_hash: chain[1].header.hash,
                pinned_nonce_consumed: false,
                message_processed: false,
            }),
        )
        .await.map(|_| ()),
    ] {
        assert!(matches!(
            result.unwrap_err().downcast_ref::<Error>(),
            Some(Error::BeaconFinalityNotConfigured)
        ));
    }
    for error in [
        api.get_finalized_receipt(B256::ZERO).await.unwrap_err(),
        api.get_tx_status(B256::ZERO).await.unwrap_err(),
        api.verified_finalized_view().await.unwrap_err(),
        api.get_finalized_submission_snapshot(&raw)
            .await
            .unwrap_err(),
        api.read_finalized_merkle_root(0).await.unwrap_err(),
        api.read_finalized_merkle_root_at(0, 0, B256::ZERO)
            .await
            .unwrap_err(),
        api.max_block_number().await.unwrap_err(),
        api.max_block_distance().await.unwrap_err(),
        api.process_admin_message_delay().await.unwrap_err(),
        api.process_pauser_message_delay().await.unwrap_err(),
        api.process_user_message_delay().await.unwrap_err(),
    ] {
        assert!(matches!(error, Error::BeaconFinalityNotConfigured));
    }
    Ok(())
}

#[tokio::test]
async fn inconsistent_tip_parent_links_and_receipts_above_common_remain_held() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let mut chain = chain(160, 3);
    let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
    let primary = Fixture::new(chain.clone(), 3).await?;
    primary.state.lock().unwrap().signed_tip = 1;
    let witness = Fixture::new(chain.clone(), 1).await?;
    let api = primary.api(&witness).await?;
    let (_, mut receipt, _) = receipt_chain(161, true).await?;
    receipt.block_number = Some(2);
    receipt.block_hash = Some(chain[2].header.hash);
    primary.state.lock().unwrap().receipt = Some(receipt.clone());
    assert!(api
        .get_finalized_receipt(receipt.transaction_hash)
        .await?
        .is_none());
    chain[3].header.inner.parent_hash = B256::repeat_byte(222);
    chain[3].header.hash = chain[3].header.inner.hash_slow();
    primary.state.lock().unwrap().chain = chain;
    primary.state.lock().unwrap().signed_tip = 3;
    assert!(api.verified_finalized_view().await.is_err());
    assert!(api.get_tx_status(receipt.transaction_hash).await.is_err());
    Ok(())
}

#[tokio::test]
async fn historical_verification_has_no_view_lifetime_or_aggregate_age_cutoff() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    for (seed, archived) in [(173, false), (174, true)] {
        let chain = chain(seed, 4);
        let cleanup = temp_archive(chain[0].header.hash);
        let primary = Fixture::new(chain.clone(), 4).await?;
        let witness = Fixture::new(chain.clone(), 4).await?;
        let api = primary.api(&witness).await?;
        let mut view = if archived {
            api.verified_finalized_view_with_archive(&cleanup.directory, chain[0].header.hash)
                .await?
        } else {
            api.verified_finalized_view().await?
        };
        let checkpoint = view.checkpoint().clone();
        let target = BTreeMap::from([(2, chain[2].header.hash)]);
        view.verify_blocks(api.raw_provider(), &target).await?;

        // Simulate time spent processing other journal records, without aging the pin.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(301)).await;
        tokio::time::resume();
        view.verify_blocks(api.raw_provider(), &target).await?;
        view.verify_blocks(
            api.raw_provider(),
            &BTreeMap::from([(0, chain[0].header.hash)]),
        )
        .await?;
        assert!(view.contains(0, chain[0].header.hash));
        assert_eq!(view.checkpoint(), &checkpoint);

        witness.state.lock().unwrap().chain[4]
            .header
            .inner
            .extra_data = vec![255].into();
        assert!(view
            .verify_blocks(api.raw_provider(), &target)
            .await
            .is_err());
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn historical_header_requests_keep_individual_timeouts() -> AnyResult<()> {
    use alloy::transports::mock::{Asserter, MockTransport};
    let chain = chain(162, 1);
    let replies = Asserter::new();
    replies.push_success(&chain[1]);
    let client = alloy::rpc::client::RpcClient::builder()
        .layer(tower::util::MapFutureLayer::new(
            |future| -> alloy::transports::TransportFut<'static> {
                Box::pin(async move {
                    let response = future.await;
                    tokio::time::sleep(Duration::from_secs(11)).await;
                    response
                })
            },
        ))
        .transport(MockTransport::new(replies), true);
    let provider = RootProvider::builder().connect_client(client);
    assert!(matches!(
        qualified_header(&provider, BlockNumberOrTag::Number(1)).await,
        Err(Error::FinalizedAncestryPending)
    ));
    Ok(())
}

#[tokio::test]
async fn unfinished_common_generations_survive_cache_capacity_pressure() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chains: Vec<_> = (163..=171).map(|seed| chain(seed, 3)).collect();
    let _owned =
        super::tests::ClearOwnedChains::<9>(std::array::from_fn(|i| chains[i][0].header.hash));
    let mut fenced = Vec::new();
    for chain in &chains[..8] {
        let primary = Fixture::new(chain.clone(), 1).await?;
        let witness = Fixture::new(chain.clone(), 1).await?;
        let api = primary.api(&witness).await?;
        api.verified_finalized_view().await?;
        primary.state.lock().unwrap().signed_tip = 3;
        primary.state.lock().unwrap().missing.insert(2);
        assert!(api.verified_finalized_view().await.is_err());
        fenced.push((primary, witness, api));
    }
    let primary = Fixture::new(chains[8].clone(), 1).await?;
    let witness = Fixture::new(chains[8].clone(), 1).await?;
    let newcomer = primary.api(&witness).await?;
    assert!(matches!(
        newcomer.verified_finalized_view().await,
        Err(Error::FinalizedAncestryPending)
    ));
    for (_, _, api) in &fenced {
        assert!(matches!(
            api.verified_finalized_view().await,
            Err(Error::FinalizedAncestryPending)
        ));
    }
    fenced[0].0.state.lock().unwrap().missing.clear();
    fenced[0].2.verified_finalized_view().await?;
    newcomer.verified_finalized_view().await?;
    Ok(())
}

#[tokio::test]
async fn every_unfinished_common_generation_remains_a_fence() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chain = chain(172, 6);
    let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
    let primary = Fixture::new(chain.clone(), 2).await?;
    let witness = Fixture::new(chain.clone(), 2).await?;
    let api = primary.api(&witness).await?;
    api.verified_finalized_view().await?;
    primary.state.lock().unwrap().signed_tip = 4;
    primary.state.lock().unwrap().missing.insert(3);
    assert!(api.verified_finalized_view().await.is_err());
    let mut fork = chain.clone();
    fork[4].header.inner.extra_data = vec![0xaa].into();
    fork[4].header.hash = fork[4].header.inner.hash_slow();
    for number in 5..=6 {
        fork[number].header.inner.parent_hash = fork[number - 1].header.hash;
        fork[number].header.hash = fork[number].header.inner.hash_slow();
    }
    for fixture in [&primary, &witness] {
        let mut state = fixture.state.lock().unwrap();
        state.chain = fork.clone();
        state.tip = 6;
        state.signed_tip = 6;
    }
    primary.state.lock().unwrap().missing.insert(5);
    assert!(api.verified_finalized_view().await.is_err());
    primary.state.lock().unwrap().missing.clear();
    assert!(matches!(
        api.verified_finalized_view().await,
        Err(Error::FinalizedAncestryConflict)
    ));
    Ok(())
}

#[tokio::test]
async fn signed_beacon_head_ignores_arbitrary_el_finalized_tag_skew() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chain = chain(180, 2);
    let cleanup = temp_archive(chain[0].header.hash);
    let primary = Fixture::new(chain.clone(), 1).await?;
    let witness = Fixture::new(chain.clone(), 1).await?;
    let api = primary.api(&witness).await?;
    let accepted = api.verified_finalized_view_with_archive(&cleanup.directory, chain[0].header.hash).await?;
    let checkpoint = accepted.checkpoint().clone();
    for (primary_tip, witness_tip) in [(0, 100_006), (100_006, 0)] {
        primary.state.lock().unwrap().tip = primary_tip;
        witness.state.lock().unwrap().tip = witness_tip;
        assert_eq!(api.verified_finalized_view().await?.checkpoint(), &checkpoint);
    }
    primary.state.lock().unwrap().signed_tip = 2;
    assert_eq!(api.verified_finalized_view().await?.block_number(), 2);
    for fixture in [&primary, &witness] {
        assert!(fixture.state.lock().unwrap().requests.iter().all(|request|
            request["method"] != "eth_getBlockByNumber" || request["params"][0] != "finalized"));
    }
    Ok(())
}

#[tokio::test]
async fn agreeing_execution_tags_cannot_replace_missing_beacon_authority() -> AnyResult<()> {
    let chain = chain(182, 1);
    let primary = Fixture::new(chain.clone(), 1).await?;
    let witness = Fixture::new(chain.clone(), 1).await?;
    let api = PollingEthApi::new(&primary.url).await?
        .with_data_witness(&witness.url, 31337, chain[0].header.hash).await?;
    assert!(api.raw_witness_provider().is_ok());
    assert!(matches!(api.verified_finalized_view().await, Err(Error::BeaconFinalityNotConfigured)));
    assert!(api.finality_identity().is_err());
    for fixture in [&primary, &witness] {
        assert!(fixture.state.lock().unwrap().requests.iter().all(|request|
            request["method"] != "eth_getBlockByNumber" || request["params"][0] != "finalized"));
    }
    Ok(())
}

#[tokio::test]
async fn inclusion_fetch_failures_preserve_typed_transport_and_retry_classification(
) -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let (chain, receipt, raw) = receipt_chain(183, true).await?;
    let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
    let primary = Fixture::new(chain.clone(), 1).await?;
    let witness = Fixture::new(chain.clone(), 1).await?;
    {
        let mut state = primary.state.lock().unwrap();
        state.receipt = Some(receipt.clone());
        state.receipts = vec![receipt.clone()];
    }
    let api = primary.api(&witness).await?;
    let polling = primary.polling(&witness).await?;
    for method in ["eth_getBlockByHash", "eth_getBlockReceipts"] {
        for status in [503, 401] {
            primary.state.lock().unwrap().http_failure = Some((method, status));
            let error = api
                .get_finalized_receipt(receipt.transaction_hash)
                .await
                .unwrap_err();
            assert!(
                matches!(&error, Error::ErrorInHTTPTransport(_)),
                "{method} HTTP {status}: {error:?}"
            );
            assert_eq!(error::is_transient_rpc_cause(&error), status == 503);
            let error = polling
                .get_finalized_receipt(receipt.transaction_hash)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<Error>(),
                    Some(Error::ErrorInHTTPTransport(_))
                ),
                "{error:?}"
            );
            assert_eq!(error::is_transient_rpc_error(&error), status == 503);
        }
        primary.state.lock().unwrap().http_failure = None;
        primary.state.lock().unwrap().rpc_failure = Some(method);
        let error = api
            .get_finalized_receipt(receipt.transaction_hash)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::ErrorInHTTPTransport(RpcError::ErrorResp(_))),
            "{error:?}"
        );
        assert!(
            !error::is_transient_rpc_cause(&error),
            "JSON-RPC rejection is not a network outage"
        );
        primary.state.lock().unwrap().rpc_failure = None;
    }
    let authenticated = api
        .get_finalized_receipt(receipt.transaction_hash)
        .await?
        .unwrap();
    assert_eq!(authenticated.inclusion.raw_transaction, raw);
    assert!(authenticated.inclusion.receipt.status());
    let mut changed = serde_json::to_value(&receipt)?;
    changed["status"] = json!("0x0");
    primary.state.lock().unwrap().receipts = vec![serde_json::from_value(changed)?];
    let error = api
        .get_finalized_receipt(receipt.transaction_hash)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, Error::InvalidFinalityEvidence(_)),
        "{error:?}"
    );
    assert!(!error::is_transient_rpc_cause(&error));
    assert!(api.get_tx_status(receipt.transaction_hash).await.is_err());
    Ok(())
}

#[tokio::test]
async fn data_witness_endpoint_policy_rejects_insecure_and_canonical_duplicates_without_rpc(
) -> AnyResult<()> {
    let chain = chain(184, 1);
    let fixture = Fixture::new(chain.clone(), 1).await?;
    let api = EthApi::new(
        &fixture.url,
        &format!("{:#x}", Address::repeat_byte(9)),
        None,
        None,
        None,
    )
    .await?;
    let polling = PollingEthApi::new(&fixture.url).await?;
    fixture.state.lock().unwrap().requests.clear();
    let port = Url::parse(&fixture.url)?.port().unwrap();
    let pairs = [
        (
            "http://primary.example/rpc".to_owned(),
            "https://witness.example/rpc".to_owned(),
        ),
        (
            "ws://primary.example/rpc".to_owned(),
            "https://witness.example/rpc".to_owned(),
        ),
        (
            "https://primary.example/rpc".to_owned(),
            "http://witness.example/rpc".to_owned(),
        ),
        (
            "https://primary.example/rpc".to_owned(),
            "http://192.168.1.1/rpc".to_owned(),
        ),
        (
            "https://primary.example/rpc".to_owned(),
            "http://localhost.example/rpc".to_owned(),
        ),
        (
            "https://RPC.example/rpc".to_owned(),
            "https://rpc.example:443/rpc/".to_owned(),
        ),
        (
            "https://rpc.example./rpc///".to_owned(),
            "https://rpc.example/rpc".to_owned(),
        ),
        (
            "https://alice:secret@rpc.example/rpc#one".to_owned(),
            "https://bob:other@rpc.example/rpc/#two".to_owned(),
        ),
        (
            "wss://rpc.example/rpc".to_owned(),
            "https://rpc.example:443/rpc/".to_owned(),
        ),
        (
            "ws://localhost/rpc".to_owned(),
            "http://127.0.0.1:80/rpc/".to_owned(),
        ),
        (fixture.url.clone(), format!("http://localhost:{port}/")),
        (fixture.url.clone(), format!("http://127.1.2.3:{port}/")),
        (fixture.url.clone(), format!("http://[::1]:{port}/")),
        (fixture.url.clone(), format!("http://LOCALHOST.:{port}/")),
    ];
    for (primary, witness) in pairs {
        // The transport is already owned; exercise public witness configuration
        // with each configured URL without contacting external test domains.
        let mut signer = api.clone();
        signer.url = Url::parse(&primary)?;
        let mut reader = polling.clone();
        reader.url = Url::parse(&primary)?;
        assert!(
            signer
                .with_data_witness(&witness, 31337, chain[0].header.hash)
                .await
                .is_err(),
            "signer accepted {primary} + {witness}"
        );
        assert!(
            reader
                .with_data_witness(&witness, 31337, chain[0].header.hash)
                .await
                .is_err(),
            "polling accepted {primary} + {witness}"
        );
    }
    assert!(fixture.state.lock().unwrap().requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn data_witness_endpoint_policy_accepts_secure_and_distinct_loopback_configuration(
) -> AnyResult<()> {
    let chain = chain(185, 1);
    let primary = Fixture::new(chain.clone(), 1).await?;
    let witness = Fixture::new(chain.clone(), 1).await?;
    let api = EthApi::new(&primary.url, &format!("{:#x}", Address::repeat_byte(9)), None, None, None).await?;
    let polling = PollingEthApi::new(&primary.url).await?;
    for (primary_url, witness_url) in [
        ("https://primary.example/rpc", "https://witness.example/rpc"),
        ("wss://primary.example/rpc", "https://witness.example/rpc"),
        ("https://rpc.example/one", "https://rpc.example/two"),
        (
            "https://rpc.example/rpc?tenant=one",
            "https://rpc.example/rpc?tenant=two",
        ),
        (primary.url.as_str(), witness.url.as_str()),
        ("http://[::1]:12340", "http://[::1]:12341"),
    ] {
        let mut signer = api.clone();
        signer.url = Url::parse(primary_url)?;
        let mut reader = polling.clone();
        reader.url = Url::parse(primary_url)?;
        let signer = signer.with_data_witness(witness_url, 31337, chain[0].header.hash).await?;
        let reader = reader.with_data_witness(witness_url, 31337, chain[0].header.hash).await?;
        assert!(signer.raw_witness_provider().is_ok());
        assert!(reader.raw_witness_provider().is_ok());
        assert!(signer.finality_identity().is_err());
        assert!(reader.finality_identity().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn data_witness_endpoint_policy_rejects_websocket_configuration_before_connecting(
) -> AnyResult<()> {
    let chain = chain(186, 1);
    let fixture = Fixture::new(chain.clone(), 1).await?;
    let api = EthApi::new(
        &fixture.url,
        &format!("{:#x}", Address::repeat_byte(9)),
        None,
        None,
        None,
    )
    .await?;
    let polling = PollingEthApi::new(&fixture.url).await?;
    let trap = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = trap.local_addr()?.port();
    for (primary, witness) in [
        (
            format!("http://127.0.0.1:{port}/rpc"),
            format!("ws://localhost:{port}/rpc/"),
        ),
        (
            format!("https://127.0.0.1:{port}/rpc"),
            format!("wss://127.0.0.1:{port}/rpc/"),
        ),
        (
            "https://primary.example/rpc".into(),
            format!("ws://0.0.0.0:{port}/rpc"),
        ),
    ] {
        let mut signer = api.clone();
        signer.url = Url::parse(&primary)?;
        let mut reader = polling.clone();
        reader.url = Url::parse(&primary)?;
        let configure = async {
            for result in [
                signer
                    .with_data_witness(&witness, 31337, chain[0].header.hash)
                    .await
                    .map(|_| ()),
                reader
                    .with_data_witness(&witness, 31337, chain[0].header.hash)
                    .await
                    .map(|_| ()),
            ] {
                let error = result.expect_err("unsafe websocket configuration accepted");
                assert!(
                    error.to_string().starts_with("HOLD:"),
                    "transport failure is not configuration rejection: {error:?}"
                );
            }
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::select! {
                _ = configure => {},
                _ = trap.accept() => panic!("rejected configuration opened a websocket connection"),
            }
        })
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn finality_endpoint_policy_rejects_insecure_primary_before_initial_rpc() -> AnyResult<()> {
    let trap = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = trap.local_addr()?.port();
    let queue = format!("{:#x}", Address::repeat_byte(9));
    for scheme in ["http", "ws"] {
        let url = format!("{scheme}://0.0.0.0:{port}");
        let configure = async {
            let error = PollingEthApi::new(&url)
                .await
                .err()
                .expect("insecure polling endpoint accepted");
            assert!(matches!(
                error.downcast_ref::<Error>(),
                Some(Error::WrongNodeUrl)
            ));
            let error = EthApi::new(&url, &queue, None, None, None)
                .await
                .err()
                .expect("insecure signer endpoint accepted");
            assert!(matches!(error, Error::WrongNodeUrl));
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::select! {
                _ = configure => {},
                _ = trap.accept() => panic!("insecure primary contacted before endpoint validation"),
            }
        }).await?;
    }
    Ok(())
}
