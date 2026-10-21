//! Independent transports and real signed transaction/receipt roots; no proof bypasses.
use super::*;
use alloy::{
    consensus::{proofs::{calculate_receipt_root, calculate_transaction_root}, SignableTransaction, TxEip1559, TxEnvelope},
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
    chain_id: u64,
    missing: BTreeSet<u64>,
    unavailable: bool,
    receipt: Option<TransactionReceipt>,
    receipts: Vec<TransactionReceipt>,
    nonce: u64,
    processed: bool,
    root: B256,
    requests: Vec<Value>,
}

struct Fixture {
    url: String,
    state: Arc<std::sync::Mutex<State>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) { self.server.abort(); }
}
impl Fixture {
    async fn new(chain: Vec<Block>, tip: usize) -> AnyResult<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(std::sync::Mutex::new(State {
            chain, tip, chain_id: 560048, missing: BTreeSet::new(), unavailable: false,
            receipt: None, receipts: vec![], nonce: 7, processed: true,
            root: B256::repeat_byte(67), requests: vec![],
        }));
        let shared = state.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let shared = shared.clone();
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let request = loop {
                        let mut chunk = [0; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        if count == 0 { return; }
                        buffer.extend_from_slice(&chunk[..count]);
                        let Some(end) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") else { continue };
                        let headers = std::str::from_utf8(&buffer[..end]).unwrap();
                        let length: usize = headers.lines().find_map(|line| line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse().unwrap())).unwrap();
                        if buffer.len() >= end + 4 + length {
                            break serde_json::from_slice::<Value>(&buffer[end + 4..end + 4 + length]).unwrap();
                        }
                    };
                    let response = {
                        let mut state = shared.lock().unwrap();
                        state.requests.push(request.clone());
                        if state.unavailable {
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
        Ok(Self { url, state, server })
    }

    async fn api(&self, witness: &Self) -> AnyResult<EthApi> {
        let genesis = self.state.lock().unwrap().chain[0].header.hash;
        Ok(EthApi::new(&self.url, &format!("{:#x}", Address::repeat_byte(9)), None, None, None).await?
            .with_finality_witness(&witness.url, 560048, genesis).await?)
    }
}

fn chain(seed: u8, height: u64) -> Vec<Block> {
    let mut parent = B256::ZERO;
    (0..=height).map(|number| {
        let mut block: Block = Block::default();
        block.header.inner.number = number;
        block.header.inner.parent_hash = parent;
        block.header.inner.extra_data = Bytes::from(vec![seed]);
        block.header.hash = block.header.inner.hash_slow();
        parent = block.header.hash;
        block
    }).collect()
}

fn temp_archive(genesis: B256) -> super::tests::Cleanup {
    super::tests::Cleanup {
        genesis,
        directory: std::env::temp_dir().join(format!("quorum-evidence-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())),
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
    assert_eq!(view.block_number(), 98);
    assert_eq!(view.checkpoint().primary_finalized_header.number, 100);
    assert_eq!(view.checkpoint().witness_finalized_header.number, 98);
    assert_eq!(api.reconnect().await?.finality_identity()?, (560048, chain[0].header.hash));
    let polling = PollingEthApi::new(&primary.url).await?
        .with_finality_witness(&witness.url, 560048, chain[0].header.hash).await?;
    assert_eq!(polling.reconnect().await?.finality_identity()?, api.finality_identity()?);
    assert_eq!(polling.finalized_block().await?.header.number, 98);
    assert!(api.is_finalized_block(99, chain[99].header.hash.0.into()).await.is_err());
    assert!(api.get_finalized_receipt(B256::repeat_byte(1)).await?.is_none());
    witness.state.lock().unwrap().chain[98].header.inner.extra_data = vec![2].into();
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
    let view = api.verified_finalized_view_with_archive(&cleanup.directory, chain[0].header.hash).await?;
    let checkpoint = view.checkpoint().clone();
    let fence = std::fs::read(cleanup.directory.join("fence.json"))?;
    for fault in ["network", "genesis", "outage", "private", "regression", "malformed"] {
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
                "regression" => state.tip = 3,
                "malformed" => state.chain[4].header.hash = B256::ZERO,
                _ => unreachable!(),
            }
        }
        assert!(api.verified_finalized_view().await.is_err(), "{fault}");
        assert!(api.get_finalized_receipt(B256::ZERO).await.is_err(), "{fault}");
        assert_eq!(std::fs::read(cleanup.directory.join("fence.json"))?, fence, "{fault}");
        let mut state = witness.state.lock().unwrap();
        state.chain = chain.clone(); state.tip = 4; state.chain_id = 560048; state.unavailable = false;
    }
    // Only the old common header is binding: changing the advertised input tips is not a fork.
    primary.state.lock().unwrap().tip = 5;
    witness.state.lock().unwrap().tip = 5;
    let mut fresh = api.verified_finalized_view().await?;
    fresh.verify_checkpoint(api.raw_provider(), &checkpoint).await?;
    assert_eq!(fresh.block_number(), 5);
    let mut invalid = checkpoint.clone(); invalid.schema_version = 2;
    assert!(fresh.verify_checkpoint(api.raw_provider(), &invalid).await.is_err());
    let mut value = serde_json::to_value(&checkpoint)?;
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<FinalityCheckpoint>(value).is_err());
    Ok(())
}

#[tokio::test]
async fn nonce_processed_roots_and_scheduling_require_exact_hash_agreement() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chain = chain(154, 2);
    let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
    let primary = Fixture::new(chain.clone(), 2).await?;
    let witness = Fixture::new(chain.clone(), 2).await?;
    let api = primary.api(&witness).await?;
    let saved = api.get_finalized_submission_snapshot(7, [8; 32]).await?.unwrap();
    assert!(!saved.pinned_nonce_consumed && saved.message_processed);
    api.verify_finalized_submission_snapshot(7, [8; 32], &saved).await?;
    witness.state.lock().unwrap().nonce = 6; // same consumed bit is insufficient: exact nonce must agree.
    assert!(api.get_finalized_submission_snapshot(7, [8; 32]).await.is_err());
    assert!(api.verify_finalized_submission_snapshot(7, [8; 32], &saved).await.is_err());
    witness.state.lock().unwrap().nonce = 7;
    witness.state.lock().unwrap().processed = false;
    assert!(api.is_message_processed([8; 32]).await.is_err());
    witness.state.lock().unwrap().processed = true;
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
            if matches!(request["method"].as_str(), Some("eth_call" | "eth_getTransactionCount")) {
                assert_eq!(request["params"][1], json!({"blockHash":chain[2].header.hash, "requireCanonical":true}));
            }
        }
    }
    Ok(())
}

async fn receipt_chain(seed: u8, status: bool) -> AnyResult<(Vec<Block>, TransactionReceipt, Vec<u8>)> {
    let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(62))?;
    let mut transaction = TxEip1559 {
        chain_id: 560048, nonce: 7, gas_limit: 100_000, max_fee_per_gas: 10,
        max_priority_fee_per_gas: 1, to: TxKind::Call(Address::repeat_byte(9)), ..Default::default()
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
    chain[1].header.inner.transactions_root = calculate_transaction_root(&[envelope.clone()]);
    chain[1].header.inner.receipts_root = calculate_receipt_root(&[consensus.clone()]);
    chain[1].header.hash = chain[1].header.inner.hash_slow();
    let mut value = serde_json::to_value(consensus)?;
    let coordinates = json!({
        "transactionHash":hash, "transactionIndex":"0x0", "blockNumber":"0x1",
        "blockHash":chain[1].header.hash,
    });
    value.as_object_mut().unwrap().extend(coordinates.as_object().unwrap().clone());
    value.as_object_mut().unwrap().extend(json!({
        "gasUsed":"0x5208", "effectiveGasPrice":"0x1", "from":signer.address(),
        "to":Address::repeat_byte(9), "contractAddress":null,
    }).as_object().unwrap().clone());
    for (index, log) in value["logs"].as_array_mut().unwrap().iter_mut().enumerate() {
        log.as_object_mut().unwrap().extend(coordinates.as_object().unwrap().clone());
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
            state.receipt = Some(receipt.clone()); state.receipts = vec![receipt.clone()];
        }
        let api = primary.api(&witness).await?;
        let evidence = api.get_finalized_receipt(receipt.transaction_hash).await?.unwrap();
        assert_eq!(evidence.inclusion.raw_transaction, raw);
        assert_eq!(evidence.inclusion.receipt.status(), status);
        let mut view = api.verified_finalized_view().await?;
        view.verify_checkpoint(api.raw_provider(), &evidence.checkpoint).await?;
        evidence.inclusion.verify_original_inclusion(&view, &raw, receipt.transaction_hash)?;
        let restored: inclusion::RecordedReceiptProof = serde_json::from_slice(&serde_json::to_vec(&evidence.inclusion)?)?;
        restored.verify_original_inclusion(&view, &raw, receipt.transaction_hash)?;
        assert!(restored.verify_original_inclusion(&view, &[1], receipt.transaction_hash).is_err());
        for transaction in [true, false] {
            let mut tampered = restored.clone();
            if transaction { tampered.transaction_proof.clear(); } else { tampered.receipt_proof.clear(); }
            assert!(tampered.verify_inclusion(&view, receipt.transaction_hash).is_err());
        }
        let mut trailing = restored.clone();
        trailing.raw_transaction.push(0);
        assert!(validate_receipt_locator(&receipt, &trailing, receipt.transaction_hash).is_err());
        assert!(matches!(api.get_tx_status(receipt.transaction_hash).await?, TxStatus::Finalized) == status);
        for field in ["status", "transactionHash", "transactionIndex", "from", "to", "logsBloom"] {
            let mut value = serde_json::to_value(&receipt)?;
            value[field] = match field {
                "status" => json!(if status {"0x0"} else {"0x1"}),
                "transactionHash" => json!(B256::repeat_byte(99)),
                "transactionIndex" => json!("0x1"),
                "from" | "to" => json!(Address::repeat_byte(98)),
                "logsBloom" => json!(format!("0x{}", "ff".repeat(256))),
                _ => unreachable!(),
            };
            primary.state.lock().unwrap().receipt = Some(serde_json::from_value(value)?);
            assert!(api.get_finalized_receipt(receipt.transaction_hash).await.is_err(), "{field}");
        }
        if status {
            for (field, bad) in [
                ("removed", json!(true)), ("logIndex", json!("0x1")),
                ("transactionIndex", json!("0x1")), ("blockNumber", json!("0x0")),
                ("blockHash", json!(B256::ZERO)), ("transactionHash", json!(B256::ZERO)),
                ("data", json!("0x03")), ("topics", json!([B256::ZERO])),
            ] {
                let mut changed = serde_json::to_value(&receipt)?;
                changed["logs"][0][field] = bad;
                primary.state.lock().unwrap().receipt = Some(serde_json::from_value(changed)?);
                assert!(api.get_tx_status(receipt.transaction_hash).await.is_err(), "{field}");
            }
        }
        let mut changed = serde_json::to_value(&receipt)?;
        changed["status"] = json!(if status {"0x0"} else {"0x1"});
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
    finality_archive::HeaderArchive::open(&first)?.witness(genesis, &chain[2].header.inner, None)?;
    finality_archive::HeaderArchive::open(&second)?.witness(genesis, &chain[4].header.inner, None)?;
    register_archive(&first, genesis)?; register_archive(&second, genesis)?;
    register_archive(&first.join("."), genesis)?;
    assert_eq!(archives_for(genesis).len(), 2);
    assert!(register_archive(&first, B256::ZERO).is_err());
    let primary = Fixture::new(chain.clone(), 6).await?;
    let witness = Fixture::new(chain.clone(), 6).await?;
    let api = primary.api(&witness).await?;
    api.verified_finalized_view().await?;
    for directory in [&first, &second] {
        let fence: Value = serde_json::from_slice(&std::fs::read(directory.join("fence.json"))?)?;
        assert_eq!(fence["quorum_checkpoint"]["schema_version"], 1);
        assert_eq!(fence["tips"].as_array().unwrap().len(), 1);
    }
    let old = std::fs::read(second.join("fence.json"))?;
    FINALIZED_ANCESTRY.get().unwrap().lock().unwrap().remove(&genesis);
    FINALITY_ARCHIVES.get().unwrap().lock().unwrap().remove(&genesis);
    register_archive(&first, genesis)?; register_archive(&second, genesis)?;
    primary.state.lock().unwrap().tip = 5; witness.state.lock().unwrap().tip = 5;
    assert!(api.verified_finalized_view().await.is_err());
    assert_eq!(std::fs::read(second.join("fence.json"))?, old);
    primary.state.lock().unwrap().tip = 8; witness.state.lock().unwrap().tip = 8;
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
async fn durable_history_has_no_age_cutoff_and_preserves_corrupt_hints() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let height = 131_850u64;
    let chain = chain(158, height);
    let genesis = chain[0].header.hash;
    let cleanup = temp_archive(genesis);
    register_archive(&cleanup.directory, genesis)?;
    let archives = archives_for(genesis);
    for chunk in chain[1..].rchunks(512) {
        archives[0].save(&chunk.iter().rev().map(|block| block.header.inner.clone()).collect::<Vec<_>>())?;
    }
    let primary = Fixture::new(chain.clone(), height as usize).await?;
    let witness = Fixture::new(chain.clone(), height as usize).await?;
    let api = primary.api(&witness).await?;
    let mut view = api.verified_finalized_view().await?;
    let targets = (0..8_430).map(|index| (index * 15, chain[(index * 15) as usize].header.hash)).collect();
    view.verify_blocks(api.raw_provider(), &targets).await?;
    assert!(view.contains(0, genesis));
    let damaged = std::fs::read_dir(&cleanup.directory)?.filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().starts_with(&format!("headers-{height}-"))).unwrap().path();
    std::fs::write(&damaged, b"corrupt")?;
    let mut fresh = api.verified_finalized_view().await?;
    fresh.verify_blocks(api.raw_provider(), &BTreeMap::from([(height - 512, chain[(height - 512) as usize].header.hash)])).await?;
    assert_eq!(std::fs::read(damaged)?, b"corrupt");
    Ok(())
}

#[tokio::test]
async fn all_unconfigured_finality_boundaries_preserve_the_typed_hold() -> AnyResult<()> {
    let chain = chain(159, 1);
    let fixture = Fixture::new(chain.clone(), 1).await?;
    let api = EthApi::new(&fixture.url, &format!("{:#x}", Address::repeat_byte(9)), None, None, None).await?;
    let polling = PollingEthApi::new(&fixture.url).await?;
    assert!(api.clone().with_finality_witness(&format!("{}/#same", fixture.url), 560048, chain[0].header.hash).await.is_err());
    assert!(api.clone().with_finality_witness("http://127.0.0.1:1", 1, chain[0].header.hash).await.is_err());
    for result in [
        polling.get_finalized_receipt(B256::ZERO).await.map(|_| ()),
        polling.finalized_block().await.map(|_| ()),
        polling.is_finalized_block(0, chain[0].header.hash.0.into()).await.map(|_| ()),
        api.finalized_block_number().await.map(|_| ()),
        api.verify_finalized_submission_snapshot(0, [0; 32], &FinalizedSubmissionSnapshot {
            block_number: 0, block_hash: B256::ZERO, pinned_nonce_consumed: false, message_processed: false,
        }).await,
    ] {
        assert!(matches!(result.unwrap_err().downcast_ref::<Error>(), Some(Error::FinalityWitnessNotConfigured)));
    }
    for error in [
        api.get_finalized_receipt(B256::ZERO).await.unwrap_err(),
        api.get_tx_status(B256::ZERO).await.unwrap_err(),
        api.verified_finalized_view().await.unwrap_err(),
        api.get_finalized_submission_snapshot(0, [0; 32]).await.unwrap_err(),
        api.is_message_processed([0; 32]).await.unwrap_err(),
        api.read_finalized_merkle_root(0).await.unwrap_err(),
        api.read_finalized_merkle_root_at(0, 0, B256::ZERO).await.unwrap_err(),
        api.max_block_number().await.unwrap_err(),
        api.max_block_distance().await.unwrap_err(),
        api.process_admin_message_delay().await.unwrap_err(),
        api.process_pauser_message_delay().await.unwrap_err(),
        api.process_user_message_delay().await.unwrap_err(),
    ] {
        assert!(matches!(error, Error::FinalityWitnessNotConfigured));
    }
    Ok(())
}

#[tokio::test]
async fn inconsistent_tip_parent_links_and_receipts_above_common_remain_held() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let mut chain = chain(160, 3);
    let _owned = super::tests::ClearOwnedChains([chain[0].header.hash]);
    let primary = Fixture::new(chain.clone(), 3).await?;
    let witness = Fixture::new(chain.clone(), 1).await?;
    let api = primary.api(&witness).await?;
    let (_, mut receipt, _) = receipt_chain(161, true).await?;
    receipt.block_number = Some(2); receipt.block_hash = Some(chain[2].header.hash);
    primary.state.lock().unwrap().receipt = Some(receipt.clone());
    assert!(api.get_finalized_receipt(receipt.transaction_hash).await?.is_none());
    chain[3].header.inner.parent_hash = B256::repeat_byte(222);
    chain[3].header.hash = chain[3].header.inner.hash_slow();
    primary.state.lock().unwrap().chain = chain;
    assert!(api.verified_finalized_view().await.is_err());
    assert!(api.get_tx_status(receipt.transaction_hash).await.is_err());
    Ok(())
}

#[tokio::test]
async fn historical_verification_budget_is_per_call_not_view_lifetime() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    for (seed, archived) in [(173, false), (174, true)] {
        let chain = chain(seed, 4);
        let cleanup = temp_archive(chain[0].header.hash);
        let primary = Fixture::new(chain.clone(), 4).await?;
        let witness = Fixture::new(chain.clone(), 4).await?;
        let api = primary.api(&witness).await?;
        let mut view = if archived {
            api.verified_finalized_view_with_archive(&cleanup.directory, chain[0].header.hash).await?
        } else {
            api.verified_finalized_view().await?
        };
        let checkpoint = view.checkpoint().clone();
        let target = BTreeMap::from([(2, chain[2].header.hash)]);
        view.verify_blocks(api.raw_provider(), &target).await?;

        // Simulate time spent processing other journal records, without aging the pin.
        tokio::time::pause();
        tokio::time::advance(MAX_PROOF_DURATION + Duration::from_secs(1)).await;
        tokio::time::resume();
        view.verify_blocks(api.raw_provider(), &target).await?;
        view.verify_blocks(api.raw_provider(), &BTreeMap::from([(0, chain[0].header.hash)])).await?;
        assert!(view.contains(0, chain[0].header.hash));
        assert_eq!(view.checkpoint(), &checkpoint);

        witness.state.lock().unwrap().chain[4].header.inner.extra_data = vec![255].into();
        assert!(view.verify_blocks(api.raw_provider(), &target).await.is_err());
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn qualification_shares_one_total_budget_across_both_inputs() -> AnyResult<()> {
    use alloy::transports::mock::{Asserter, MockTransport};
    let chain = chain(162, 2);
    let primary = Asserter::new();
    let secondary = Asserter::new();
    let delayed = |asserter| {
        let client = alloy::rpc::client::RpcClient::builder()
            .layer(tower::util::MapFutureLayer::new(
                |future| -> alloy::transports::TransportFut<'static> {
                    Box::pin(async move {
                        let response = future.await;
                        tokio::time::sleep(Duration::from_secs(8)).await;
                        response
                    })
                },
            )).transport(MockTransport::new(asserter), true);
        RootProvider::builder().connect_client(client)
    };
    let provider = delayed(primary.clone());
    let witness = FinalityWitness { provider: delayed(secondary.clone()), chain_id: 560048, genesis: chain[0].header.hash };
    for (queue, tip) in [(&primary, 2), (&secondary, 1)] {
        queue.push_success(&"0x88bb0");
        queue.push_success(&chain[0]);
        queue.push_success(&chain[tip]);
        queue.push_success(&chain[1]);
    }
    // The initial network/header reads count against the same traversal budget.
    let started = Instant::now() - Duration::from_secs(250);
    assert!(matches!(finality_pin(&provider, &witness, started).await, Err(Error::FinalizedAncestryPending)));
    assert!(started.elapsed() >= MAX_PROOF_DURATION);
    Ok(())
}

#[tokio::test]
async fn unfinished_common_generations_survive_cache_capacity_pressure() -> AnyResult<()> {
    let _serial = super::tests::ANCESTRY_TEST_LOCK.lock().await;
    let chains: Vec<_> = (163..=171).map(|seed| chain(seed, 3)).collect();
    let _owned = super::tests::ClearOwnedChains::<9>(std::array::from_fn(|i| chains[i][0].header.hash));
    let mut fenced = Vec::new();
    for chain in &chains[..8] {
        let primary = Fixture::new(chain.clone(), 1).await?;
        let witness = Fixture::new(chain.clone(), 1).await?;
        let api = primary.api(&witness).await?;
        api.verified_finalized_view().await?;
        primary.state.lock().unwrap().tip = 3;
        witness.state.lock().unwrap().tip = 3;
        primary.state.lock().unwrap().missing.insert(2);
        assert!(api.verified_finalized_view().await.is_err());
        fenced.push((primary, witness, api));
    }
    let primary = Fixture::new(chains[8].clone(), 1).await?;
    let witness = Fixture::new(chains[8].clone(), 1).await?;
    let newcomer = primary.api(&witness).await?;
    assert!(matches!(newcomer.verified_finalized_view().await, Err(Error::FinalizedAncestryPending)));
    for (_, _, api) in &fenced {
        assert!(matches!(api.is_message_processed([1; 32]).await, Err(Error::FinalizedAncestryPending)));
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
    primary.state.lock().unwrap().tip = 4;
    witness.state.lock().unwrap().tip = 4;
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
        state.chain = fork.clone(); state.tip = 6;
    }
    primary.state.lock().unwrap().missing.insert(5);
    assert!(api.verified_finalized_view().await.is_err());
    primary.state.lock().unwrap().missing.clear();
    assert!(matches!(api.is_message_processed([1; 32]).await, Err(Error::FinalizedAncestryConflict)));
    Ok(())
}
