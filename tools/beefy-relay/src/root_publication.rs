use alloy::primitives::{Address, B256, U256};
use anyhow::{anyhow, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};
/// Publish a finalized token registration root and persist enough evidence to reconcile retries.
fn state_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub fn lock_token_deployment(manifest_path: &Path) -> Result<File> {
    let manifest_path = fs::canonicalize(manifest_path)?;
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let chain_id = manifest["ethereum"]["chainId"]
        .as_u64()
        .ok_or_else(|| anyhow!("deployment chain id is missing"))?;
    let queue: alloy::primitives::Address = manifest["ethereum"]["queue"]
        .as_str()
        .ok_or_else(|| anyhow!("deployment queue is missing"))?
        .parse()?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state_directory(&manifest_path).join(format!("beefy-{chain_id}-{queue:x}.lock")))?;
    lock.try_lock().map_err(|error| {
        anyhow!("another BEEFY actor or maintenance publisher owns this deployment: {error}")
    })?;
    Ok(lock)
}

fn persist_root_intent(path: &Path, intent: &Value) -> Result<()> {
    let parent = state_directory(path);
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, intent)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn decode_hash(value: &Value, field: &str) -> Result<[u8; 32]> {
    let text = value
        .as_str()
        .ok_or_else(|| anyhow!("{field} is missing"))?;
    hex::decode(text.strip_prefix("0x").unwrap_or(text))?
        .try_into()
        .map_err(|_| anyhow!("{field} must be 32 bytes"))
}

pub fn ensure_source_identity(
    manifest: &Value,
    source_genesis: [u8; 32],
    bridge_domain: [u8; 32],
    witness_genesis: [u8; 32],
    witness_domain: [u8; 32],
) -> Result<()> {
    ensure!(
        source_genesis == witness_genesis
            && bridge_domain == witness_domain
            && manifest["sourceGenesis"] == format!("0x{}", hex::encode(source_genesis))
            && manifest["bridgeDomain"] == format!("0x{}", hex::encode(bridge_domain)),
        "source genesis or destination-bound bridge domain differs between authorities and manifest"
    );
    Ok(())
}

fn ensure_recaptured_anchor(
    registration_block: u32,
    (recorded_block, recorded_hash, recorded_raw): (u32, [u8; 32], &str),
    (source_block, source_hash, source_raw): (u32, [u8; 32], &[u8]),
    (witness_block, witness_hash, witness_raw): (u32, [u8; 32], &[u8]),
    supplied_raw: &[u8],
) -> Result<()> {
    ensure!(
        source_block == recorded_block
            && witness_block == recorded_block
            && recorded_block > registration_block,
        "accepted BEEFY anchor does not cover the registered queue root"
    );
    ensure!(
        recorded_hash == source_hash && source_hash == witness_hash,
        "accepted source anchor was reorged or differs between authorities"
    );
    let encoded_raw = format!("0x{}", hex::encode(supplied_raw));
    ensure!(
        recorded_raw == encoded_raw && source_raw == supplied_raw && witness_raw == supplied_raw,
        "recaptured signed commitment differs from publisher journal"
    );
    Ok(())
}

fn ensure_separate_evm_signers(
    follower: alloy::primitives::Address,
    publisher: alloy::primitives::Address,
) -> Result<()> {
    ensure!(
        follower != publisher,
        "root publisher and follower must use separate EVM signer accounts"
    );
    Ok(())
}

fn saved_publication_nonce(intent: &Value) -> Result<u64> {
    intent["nonce"]
        .as_str()
        .ok_or_else(|| anyhow!("saved root publication nonce is invalid"))?
        .parse::<u64>()
        .map_err(|_| anyhow!("saved root publication nonce is invalid"))
}

fn ensure_saved_nonce_available(nonce: u64, latest: u64, pending: u64) -> Result<()> {
    ensure!(
        latest == nonce && pending == nonce,
        "root publication nonce is uncertain; refusing a replacement nonce"
    );
    Ok(())
}

fn ensure_immutable_publication(saved: &Value, expected: &Value) -> Result<()> {
    for field in [
        "schemaVersion",
        "sourceBlock",
        "root",
        "kind",
        "localRehearsal",
        "ownedFixtureNetwork",
        "sourceIdentity",
        "acceptedAnchorTx",
        "acceptedAnchorClient",
        "acceptedCheckpoint",
        "sender",
        "proof",
        "queueProof",
    ] {
        ensure!(
            saved[field] == expected[field],
            "saved root publication {field} differs from independently recaptured evidence; hold original intent"
        );
    }
    Ok(())
}

async fn root_account_nonces(
    ethereum: &crate::ethereum::Ethereum,
    address: alloy::primitives::Address,
) -> Result<(u64, u64)> {
    use alloy::providers::Provider;

    let provider = ethereum.api.raw_provider();
    let latest = provider.get_transaction_count(address).await?;
    let pending = provider.get_transaction_count(address).pending().await?;
    Ok((latest, pending))
}

async fn finalized_ethereum_head(ethereum: &crate::ethereum::Ethereum) -> Result<(u64, B256)> {
    let head = ethereum.api.verified_finalized_view().await?;
    Ok((head.block_number(), head.block_hash()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReceiptFinality {
    Mined,
    Accepted,
    Reverted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootPublicationState {
    Pending,
    Mined,
    Accepted,
    Observed,
}

fn classify_receipt_finality(
    included_block: u64,
    included_hash: B256,
    succeeded: bool,
    finalized_block: u64,
    canonical_hash: Option<B256>,
) -> Result<ReceiptFinality> {
    ensure!(
        canonical_hash == Some(included_hash),
        "root transaction inclusion is no longer canonical; hold original signed intent"
    );
    Ok(if !succeeded {
        ReceiptFinality::Reverted
    } else if included_block > finalized_block {
        ReceiptFinality::Mined
    } else {
        ReceiptFinality::Accepted
    })
}

fn signed_publication_identity(
    intent: &Value,
    queue: Address,
) -> Result<ethereum_client::TransactionIdentity> {
    use alloy::{
        consensus::{Transaction as _, TxEnvelope},
        eips::Decodable2718,
        sol_types::SolCall,
    };
    let hash = B256::from(decode_hash(
        &intent["txHash"],
        "saved root transaction hash",
    )?);
    let raw = hex::decode(
        intent["rawTransaction"]
            .as_str()
            .context("saved root transaction bytes missing")?
            .trim_start_matches("0x"),
    )?;
    let identity = crate::ethereum::signed_transaction_identity(
        &raw,
        hash,
        saved_publication_nonce(intent)?,
        queue,
    )?;
    let sender: Address = intent["sender"]
        .as_str()
        .context("saved root publisher missing")?
        .parse()?;
    ensure!(
        identity.from == sender,
        "saved root signature differs from original publisher"
    );
    let signed = TxEnvelope::decode_2718(&mut raw.as_slice())?;
    {
        let chain = intent["sourceIdentity"]["destinationChainId"]
            .as_u64()
            .context("saved root chain identity missing; HOLD")?;
        ensure!(
            signed.chain_id() == Some(chain),
            "saved root transaction chain changed; HOLD"
        );
        let block = U256::from(
            intent["sourceBlock"]
                .as_u64()
                .context("root source block missing")?,
        );
        let proof = hex::decode(
            intent["queueProof"]
                .as_str()
                .context("saved root proof missing")?
                .trim_start_matches("0x"),
        )?;
        let input = match intent["kind"].as_str().unwrap_or("merkleRoot") {
            "emptyProgress" => ethereum_client::abi::IMessageQueue::submitEmptyQueueProgressCall {
                sourceBlock: block,
                authenticatedSnapshotProof: proof.into(),
            }
            .abi_encode(),
            "merkleRoot" => ethereum_client::abi::IMessageQueue::submitMerkleRootCall {
                blockNumber: block,
                merkleRoot: B256::from(decode_hash(&intent["root"], "saved root")?),
                proof: proof.into(),
            }
            .abi_encode(),
            _ => return Err(anyhow!("unknown signed root kind; HOLD")),
        };
        ensure!(
            signed.input().as_ref() == input.as_slice() && signed.value().is_zero(),
            "saved root transaction changed original calldata or value; HOLD"
        );
    }
    Ok(identity)
}

fn validate_root_intent(intent: &Value, queue: Address) -> Result<()> {
    ensure!(
        matches!(intent["schemaVersion"].as_u64(), Some(3 | 4)),
        "unsupported root publication schema; HOLD"
    );
    ensure!(
        intent.get("nonce").is_some()
            && intent.get("txHash").is_some()
            && intent.get("rawTransaction").is_some()
            && intent["txHash"].is_null() == intent["rawTransaction"].is_null()
            && (intent["txHash"].is_null() || !intent["nonce"].is_null()),
        "root publication has mixed original identities; HOLD"
    );
    let kind = intent["kind"].as_str().unwrap_or("merkleRoot");
    let root = decode_hash(&intent["root"], "root intent")?;
    ensure!(
        matches!(kind, "merkleRoot" | "emptyProgress")
            && ((kind == "emptyProgress") == (root == [0; 32])),
        "root kind or value is invalid; HOLD"
    );
    let _: u32 = intent["sourceBlock"]
        .as_u64()
        .context("root source height missing")?
        .try_into()?;
    if !intent["nonce"].is_null() {
        saved_publication_nonce(intent)?;
    }
    if !intent["txHash"].is_null() {
        signed_publication_identity(intent, queue)?;
    }
    ensure!(
        intent["publicationReceipt"].is_null() || !intent["txHash"].is_null(),
        "root receipt has no original signed transaction; HOLD"
    );
    ensure!(
        intent["schemaVersion"] != 3 || intent["observedProgress"].is_null(),
        "legacy root intent contains unclassified external evidence; HOLD"
    );
    if intent["status"] == "observed" {
        ensure!(
            intent["schemaVersion"] == 4
                && intent["txHash"].is_null()
                && intent["publicationReceipt"].is_null()
                && intent["observedProgress"].is_object(),
            "observed root conflates original publication outcome; HOLD"
        );
    }
    if let Some(terminals) = intent["terminalSubmissions"].as_array() {
        let mut previous_nonce = None;
        for terminal in terminals {
            let original = &terminal["original"];
            ensure!(
                original["terminalSubmissions"].is_null(),
                "nested terminal root history; HOLD"
            );
            let signed = signed_publication_identity(original, queue)?;
            ensure!(
                previous_nonce.is_none_or(|previous| signed.nonce > previous),
                "terminal root originals are duplicated or reordered; HOLD"
            );
            previous_nonce = Some(signed.nonce);
            for field in ["sourceBlock", "root", "kind", "sourceIdentity", "sender"] {
                ensure!(
                    original[field] == intent[field],
                    "terminal root changed {field}; HOLD"
                );
            }
            ensure!(
                matches!(
                    terminal["disposition"].as_str(),
                    Some("reverted" | "nonceConsumed")
                ),
                "unknown terminal root disposition; HOLD"
            );
            ensure!(
                terminal["failureEvidence"].is_object() != terminal["nonceDisposition"].is_object(),
                "terminal root has missing or mixed outcome evidence; HOLD"
            );
        }
    } else {
        ensure!(
            intent["terminalSubmissions"].is_null(),
            "invalid terminal root history; HOLD"
        );
    }
    Ok(())
}

fn refresh_unsigned_intent(saved: &Value, expected: &Value) -> Result<Value> {
    if !saved["txHash"].is_null() {
        ensure_immutable_publication(saved, expected)?;
        return Ok(saved.clone());
    }
    for field in [
        "sourceBlock",
        "root",
        "kind",
        "localRehearsal",
        "ownedFixtureNetwork",
        "sourceIdentity",
        "sender",
    ] {
        ensure!(
            saved[field] == expected[field],
            "unsigned root changed {field}; HOLD"
        );
    }
    ensure!(
        saved["publicationReceipt"].is_null(),
        "unsigned root has an original receipt; HOLD"
    );
    let mut refreshed = expected.clone();
    for field in [
        "nonce",
        "txHash",
        "rawTransaction",
        "terminalSubmissions",
        "observedProgress",
        "originalDisposition",
    ] {
        refreshed[field] = saved[field].clone();
    }
    refreshed["status"] = json!("prepared");
    Ok(refreshed)
}

async fn authenticated_root_receipt(
    ethereum: &crate::ethereum::Ethereum,
    path: &Path,
    hash: B256,
    included: (u64, B256),
    finalized: (u64, B256),
) -> Result<crate::ethereum::RecordedReceiptProof> {
    ensure!(
        finalized.0 >= included.0,
        "root finalized pin precedes original inclusion; HOLD"
    );
    let directory = state_directory(path).join("finality-history");
    let mut view = ethereum
        .api
        .verified_finalized_view_with_archive(
            &directory.join("headers"),
            ethereum.finality_genesis().await?,
        )
        .await?;
    let mut targets = std::collections::BTreeMap::new();
    for (number, hash) in [included, finalized] {
        ensure!(
            targets.insert(number, hash).is_none_or(|old| old == hash),
            "root evidence conflicts at one finalized height; HOLD"
        );
    }
    view.verify_blocks(ethereum.api.raw_provider(), &targets)
        .await?;
    crate::ethereum::authenticate_transaction_receipt(
        ethereum,
        &view,
        included.0,
        included.1,
        hash,
        &directory.join("receipts"),
    )
    .await
}

fn accepted_root_transaction(
    proof: &crate::ethereum::RecordedReceiptProof,
    chain: u64,
) -> Result<Value> {
    use alloy::{
        consensus::{Transaction as _, TxEnvelope},
        eips::Decodable2718,
    };
    let mut encoded = proof.raw_transaction.as_slice();
    let signed = TxEnvelope::decode_2718(&mut encoded)?;
    ensure!(
        encoded.is_empty() && signed.chain_id() == Some(chain),
        "accepted root transaction has wrong chain identity; HOLD"
    );
    let destination = signed
        .to()
        .context("accepted root transaction has no destination; HOLD")?;
    let hash = B256::from(crate::keccak256(&proof.raw_transaction));
    let identity = crate::ethereum::signed_transaction_identity(
        &proof.raw_transaction,
        hash,
        signed.nonce(),
        destination,
    )?;
    Ok(json!({"txHash": format!("{hash:#x}"),
        "rawTransaction": format!("0x{}", hex::encode(&proof.raw_transaction)),
        "nonce": identity.nonce.to_string(), "sender": format!("{:#x}", identity.from),
        "to": format!("{destination:#x}"), "chainId": chain}))
}

fn queue_progress_event(
    log: &alloy::primitives::Log,
    queue: Address,
) -> Result<Option<(&'static str, u32, [u8; 32])>> {
    if log.address != queue {
        return Ok(None);
    }
    let topics = log.data.topics();
    let data = log.data.data.as_ref();
    let merkle = B256::from(crate::keccak256(b"MerkleRoot(uint256,bytes32)"));
    let empty = B256::from(crate::keccak256(b"EmptyQueueProgress(uint256)"));
    if topics.first() == Some(&merkle) {
        ensure!(
            topics.len() == 1 && data.len() == 64,
            "malformed root event; HOLD"
        );
        return Ok(Some((
            "merkleRoot",
            U256::from_be_slice(&data[..32]).try_into()?,
            data[32..].try_into()?,
        )));
    }
    if topics.first() == Some(&empty) {
        ensure!(
            topics.len() == 2 && data.is_empty(),
            "malformed empty progress event; HOLD"
        );
        return Ok(Some((
            "emptyProgress",
            U256::from_be_slice(topics[1].as_slice()).try_into()?,
            [0; 32],
        )));
    }
    Ok(None)
}

fn authenticated_publication_matches(
    proof: &crate::ethereum::RecordedReceiptProof,
    queue: Address,
    target: u32,
    kind: &str,
    root: [u8; 32],
) -> bool {
    proof.receipt.status()
        && proof.receipt.logs().iter().any(|log| {
            queue_progress_event(log, queue).is_ok_and(|event| event == Some((kind, target, root)))
        })
}

fn matching_progress(
    target: u32,
    kind: &str,
    root: [u8; 32],
    event: (&str, u32, [u8; 32]),
) -> bool {
    if kind == "emptyProgress" {
        event.1 >= target
    } else {
        event.0 == "merkleRoot" && event.1 == target && event.2 == root
    }
}

async fn revalidate_root_finalized_pin(
    ethereum: &crate::ethereum::Ethereum,
    number: u64,
    hash: B256,
) -> Result<()> {
    ensure!(
        ethereum
            .api
            .is_finalized_block(number, hash.0.into())
            .await?,
        "root finalized pin changed during state observation; HOLD"
    );
    Ok(())
}

async fn validate_observed_progress(
    ethereum: &crate::ethereum::Ethereum,
    path: &Path,
    intent: &Value,
) -> Result<()> {
    use alloy::rpc::types::BlockId;
    let observed = &intent["observedProgress"];
    ensure!(
        observed.is_object()
            && observed["kind"] == intent["kind"]
            && observed["sourceBlock"] == intent["sourceBlock"]
            && observed["root"] == intent["root"],
        "external root evidence changed target; HOLD"
    );
    let included = (
        observed["includedBlock"]
            .as_u64()
            .context("observed inclusion missing")?,
        B256::from(decode_hash(
            &observed["includedBlockHash"],
            "observed inclusion hash",
        )?),
    );
    let finalized = (
        observed["finalizedBlock"]
            .as_u64()
            .context("observed finality missing")?,
        B256::from(decode_hash(
            &observed["finalizedBlockHash"],
            "observed finalized hash",
        )?),
    );
    ensure!(
        finalized.0 >= included.0,
        "external root finality precedes inclusion; HOLD"
    );
    let hash = B256::from(decode_hash(
        &observed["actualTransactionHash"],
        "external root transaction",
    )?);
    let proof = authenticated_root_receipt(ethereum, path, hash, included, finalized).await?;
    ensure!(
        proof.receipt.status() && serde_json::to_value(&proof)? == observed["receiptProof"],
        "external root authenticated receipt changed; HOLD"
    );
    let transaction = accepted_root_transaction(
        &proof,
        intent["sourceIdentity"]["destinationChainId"]
            .as_u64()
            .context("root chain missing")?,
    )?;
    ensure!(
        transaction == observed["acceptedTransaction"],
        "external root signed identity changed; HOLD"
    );
    let target: u32 = intent["sourceBlock"]
        .as_u64()
        .context("root block missing")?
        .try_into()?;
    let kind = intent["kind"].as_str().context("root kind missing")?;
    let root = decode_hash(&intent["root"], "root target")?;
    let event = (
        observed["progressKind"]
            .as_str()
            .context("external progress kind missing")?,
        observed["progressSourceBlock"]
            .as_u64()
            .context("external progress source missing")?
            .try_into()?,
        decode_hash(&observed["progressRoot"], "external progress root")?,
    );
    ensure!(
        matching_progress(target, kind, root, event)
            && proof.receipt.logs().iter().any(|log| queue_progress_event(
                log,
                ethereum.queue_address
            )
            .is_ok_and(|candidate| candidate == Some(event))),
        "external receipt does not authenticate target progress; HOLD"
    );
    let queue = ethereum_client::abi::IMessageQueue::new(
        ethereum.queue_address,
        ethereum.api.raw_provider().clone(),
    );
    if kind == "emptyProgress" {
        ensure!(
            queue
                .maxBlockNumber()
                .block(BlockId::hash_canonical(included.1))
                .call()
                .await?
                >= U256::from(target),
            "external receipt does not cover empty target; HOLD"
        );
    } else {
        ensure!(
            queue
                .getMerkleRoot(U256::from(target))
                .block(BlockId::hash_canonical(included.1))
                .call()
                .await?
                == B256::from(root),
            "external receipt did not register exact historical root; HOLD"
        );
    }
    revalidate_root_finalized_pin(ethereum, finalized.0, finalized.1).await?;
    Ok(())
}

async fn observe_finalized_progress(
    ethereum: &crate::ethereum::Ethereum,
    output: &Path,
    intent: &mut Value,
    target: u32,
    kind: &str,
    root: [u8; 32],
) -> Result<bool> {
    use alloy::{
        providers::Provider,
        rpc::types::{BlockId, Filter},
    };
    if !intent["observedProgress"].is_null() {
        validate_observed_progress(ethereum, output, intent).await?;
        if intent["txHash"].is_null() {
            ensure!(
                intent["publicationReceipt"].is_null(),
                "external progress cannot replace original receipt; HOLD"
            );
            intent["status"] = json!("observed");
            intent["finalityStatus"] = json!("finalized");
            persist_root_intent(output, intent)?;
        }
        return Ok(true);
    }
    let (finalized_block, finalized_hash) = finalized_ethereum_head(ethereum).await?;
    let provider = ethereum.api.raw_provider();
    let queue = ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, provider.clone());
    let at = BlockId::hash_canonical(finalized_hash);
    if kind == "merkleRoot" {
        let stored = queue
            .getMerkleRoot(U256::from(target))
            .block(at)
            .call()
            .await?;
        ensure!(
            stored.is_zero() || stored == B256::from(root),
            "conflicting finalized queue root; HOLD"
        );
        if stored.is_zero() {
            return Ok(false);
        }
    } else if queue.maxBlockNumber().block(at).call().await? < U256::from(target) {
        return Ok(false);
    }
    let signatures = if kind == "emptyProgress" {
        vec![
            B256::from(crate::keccak256(b"MerkleRoot(uint256,bytes32)")),
            B256::from(crate::keccak256(b"EmptyQueueProgress(uint256)")),
        ]
    } else {
        vec![B256::from(crate::keccak256(b"MerkleRoot(uint256,bytes32)"))]
    };
    let mut candidates = Vec::new();
    let mut end = finalized_block;
    loop {
        let start = end.saturating_sub(1_999);
        for &signature in &signatures {
            let logs = provider
                .get_logs(
                    &Filter::new()
                        .address(ethereum.queue_address)
                        .event_signature(signature)
                        .from_block(start)
                        .to_block(end),
                )
                .await?;
            for log in logs {
                ensure!(!log.removed, "removed finalized root log; HOLD");
                let Some(event) = queue_progress_event(&log.inner, ethereum.queue_address)? else {
                    continue;
                };
                if !matching_progress(target, kind, root, event) {
                    continue;
                }
                let number = log
                    .block_number
                    .context("external root log block missing")?;
                let block_hash = log.block_hash.context("external root log hash missing")?;
                let hash = log
                    .transaction_hash
                    .context("external root log transaction missing")?;
                let index = log.log_index.context("external root log index missing")?;
                ensure!(
                    number >= start && number <= end,
                    "external root log is outside requested finalized range; HOLD"
                );
                candidates.push((number, index, block_hash, hash, event));
            }
        }
        if !candidates.is_empty() || start == 0 {
            break;
        }
        end = start - 1;
    }
    candidates.sort_by_key(|candidate| (candidate.0, candidate.1));
    let (number, _, block_hash, hash, event) = candidates
        .into_iter()
        .next()
        .context("finalized root state has no authenticated progress receipt; HOLD")?;
    if intent["txHash"].as_str() == Some(format!("{hash:#x}").as_str()) {
        return Ok(false);
    }
    let proof = authenticated_root_receipt(
        ethereum,
        output,
        hash,
        (number, block_hash),
        (finalized_block, finalized_hash),
    )
    .await?;
    ensure!(
        proof.receipt.status()
            && proof.receipt.logs().iter().any(|log| queue_progress_event(
                log,
                ethereum.queue_address
            )
            .is_ok_and(|candidate| candidate == Some(event))),
        "external log is absent from authenticated successful receipt; HOLD"
    );
    let transaction = accepted_root_transaction(
        &proof,
        intent["sourceIdentity"]["destinationChainId"]
            .as_u64()
            .context("root chain missing")?,
    )?;
    intent["observedProgress"] = json!({"kind":kind,"sourceBlock":target,
        "root":format!("0x{}",hex::encode(root)),
        "progressKind":event.0,"progressSourceBlock":event.1,
        "progressRoot":format!("0x{}",hex::encode(event.2)),
        "actualTransactionHash":format!("{hash:#x}"),"acceptedTransaction":transaction,
        "includedBlock":number,"includedBlockHash":format!("{block_hash:#x}"),
        "finalizedBlock":finalized_block,"finalizedBlockHash":format!("{finalized_hash:#x}"),
        "receiptProof":serde_json::to_value(&proof)?});
    if kind == "emptyProgress" {
        ensure!(
            queue
                .maxBlockNumber()
                .block(BlockId::hash_canonical(block_hash))
                .call()
                .await?
                >= U256::from(target),
            "external receipt did not cover empty target; HOLD"
        );
    } else {
        ensure!(
            queue
                .getMerkleRoot(U256::from(target))
                .block(BlockId::hash_canonical(block_hash))
                .call()
                .await?
                == B256::from(root),
            "external receipt did not register exact historical root; HOLD"
        );
    }
    revalidate_root_finalized_pin(ethereum, finalized_block, finalized_hash).await?;
    if intent["txHash"].is_null() {
        ensure!(
            intent["publicationReceipt"].is_null(),
            "external progress cannot replace original receipt; HOLD"
        );
        intent["status"] = json!("observed");
        intent["finalityStatus"] = json!("finalized");
    }
    persist_root_intent(output, intent)?;
    Ok(true)
}

fn archive_root_original(intent: &mut Value, disposition: &str, evidence: Value) -> Result<()> {
    ensure!(
        !intent["txHash"].is_null(),
        "terminal root has no original signature; HOLD"
    );
    let mut original = intent.clone();
    original
        .as_object_mut()
        .context("invalid root intent")?
        .remove("terminalSubmissions");
    let mut terminal = json!({"original":original,"disposition":disposition});
    terminal[if disposition == "reverted" {
        "failureEvidence"
    } else {
        "nonceDisposition"
    }] = evidence;
    if intent["terminalSubmissions"].is_null() {
        intent["terminalSubmissions"] = json!([]);
    }
    intent["terminalSubmissions"]
        .as_array_mut()
        .context("invalid terminal root history")?
        .push(terminal);
    for field in ["txHash", "rawTransaction", "nonce", "publicationReceipt"] {
        intent[field] = Value::Null;
    }
    intent["originalDisposition"] = json!(disposition);
    intent["status"] = json!(if disposition == "reverted" {
        "failed original archived"
    } else {
        "held original archived"
    });
    Ok(())
}

async fn revalidate_terminal_roots(
    ethereum: &crate::ethereum::Ethereum,
    path: &Path,
    intent: &Value,
) -> Result<()> {
    use alloy::{providers::Provider, rpc::types::BlockId};
    let Some(terminals) = intent["terminalSubmissions"].as_array() else {
        return Ok(());
    };
    for terminal in terminals {
        let original = &terminal["original"];
        let signed = signed_publication_identity(original, ethereum.queue_address)?;
        if terminal["disposition"] == "reverted" {
            let evidence = &terminal["failureEvidence"];
            let included = (
                evidence["includedBlock"]
                    .as_u64()
                    .context("failure inclusion missing")?,
                B256::from(decode_hash(
                    &evidence["includedBlockHash"],
                    "failure inclusion hash",
                )?),
            );
            let finalized = (
                evidence["finalizedBlock"]
                    .as_u64()
                    .context("failure finality missing")?,
                B256::from(decode_hash(
                    &evidence["finalizedBlockHash"],
                    "failure finalized hash",
                )?),
            );
            let proof =
                authenticated_root_receipt(ethereum, path, signed.hash, included, finalized)
                    .await?;
            ensure!(
                !proof.receipt.status()
                    && serde_json::to_value(&proof)? == evidence["receiptProof"]
                    && format!("0x{}", hex::encode(&proof.raw_transaction))
                        == original["rawTransaction"],
                "archived failed original receipt changed; HOLD"
            );
        } else {
            let evidence = &terminal["nonceDisposition"];
            let number = evidence["block"]
                .as_u64()
                .context("nonce disposition block missing")?;
            let hash = B256::from(decode_hash(
                &evidence["blockHash"],
                "nonce disposition hash",
            )?);
            let directory = state_directory(path).join("finality-history");
            let mut view = ethereum
                .api
                .verified_finalized_view_with_archive(
                    &directory.join("headers"),
                    ethereum.finality_genesis().await?,
                )
                .await?;
            view.verify_blocks(
                ethereum.api.raw_provider(),
                &std::collections::BTreeMap::from([(number, hash)]),
            )
            .await?;
            let nonce = ethereum
                .api
                .raw_provider()
                .get_transaction_count(signed.from)
                .block_id(BlockId::hash_canonical(hash))
                .await?;
            revalidate_root_finalized_pin(ethereum, number, hash).await?;
            ensure!(
                nonce > signed.nonce && evidence["accountNonce"].as_u64() == Some(nonce),
                "archived root nonce disposition changed; HOLD"
            );
            if let Some(receipt) = ethereum.api.get_finalized_receipt(signed.hash).await? {
                ensure!(
                    receipt.receipt.from == signed.from && receipt.receipt.to == signed.to,
                    "archived original receipt identity changed; HOLD"
                );
                let proof = authenticated_root_receipt(
                    ethereum,
                    path,
                    signed.hash,
                    (receipt.included_block_number, receipt.included_block_hash),
                    (receipt.finalized_block_number, receipt.finalized_block_hash),
                )
                .await?;
                ensure!(
                    format!("0x{}", hex::encode(&proof.raw_transaction))
                        == original["rawTransaction"],
                    "archived original signed bytes changed; HOLD"
                );
            }
        }
    }
    Ok(())
}

async fn reconcile_root_original(
    ethereum: &crate::ethereum::Ethereum,
    output: &Path,
    intent: &mut Value,
) -> Result<bool> {
    use alloy::{providers::Provider, rpc::types::BlockId};
    if intent["txHash"].is_null() {
        return Ok(false);
    }
    let signed = signed_publication_identity(intent, ethereum.queue_address)?;
    if let Some(receipt) = ethereum.api.get_finalized_receipt(signed.hash).await? {
        ensure!(
            receipt.receipt.from == signed.from && receipt.receipt.to == signed.to,
            "original finalized root receipt identity changed; HOLD"
        );
        if receipt.receipt.status() {
            return Ok(false);
        }
        let proof = authenticated_root_receipt(
            ethereum,
            output,
            signed.hash,
            (receipt.included_block_number, receipt.included_block_hash),
            (receipt.finalized_block_number, receipt.finalized_block_hash),
        )
        .await?;
        ensure!(
            !proof.receipt.status()
                && format!("0x{}", hex::encode(&proof.raw_transaction)) == intent["rawTransaction"],
            "failed root receipt is not the original signed transaction; HOLD"
        );
        archive_root_original(
            intent,
            "reverted",
            json!({
            "includedBlock":receipt.included_block_number,"includedBlockHash":format!("{:#x}",receipt.included_block_hash),
            "finalizedBlock":receipt.finalized_block_number,"finalizedBlockHash":format!("{:#x}",receipt.finalized_block_hash),
            "receiptProof":serde_json::to_value(&proof)?}),
        )?;
        persist_root_intent(output, intent)?;
        return Ok(true);
    }
    if ethereum
        .api
        .raw_provider()
        .get_transaction_receipt(signed.hash)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    let directory = state_directory(output).join("finality-history");
    let view = ethereum
        .api
        .verified_finalized_view_with_archive(
            &directory.join("headers"),
            ethereum.finality_genesis().await?,
        )
        .await?;
    let nonce = ethereum
        .api
        .raw_provider()
        .get_transaction_count(signed.from)
        .block_id(BlockId::hash_canonical(view.block_hash()))
        .await?;
    if nonce > signed.nonce {
        // Receipt availability may advance during the pinned nonce query. Keep
        // original ownership and reconcile that receipt before recording an unknown outcome.
        if ethereum
            .api
            .get_finalized_receipt(signed.hash)
            .await?
            .is_some()
        {
            return Ok(false);
        }
        revalidate_root_finalized_pin(ethereum, view.block_number(), view.block_hash()).await?;
        archive_root_original(
            intent,
            "nonceConsumed",
            json!({"block":view.block_number(),
            "blockHash":format!("{:#x}",view.block_hash()),"accountNonce":nonce,
            "originalReceiptKnown":false}),
        )?;
        persist_root_intent(output, intent)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) async fn validate_root_publication(
    ethereum: &crate::ethereum::Ethereum,
    path: &Path,
    intent: &Value,
) -> Result<()> {
    validate_root_intent(intent, ethereum.queue_address)?;
    revalidate_terminal_roots(ethereum, path, intent).await?;
    if !intent["observedProgress"].is_null() {
        validate_observed_progress(ethereum, path, intent).await?;
    }
    if !intent["publicationReceipt"].is_null() {
        let target = intent["sourceBlock"]
            .as_u64()
            .context("root block missing")?
            .try_into()?;
        let kind = intent["kind"].as_str().unwrap_or("merkleRoot");
        let root = decode_hash(&intent["root"], "saved root")?;
        let included = canonical_root_inclusion(ethereum, intent, target, kind, root)
            .await?
            .context("original root inclusion disappeared; HOLD")?;
        if intent["publicationReceipt"]["finalized"] == true {
            let pin = &intent["publicationReceipt"];
            let proof = authenticated_root_receipt(
                ethereum,
                path,
                B256::from(decode_hash(&intent["txHash"], "original root transaction")?),
                included,
                (
                    pin["finalizedBlock"]
                        .as_u64()
                        .context("root finality pin missing")?,
                    B256::from(decode_hash(
                        &pin["finalizedBlockHash"],
                        "root finality hash",
                    )?),
                ),
            )
            .await?;
            ensure!(
                authenticated_publication_matches(
                    &proof,
                    ethereum.queue_address,
                    target,
                    kind,
                    root
                ) && format!("0x{}", hex::encode(&proof.raw_transaction))
                    == intent["rawTransaction"],
                "original finalized root proof changed signed bytes or status; HOLD"
            );
            if !pin["receiptProof"].is_null() {
                ensure!(
                    pin["receiptProof"] == serde_json::to_value(&proof)?,
                    "original root proof changed; HOLD"
                );
            }
        }
    }
    Ok(())
}

fn persist_root_inclusion(
    path: &Path,
    intent: &mut Value,
    (block, hash): (u64, B256),
    kind: &str,
    finalized: Option<(u64, B256)>,
) -> Result<()> {
    ensure!(
        intent["publicationReceipt"]["finalized"] != true || finalized.is_some(),
        "previously finalized root lost finality; hold original signed intent"
    );
    if !intent["publicationReceipt"].is_null() {
        ensure!(
            intent["publicationReceipt"]["block"].as_u64() == Some(block)
                && intent["publicationReceipt"]["blockHash"].as_str()
                    == Some(format!("{hash:#x}").as_str())
                && intent["publicationReceipt"]["kind"].as_str() == Some(kind),
            "original root inclusion/kind changed; hold original signed intent"
        );
        if intent["publicationReceipt"]["finalized"] == true {
            return Ok(());
        }
    }
    let mut receipt = json!({
        "block": block, "blockHash": format!("{hash:#x}"),
        "finalized": finalized.is_some(), "kind": kind,
    });
    if let Some((finalized_block, finalized_hash)) = finalized {
        receipt["finalizedBlock"] = json!(finalized_block);
        receipt["finalizedBlockHash"] = json!(format!("{finalized_hash:#x}"));
    }
    intent["status"] = json!(if finalized.is_some() {
        "accepted"
    } else {
        "mined"
    });
    intent["finalityStatus"] = json!(if finalized.is_some() {
        "finalized"
    } else {
        "mined"
    });
    intent["publicationReceipt"] = receipt;
    persist_root_intent(path, intent)
}

fn ensure_pinned_latest_anchor(
    checkpoint: &crate::ethereum::DestinationCheckpoint,
    anchor_block: u32,
    anchor_root: [u8; 32],
) -> Result<()> {
    ensure!(
        checkpoint.block == u64::from(anchor_block) && checkpoint.root == anchor_root,
        "pinned BEEFY anchor is no longer latest; hold the original root intent"
    );
    Ok(())
}

async fn canonical_root_inclusion(
    ethereum: &crate::ethereum::Ethereum,
    intent: &Value,
    source_block: u32,
    kind: &str,
    expected_root: [u8; 32],
) -> Result<Option<(u64, B256)>> {
    use alloy::{
        providers::Provider,
        rpc::types::{BlockId, BlockNumberOrTag},
    };

    let signed = signed_publication_identity(intent, ethereum.queue_address)?;
    let hash = signed.hash;
    let provider = ethereum.api.raw_provider();
    let Some(receipt) = provider.get_transaction_receipt(hash).await? else {
        ensure!(
            intent["publicationReceipt"].is_null(),
            "previously mined root receipt disappeared; hold original signed intent"
        );
        return Ok(None);
    };
    let block = receipt
        .block_number
        .context("root receipt has no block number")?;
    let block_hash = receipt
        .block_hash
        .context("root receipt has no block hash")?;
    ensure!(
        receipt.transaction_hash == hash && receipt.from == signed.from && receipt.to == signed.to,
        "root receipt identity changed"
    );
    let canonical = provider
        .get_block_by_number(BlockNumberOrTag::Number(block))
        .await?
        .context("mined root receipt block is unavailable")?;
    let finality = classify_receipt_finality(
        block,
        block_hash,
        receipt.status(),
        0,
        Some(canonical.header.hash),
    )?;
    if finality == ReceiptFinality::Reverted {
        ensure!(
            intent["publicationReceipt"].is_null(),
            "original root inclusion changed status; HOLD"
        );
        return Ok(None);
    }
    let recorded = &intent["publicationReceipt"];
    ensure!(
        recorded.is_null()
            || (recorded["block"] == block
                && recorded["blockHash"] == format!("{block_hash:#x}")
                && recorded["kind"] == kind),
        "saved root receipt moved or changed; hold original signed intent"
    );
    let event = if kind == "emptyProgress" {
        B256::from(crate::keccak256(b"EmptyQueueProgress(uint256)"))
    } else {
        B256::from(crate::keccak256(b"MerkleRoot(uint256,bytes32)"))
    };
    let block_bytes = U256::from(source_block).to_be_bytes::<32>();
    ensure!(
        receipt.as_ref().logs().iter().any(|log| {
            let topics = log.topics();
            let data = log.data().data.as_ref();
            log.address() == ethereum.queue_address
                && topics.first() == Some(&event)
                && if kind == "emptyProgress" {
                    topics.len() == 2 && topics[1] == B256::from(block_bytes) && data.is_empty()
                } else {
                    topics.len() == 1
                        && data.len() == 64
                        && data[..32] == block_bytes
                        && data[32..] == expected_root
                }
        }),
        "canonical root receipt does not record the expected queue progress"
    );
    let queue = ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, provider.clone());
    let at = BlockId::hash_canonical(block_hash);
    let stored = queue
        .getMerkleRoot(U256::from(source_block))
        .block(at)
        .call()
        .await?;
    ensure!(
        stored == B256::from(expected_root)
            && (kind != "emptyProgress"
                || queue.maxBlockNumber().block(at).call().await? >= U256::from(source_block)),
        "canonical root receipt does not store the expected queue progress"
    );
    Ok(Some((block, block_hash)))
}

// Called under the deployment lock before advancing to another BEEFY commitment.
// Authenticated external progress removes the progress barrier, not original nonce ownership.
pub(crate) async fn ensure_mined_root_publications(
    ethereum: &crate::ethereum::Ethereum,
    roots: &Value,
) -> Result<()> {
    for root in roots
        .as_object()
        .context("root registration journal is missing")?
        .values()
    {
        let status = root["status"].as_str().context("root status missing")?;
        ensure!(
            matches!(status, "pending" | "mined" | "accepted" | "observed"),
            "unknown root status"
        );
        let path = Path::new(
            root["publication"]
                .as_str()
                .context("root publication path missing")?,
        );
        if !path.exists() && status == "pending" {
            continue;
        }
        let intent: Value = serde_json::from_slice(&fs::read(path)?)?;
        let block: u32 = root["block"]
            .as_u64()
            .context("root source block missing")?
            .try_into()?;
        let kind = root["kind"].as_str().unwrap_or("merkleRoot");
        let expected_root = decode_hash(&root["queueRoot"], "root registration")?;
        let saved_client: Address = intent["acceptedAnchorClient"]
            .as_str()
            .context("pinned BEEFY client missing")?
            .parse()?;
        let saved_queue: Address = intent["sourceIdentity"]["destinationQueue"]
            .as_str()
            .context("pinned queue missing")?
            .parse()?;
        ensure!(
            intent["sourceBlock"] == block
                && intent["root"] == root["queueRoot"]
                && intent["kind"] == kind
                && saved_client == ethereum.client_address
                && saved_queue == ethereum.queue_address,
            "saved root publication differs from registration or active deployment"
        );
        validate_root_publication(ethereum, path, &intent).await?;
        if !intent["observedProgress"].is_null() {
            continue;
        }
        ensure!(
            !intent["txHash"].is_null()
                && !intent["publicationReceipt"].is_null()
                && canonical_root_inclusion(ethereum, &intent, block, kind, expected_root)
                    .await?
                    .is_some(),
            "root publication has no canonical progress; hold next handover"
        );
    }
    Ok(())
}

fn hold_root_reconciliation(output: &Path, intent: &mut Value) -> Result<RootPublicationState> {
    let state = if intent["publicationReceipt"].is_null() {
        intent["status"] = json!(if intent["txHash"].is_null() {
            if intent["originalDisposition"] == "nonceConsumed" {
                "held: archived original nonce consumed"
            } else {
                "prepared: no active signed root transaction"
            }
        } else {
            "held: original signed root remains unmined"
        });
        intent["finalityStatus"] = json!("pending");
        RootPublicationState::Pending
    } else {
        RootPublicationState::Mined
    };
    intent["lastError"] =
        json!("reconciliation held; retain original signed hash, nonce, anchor and inclusion");
    persist_root_intent(output, intent)?;
    Ok(state)
}

async fn wait_for_root_receipt(
    ethereum: &crate::ethereum::Ethereum,
    output: &Path,
    intent: &mut Value,
    source_block: u32,
    kind: &str,
    expected_root: [u8; 32],
    (_anchor_block, _anchor_root, anchor_tx, anchor_inclusion): (u32, [u8; 32], B256, (u64, B256)),
) -> Result<RootPublicationState> {
    use alloy::providers::Provider;
    use std::time::{Duration, Instant};

    let signed = signed_publication_identity(intent, ethereum.queue_address)?;
    let transaction_hash = signed.hash;
    let provider = ethereum.api.raw_provider();
    let deadline = Instant::now() + Duration::from_secs(30);
    let observation = tokio::time::timeout_at(deadline.into(), async {
        loop {
            if reconcile_root_original(ethereum, output, intent).await? {
                let observed = observe_finalized_progress(
                    ethereum,
                    output,
                    intent,
                    source_block,
                    kind,
                    expected_root,
                )
                .await?;
                return Ok(if observed {
                    RootPublicationState::Observed
                } else {
                    RootPublicationState::Pending
                });
            }
            observe_finalized_progress(ethereum, output, intent, source_block, kind, expected_root)
                .await?;
            if let Some((block, hash)) =
                canonical_root_inclusion(ethereum, intent, source_block, kind, expected_root)
                    .await?
            {
                if intent["publicationReceipt"]["finalized"] != true {
                    persist_root_inclusion(output, intent, (block, hash), kind, None)?;
                }
                let (finalized_block, _) = finalized_ethereum_head(ethereum).await?;
                if classify_receipt_finality(block, hash, true, finalized_block, Some(hash))?
                    == ReceiptFinality::Mined
                {
                    persist_root_inclusion(output, intent, (block, hash), kind, None)?;
                    return Ok(RootPublicationState::Mined);
                }
                if let Some(receipt) = ethereum.api.get_finalized_receipt(transaction_hash).await? {
                    ensure!(
                        receipt.receipt.transaction_hash == transaction_hash
                            && receipt.receipt.from == signed.from
                            && receipt.receipt.to == signed.to
                            && receipt.receipt.status()
                            && receipt.included_block_number == block
                            && receipt.included_block_hash == hash
                            && receipt.receipt.block_number == Some(block)
                            && receipt.receipt.block_hash == Some(hash)
                            && receipt.finalized_block_number >= block,
                        "finalized root receipt differs from original canonical inclusion"
                    );
                    if let Some(anchor) = ethereum.api.get_finalized_receipt(anchor_tx).await? {
                        ensure!(
                            anchor.receipt.transaction_hash == anchor_tx
                                && anchor.receipt.status()
                                && anchor.receipt.block_number == Some(anchor_inclusion.0)
                                && anchor.receipt.block_hash == Some(anchor_inclusion.1)
                                && anchor.included_block_number == anchor_inclusion.0
                                && anchor.included_block_hash == anchor_inclusion.1
                                && anchor.finalized_block_number >= anchor_inclusion.0,
                            "finalized BEEFY anchor differs from pinned canonical inclusion"
                        );
                        ensure!(
                            provider
                                .get_block_by_number(alloy::rpc::types::BlockNumberOrTag::Number(
                                    anchor_inclusion.0
                                ))
                                .await?
                                .is_some_and(
                                    |canonical| canonical.header.hash == anchor_inclusion.1
                                ),
                            "pinned BEEFY anchor receipt block is no longer canonical"
                        );
                        if intent["publicationReceipt"]["finalized"] == true {
                            let saved_block = intent["publicationReceipt"]["finalizedBlock"]
                                .as_u64()
                                .context("original root finalized anchor height missing")?;
                            let saved_hash: B256 = intent["publicationReceipt"]
                                ["finalizedBlockHash"]
                                .as_str()
                                .context("original root finalized anchor hash missing")?
                                .parse()?;
                            ensure!(
                                ethereum
                                    .api
                                    .is_finalized_block(saved_block, saved_hash.0.into())
                                    .await?,
                                "original root finalized anchor changed; HOLD"
                            );
                        }
                        let proof = authenticated_root_receipt(
                            ethereum,
                            output,
                            transaction_hash,
                            (block, hash),
                            (receipt.finalized_block_number, receipt.finalized_block_hash),
                        )
                        .await?;
                        ensure!(
                            authenticated_publication_matches(
                                &proof,
                                ethereum.queue_address,
                                source_block,
                                kind,
                                expected_root
                            ) && format!("0x{}", hex::encode(&proof.raw_transaction))
                                == intent["rawTransaction"],
                            "authenticated original root receipt changed signed bytes; HOLD"
                        );
                        persist_root_inclusion(
                            output,
                            intent,
                            (block, hash),
                            kind,
                            Some((receipt.finalized_block_number, receipt.finalized_block_hash)),
                        )?;
                        if intent["publicationReceipt"]["receiptProof"].is_null() {
                            intent["publicationReceipt"]["receiptProof"] =
                                serde_json::to_value(&proof)?;
                            persist_root_intent(output, intent)?;
                        }
                        return Ok(RootPublicationState::Accepted);
                    }
                }
                return Ok(RootPublicationState::Mined);
            }
            // A later accepted checkpoint can stale the signed proof, but never
            // releases its nonce or authorizes a replacement. Reconcile/rebroadcast it unchanged.
            intent["finalityStatus"] = json!("pending");
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
    .await;
    match observation {
        Ok(Ok(state)) => Ok(state),
        Ok(Err(error))
            if !matches!(
                error.downcast_ref::<ethereum_client::Error>(),
                Some(ethereum_client::Error::FinalizedAncestryPending)
            ) =>
        {
            Err(error)
        }
        _ => hold_root_reconciliation(output, intent),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn publish_root(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    deployment_manifest: &Path,
    publisher_state: &Path,
    registration: &Path,
    output: &Path,
    owned_fixture_network: Option<&Path>,
    public_network: crate::ethereum::PublicEthereumNetwork,
) -> Result<()> {
    let _owner = lock_token_deployment(deployment_manifest)?;
    let manifest: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    let fixture = crate::ethereum::approved_fixture_network(
        owned_fixture_network, &manifest, source_rpc, witness_rpc, ethereum_rpc, public_network)?;
    let registration: Value = serde_json::from_slice(&fs::read(registration)?)?;
    loop {
        if publish_root_locked(
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            wallet,
            deployment_manifest,
            publisher_state,
            &registration,
            output,
            fixture.as_ref(),
            public_network,
        )
        .await?
            == RootPublicationState::Accepted
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

// Caller owns the deployment lock across anchor selection and publication.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn publish_root_locked(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    deployment_manifest: &Path,
    publisher_state: &Path,
    registration: &Value,
    output: &Path,
    fixture: Option<&crate::ethereum::OwnedFixtureNetwork>,
    public_network: crate::ethereum::PublicEthereumNetwork,
) -> Result<RootPublicationState> {
    use crate::{encode_queue_proof, ethereum::Ethereum, evidence::proof_evidence, source::Source};
    use alloy::{eips::Encodable2718, primitives::B256, providers::Provider};
    use anyhow::Context;
    use gear_rpc_client::GearApi as SourceApi;
    use gsdk::ext::subxt::utils::H256;

    ensure!(
        !source_rpc.is_empty() && !witness_rpc.is_empty(),
        "source and witness RPCs are required"
    );
    ensure!(
        source_rpc != witness_rpc,
        "independent Gear authorities required"
    );
    let manifest: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    crate::ethereum::validate_fixture_approval(fixture, &manifest, source_rpc, witness_rpc, ethereum_rpc, public_network)?;
    ensure!(
        manifest["mode"] == public_network.deployment_mode(),
        "token deployment differs from selected public network"
    );
    let block: u32 = registration["block"]
        .as_u64()
        .ok_or_else(|| anyhow!("registration block missing"))?
        .try_into()?;
    let kind = registration["kind"].as_str().unwrap_or("merkleRoot");
    ensure!(
        matches!(kind, "merkleRoot" | "emptyProgress"),
        "unknown queue progress registration kind"
    );
    let root = decode_hash(&registration["queueRoot"], "registration root")?;
    ensure!(
        if kind == "emptyProgress" {
            root == [0; 32]
        } else {
            root != [0; 32]
        },
        "registration root does not match its progress kind"
    );
    let root_text = format!("0x{}", hex::encode(root));
    let mut existing_journal: Value = serde_json::from_slice(&fs::read(publisher_state)?)?;
    crate::follower::validate_token_follower_journal(&existing_journal, &manifest)?;
    crate::follower::migrate_commitment_journal(&mut existing_journal)?;
    let mut matches = existing_journal["roots"]
        .as_object()
        .context("root publication inventory missing; HOLD")?
        .values()
        .filter(|saved| {
            saved["block"] == block
                && saved["queueRoot"] == root_text
                && saved["kind"].as_str().unwrap_or("merkleRoot") == kind
        });
    let registered = matches
        .next()
        .context("publication has no original follower registration; HOLD")?;
    ensure!(
        matches.next().is_none(),
        "publication has ambiguous follower registrations; HOLD"
    );
    let registered_path = Path::new(
        registered["publication"]
            .as_str()
            .context("registered publication path missing; HOLD")?,
    );
    ensure!(
        output == registered_path
            && output.parent()
                == Some(
                    state_directory(publisher_state)
                        .join("root-publications")
                        .as_path()
                ),
        "publication output differs from its owned original registration; HOLD"
    );
    let local_rehearsal = existing_journal["localRehearsal"]
        .as_bool()
        .context("actor rehearsal mode missing")?;
    ensure!(
        manifest["localRehearsal"] == local_rehearsal,
        "deployment rehearsal mode differs from actor"
    );
    ensure!(
        if local_rehearsal || fixture.is_some() {
            crate::local_source_rpc(ethereum_rpc)
        } else {
            ethereum_rpc.starts_with("wss://") || ethereum_rpc.starts_with("https://")
        },
        "Public Ethereum requires HTTPS or WSS; local rehearsal requires a loopback websocket"
    );
    fs::create_dir_all(state_directory(output))?;
    ensure!(
        !output.exists() || output.is_file(),
        "publication evidence path is not a file"
    );
    let mut prior = if output.exists() {
        let intent: Value = serde_json::from_slice(&fs::read(output)?)?;
        ensure!(
            matches!(intent["schemaVersion"].as_u64(), Some(3 | 4))
                && intent["sourceBlock"] == block
                && intent["root"] == root_text
                && intent["kind"].as_str().unwrap_or("merkleRoot") == kind,
            "publication intent is incompatible with this root or schema"
        );
        Some(intent)
    } else {
        None
    };

    let active_ethereum = if existing_journal["activeEthereum"].is_object() {
        existing_journal["activeEthereum"].clone()
    } else {
        manifest["ethereum"].clone()
    };
    ensure!(
        active_ethereum["chainId"] == manifest["ethereum"]["chainId"]
            && active_ethereum["queue"] == manifest["ethereum"]["queue"]
            && active_ethereum["receiver"] == manifest["ethereum"]["receiver"]
            && active_ethereum["sourceDomain"] == manifest["ethereum"]["sourceDomain"]
            && active_ethereum["bridgeDomain"] == manifest["ethereum"]["bridgeDomain"]
            && active_ethereum["mmrStartBlock"] == manifest["ethereum"]["mmrStartBlock"],
        "actor active BEEFY identity differs from the immutable token deployment"
    );
    let ethereum = Ethereum::connect(ethereum_rpc, wallet, &active_ethereum, fixture, public_network).await?;
    ethereum
        .verify_token_bindings(decode_hash(&manifest["gearManager"], "gearManager")?)
        .await?;
    let archived_this_call = if let Some(intent) = prior.as_mut() {
        validate_root_publication(&ethereum, output, intent).await?;
        reconcile_root_original(&ethereum, output, intent).await?
    } else {
        false
    };
    let publisher_signer = crate::ethereum::hoodi_wallet_address(wallet)?;
    let publisher_address = format!("{publisher_signer:#x}");

    let (source, witness) = Source::connect_pair(
        SourceApi::new(source_rpc, 3).await?,
        SourceApi::new(witness_rpc, 3).await?,
    )
    .await?;
    source.validate_attachment(&manifest["anchor"]).await?;
    ensure_source_identity(
        &manifest["ethereum"],
        source.source_genesis,
        source.bridge_domain,
        witness.source_genesis,
        witness.bridge_domain,
    )?;
    ensure!(
        manifest["anchor"]["sourceGenesis"] == format!("0x{}", hex::encode(source.source_genesis))
            && manifest["anchor"]["bridgeDomain"]
                == format!("0x{}", hex::encode(source.bridge_domain)),
        "token deployment anchor differs from current source identity"
    );

    let active_client = active_ethereum["client"]
        .as_str()
        .context("active BEEFY client address missing")?;
    let prior_anchor = prior
        .as_ref()
        .filter(|intent| !intent["txHash"].is_null())
        .map(|intent| {
            ensure!(
                intent["acceptedAnchorClient"] == active_client,
                "saved root client differs from active deployment; hold original intent"
            );
            intent["acceptedAnchorTx"]
                .as_str()
                .context("saved root anchor transaction is missing")
        })
        .transpose()?;
    let mut journal = crate::follower::read_state_file(publisher_state)?;
    crate::follower::validate_token_follower_journal(&journal, &manifest)?;
    crate::follower::migrate_commitment_journal(&mut journal)?;
    let follower_signer = journal["followerSigner"]
        .as_str()
        .context("token follower signer is missing")?
        .parse()?;
    ensure_separate_evm_signers(follower_signer, publisher_signer)?;
    ensure!(
        journal["rootPublisherSigner"] == publisher_address,
        "root publisher signer differs from actor journal"
    );
    ensure!(
        journal["follower"]["status"] != "failed",
        "token actor is failed; reconcile it before maintenance publication"
    );
    // Independent signers may reconcile their originals concurrently. Only the
    // latest accepted checkpoint check immediately before a new signature gates preparation.
    let commitments = journal["commitments"]
        .as_array()
        .context("token follower journal has no accepted commitments")?;
    let legacy_client = manifest["ethereum"]["client"]
        .as_str()
        .context("immutable deployment client is missing")?
        .parse()?;
    let selected_client: Address = active_client.parse()?;
    let mut selected = None;
    for entry in commitments.iter().rev() {
        if crate::follower::journal_client(entry, legacy_client)? == selected_client
            && prior_anchor.is_none_or(|tx_hash| {
                crate::follower::commitment_transaction(entry)
                    .is_ok_and(|transaction| transaction["txHash"].as_str() == Some(tx_hash))
            })
        {
            selected = Some(entry);
            break;
        }
    }
    let last = selected.context("publication anchor is absent from active commitment history")?;
    ensure!(
        last["block"]
            .as_u64()
            .context("accepted anchor block missing")?
            > u64::from(block),
        "actor must accept a covering anchor before root publication"
    );

    let anchor_block: u32 = last["block"]
        .as_u64()
        .context("accepted anchor block missing")?
        .try_into()?;
    let raw = hex::decode(
        last["rawScale"]
            .as_str()
            .context("accepted signed commitment missing")?
            .trim_start_matches("0x"),
    )?;
    let (anchor, second_anchor) = tokio::try_join!(
        source.recapture(anchor_block, raw.clone()),
        witness.recapture(anchor_block, raw.clone()),
    )?;
    let source_finalized = source
        .api
        .block_hash_to_number(source.api.latest_finalized_block().await?)
        .await?;
    let witness_finalized = witness
        .api
        .block_hash_to_number(witness.api.latest_finalized_block().await?)
        .await?;
    ensure!(
        source_finalized >= anchor_block && witness_finalized >= anchor_block,
        "accepted source anchor is not finalized by both Gear authorities"
    );
    ensure!(
        source.api.block_number_to_hash(anchor_block).await? == H256(anchor.block_hash)
            && witness.api.block_number_to_hash(anchor_block).await? == H256(anchor.block_hash),
        "accepted source anchor was reorged at one of the Gear authorities"
    );
    let recorded_hash = decode_hash(&last["blockHash"], "accepted anchor block hash")?;
    ensure_recaptured_anchor(
        block,
        (
            anchor_block,
            recorded_hash,
            last["rawScale"]
                .as_str()
                .context("accepted commitment SCALE missing")?,
        ),
        (anchor.block, anchor.block_hash, &anchor.raw),
        (
            second_anchor.block,
            second_anchor.block_hash,
            &second_anchor.raw,
        ),
        &raw,
    )?;
    let (proof, second) = tokio::try_join!(
        source.proof(block, &anchor),
        witness.proof(block, &second_anchor)
    )?;
    ensure!(
        proof.snapshot == second.snapshot
            && proof.raw_leaf == second.raw_leaf
            && proof.source_hash == second.source_hash
            && proof.snapshot.bridge_domain == source.bridge_domain
            && registration["blockHash"] == format!("0x{}", hex::encode(proof.source_hash))
            && registration["queueId"] == proof.snapshot.queue_id
            && proof.snapshot.initialized
            && proof.snapshot.queue_root == root
            && proof.leaf.leaf_extra == proof.snapshot.hash(),
        "historical registration root or independently witnessed MMR proof differs"
    );
    let accepted_tx = decode_hash(
        &crate::follower::commitment_transaction(last)?["txHash"],
        "accepted anchor transaction hash",
    )?;
    let accepted_anchor_tx = format!("0x{}", hex::encode(accepted_tx));
    let anchor_proof;
    let anchor_snapshot = if block == anchor.block - 1 {
        &proof.snapshot
    } else {
        let (first, second) = tokio::try_join!(
            source.proof(anchor.block - 1, &anchor),
            witness.proof(anchor.block - 1, &second_anchor)
        )?;
        ensure!(
            first.snapshot == second.snapshot
                && first.raw_leaf == second.raw_leaf
                && first.source_hash == second.source_hash,
            "accepted checkpoint snapshot differs between authorities"
        );
        anchor_proof = first;
        &anchor_proof.snapshot
    };
    let (destination_block, destination_hash) = ethereum
        .verify_accepted_commitment(B256::from(accepted_tx), &anchor, anchor_snapshot)
        .await?;
    let destination_hash_text = format!("{destination_hash:#x}");
    ensure!(
        last["destinationBlock"].as_u64() == Some(destination_block)
            && last["destinationHash"].as_str() == Some(destination_hash_text.as_str()),
        "accepted Hoodi checkpoint is noncanonical or differs from follower journal"
    );
    let encoded = encode_queue_proof(
        &proof.snapshot,
        u64::from(anchor.block),
        anchor.validated.mmr_root,
        &proof.leaf,
        &proof.simplified.items,
        proof.simplified.proof_order,
    )?;
    let proof_record = proof_evidence(&proof, &anchor, &encoded);
    let immutable_intent = json!({
        "schemaVersion": 4,
        "sourceBlock": block,
        "root": root_text,
        "kind": kind,
        "localRehearsal": journal["localRehearsal"],
        "ownedFixtureNetwork": manifest["ownedFixtureNetwork"],
        "sourceIdentity": {
            "sourceGenesis": manifest["ethereum"]["sourceGenesis"],
            "sourceDomain": manifest["ethereum"]["sourceDomain"],
            "bridgeDomain": manifest["ethereum"]["bridgeDomain"],
            "destinationChainId": manifest["ethereum"]["chainId"],
            "destinationQueue": manifest["ethereum"]["queue"],
        },
        "acceptedAnchorTx": accepted_anchor_tx,
        "acceptedAnchorClient": active_client,
        "acceptedCheckpoint": {
            "block": destination_block,
            "blockHash": format!("{destination_hash:#x}"),
        },
        "sender": publisher_address,
        "proof": proof_record,
        "queueProof": format!("0x{}", hex::encode(&encoded)),
    });
    let mut intent = match prior {
        Some(mut saved) => {
            // Promote legacy only after original identity, receipt, source/witness
            // and independently recaptured proof checks above.
            saved["schemaVersion"] = json!(4);
            refresh_unsigned_intent(&saved, &immutable_intent)?
        }
        None => {
            let mut intent = immutable_intent;
            intent["nonce"] = Value::Null;
            intent["txHash"] = Value::Null;
            intent["rawTransaction"] = Value::Null;
            intent["status"] = json!("prepared");
            intent
        }
    };
    ensure!(
        intent.get("nonce").is_some()
            && intent.get("txHash").is_some()
            && intent.get("rawTransaction").is_some(),
        "publication intent is missing durable nonce or transaction hash fields"
    );
    ensure!(
        intent["txHash"].is_null() == intent["rawTransaction"].is_null()
            && (intent["txHash"].is_null() || !intent["nonce"].is_null()),
        "saved root transaction hash has no reserved nonce"
    );
    ensure!(
        intent["txHash"].is_null() || intent["txHash"].as_str().is_some(),
        "saved root transaction hash is invalid"
    );
    persist_root_intent(output, &intent)?;

    validate_root_publication(&ethereum, output, &intent).await?;
    let observed =
        observe_finalized_progress(&ethereum, output, &mut intent, block, kind, root).await?;
    if intent["txHash"].is_null() && observed {
        return Ok(RootPublicationState::Observed);
    }
    if archived_this_call || intent["originalDisposition"] == "nonceConsumed" {
        // Consumed nonce without original receipt is held, not a fresh signing permit.
        return hold_root_reconciliation(output, &mut intent);
    }
    let mined = if intent["txHash"].is_null() {
        None
    } else {
        canonical_root_inclusion(&ethereum, &intent, block, kind, root).await?
    };

    let saved_nonce = if intent["nonce"].is_null() {
        let (latest, pending) = root_account_nonces(&ethereum, publisher_signer).await?;
        ensure!(
            latest == pending,
            "publisher account has an outstanding transaction; reconcile it before root publication"
        );
        intent["nonce"] = json!(pending.to_string());
        persist_root_intent(output, &intent)?;
        pending
    } else {
        saved_publication_nonce(&intent)?
    };
    let provider = ethereum.api.raw_provider();
    if intent["txHash"].is_null() {
        ensure_pinned_latest_anchor(
            &ethereum.checkpoint().await?,
            anchor_block,
            anchor.validated.mmr_root,
        )?;
        let (latest, pending) = root_account_nonces(&ethereum, publisher_signer).await?;
        ensure_saved_nonce_available(saved_nonce, latest, pending)?;
        let request = if kind == "emptyProgress" {
            ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, provider.clone())
                .submitEmptyQueueProgress(alloy::primitives::U256::from(block), encoded.into())
                .nonce(saved_nonce)
                .into_transaction_request()
        } else {
            ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, provider.clone())
                .submitMerkleRoot(
                    alloy::primitives::U256::from(block),
                    B256::from(root),
                    encoded.into(),
                )
                .nonce(saved_nonce)
                .into_transaction_request()
        };
        let filled = provider
            .fill(request)
            .await
            .context("sign original queue-root transaction")?;
        let raw = filled
            .as_envelope()
            .context("root transaction was not signed")?
            .encoded_2718();
        intent["rawTransaction"] = json!(format!("0x{}", hex::encode(&raw)));
        intent["txHash"] = json!(format!("0x{}", hex::encode(crate::keccak256(&raw))));
        intent["status"] =
            json!("signed original root transaction; broadcast outcome not yet known");
        persist_root_intent(output, &intent)?;
    }
    let signed = signed_publication_identity(&intent, ethereum.queue_address)?;
    let transaction_hash = signed.hash;
    ensure!(
        signed.from == publisher_signer && signed.nonce == saved_nonce,
        "saved root publisher or reserved nonce differs from signed intent"
    );
    let raw = hex::decode(
        intent["rawTransaction"]
            .as_str()
            .context("saved signed root transaction missing")?
            .trim_start_matches("0x"),
    )?;
    if mined.is_none() {
        if let Some(observed) =
            ethereum_client::transaction_identity(provider, transaction_hash).await?
        {
            crate::ethereum::ensure_same_submission(observed, signed)?;
        } else {
            let (latest, pending) = root_account_nonces(&ethereum, publisher_signer).await?;
            ensure_saved_nonce_available(saved_nonce, latest, pending)?;
            // Recovery can only rebroadcast these identical signed bytes, never replace the transaction.
            match provider.send_raw_transaction(&raw).await {
                Ok(transaction) => ensure!(
                    *transaction.tx_hash() == transaction_hash,
                    "RPC returned a different root transaction hash"
                ),
                Err(_) => {
                    intent["lastError"] = json!(
                        "send API returned an error; reconcile the original signed hash and nonce"
                    );
                    persist_root_intent(output, &intent)?;
                }
            }
        }
        intent["status"] =
            json!("broadcast outcome pending; reconcile the saved signed transaction");
        persist_root_intent(output, &intent)?;
    }
    let finalized = wait_for_root_receipt(
        &ethereum,
        output,
        &mut intent,
        block,
        kind,
        root,
        (
            anchor_block,
            anchor.validated.mmr_root,
            B256::from(accepted_tx),
            (destination_block, destination_hash),
        ),
    )
    .await?;
    if finalized == RootPublicationState::Accepted {
        println!("accepted token queue root at Gear {block}, Hoodi tx {transaction_hash:#x}");
    }
    Ok(finalized)
}
#[cfg(test)]
mod root_publication_tests {
    use super::*;

    #[tokio::test]
    async fn mined_root_precedes_acceptance_and_keeps_the_original_signed_intent() -> Result<()> {
        use alloy::{
            consensus::{SignableTransaction, TxEip1559, TxEnvelope},
            eips::Encodable2718,
            network::TxSigner,
            primitives::TxKind,
            signers::local::PrivateKeySigner,
            sol_types::SolCall,
        };
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("root.json");
        let queue = Address::from([4; 20]);
        let wallet = PrivateKeySigner::from_bytes(&B256::from([7; 32]))?;
        let root = B256::from([9; 32]);
        let input = ethereum_client::abi::IMessageQueue::submitMerkleRootCall {
            blockNumber: U256::from(21),
            merkleRoot: root,
            proof: vec![1, 2].into(),
        }
        .abi_encode();
        let mut transaction = TxEip1559 {
            input: input.into(),
            chain_id: 560_048,
            nonce: 4,
            gas_limit: 21_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(queue),
            ..Default::default()
        };
        let signature = wallet.sign_transaction(&mut transaction).await?;
        let raw = TxEnvelope::Eip1559(transaction.into_signed(signature)).encoded_2718();
        let hash = alloy::primitives::keccak256(&raw);
        let mut intent = json!({
            "schemaVersion":4,"root":format!("{root:#x}"),"kind":"merkleRoot",
            "sourceIdentity":{"destinationChainId":560_048},
            "sourceBlock": 21, "acceptedAnchorTx": "0xanchor", "proof": {"anchorBlock": 25},
            "queueProof": "0x0102", "nonce": "4", "sender": wallet.address(),
            "rawTransaction": format!("0x{}", hex::encode(&raw)),
            "txHash": format!("{hash:#x}"),
        });
        let signed = intent.clone();
        let mut legacy = signed.clone();
        legacy["schemaVersion"] = json!(3);
        validate_root_intent(&legacy, queue)?;
        legacy["schemaVersion"] = json!(4);
        let mut expected = signed.clone();
        for field in ["nonce", "txHash", "rawTransaction"] {
            expected.as_object_mut().unwrap().remove(field);
        }
        let migrated = refresh_unsigned_intent(&legacy, &expected)?;
        assert_eq!(migrated["txHash"], signed["txHash"]);
        assert_eq!(migrated["rawTransaction"], signed["rawTransaction"]);
        assert_eq!(migrated["nonce"], "4");
        let mut ambiguous_legacy = legacy.clone();
        ambiguous_legacy["schemaVersion"] = json!(3);
        ambiguous_legacy["observedProgress"] = json!({});
        assert!(validate_root_intent(&ambiguous_legacy, queue).is_err());
        assert_eq!(
            hold_root_reconciliation(&path, &mut intent)?,
            RootPublicationState::Pending
        );
        let held: Value = serde_json::from_slice(&fs::read(&path)?)?;
        for field in [
            "nonce",
            "txHash",
            "rawTransaction",
            "acceptedAnchorTx",
            "proof",
            "queueProof",
        ] {
            assert_eq!(held[field], signed[field]);
        }
        assert_eq!(signed_publication_identity(&held, queue)?.hash, hash);
        let inclusion = (41, B256::from([1; 32]));
        persist_root_inclusion(&path, &mut intent, inclusion, "merkleRoot", None)?;
        let mined: Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(mined["status"], "mined");
        assert_eq!(mined["publicationReceipt"]["finalized"], false);
        for field in [
            "nonce",
            "txHash",
            "rawTransaction",
            "acceptedAnchorTx",
            "proof",
            "queueProof",
        ] {
            assert_eq!(mined[field], signed[field]);
        }
        assert_eq!(signed_publication_identity(&mined, queue)?.hash, hash);
        assert_eq!(
            hold_root_reconciliation(&path, &mut intent)?,
            RootPublicationState::Mined
        );
        assert_eq!(intent["publicationReceipt"], mined["publicationReceipt"]);
        let finalized = (48, B256::from([9; 32]));
        persist_root_inclusion(&path, &mut intent, inclusion, "merkleRoot", Some(finalized))?;
        let accepted: Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(accepted["status"], "accepted");
        assert_eq!(accepted["finalityStatus"], "finalized");
        assert_eq!(accepted["publicationReceipt"]["finalized"], true);
        assert_eq!(accepted["publicationReceipt"]["block"], inclusion.0);
        assert_eq!(
            accepted["publicationReceipt"]["blockHash"],
            format!("{:#x}", inclusion.1)
        );
        assert_eq!(
            accepted["publicationReceipt"]["finalizedBlock"],
            finalized.0
        );
        assert_eq!(signed_publication_identity(&accepted, queue)?.hash, hash);
        for (field, replacement) in [
            ("sender", json!(Address::from([5; 20]))),
            ("nonce", json!("5")),
            ("txHash", json!(B256::ZERO)),
        ] {
            let mut changed = accepted.clone();
            changed[field] = replacement;
            assert!(
                signed_publication_identity(&changed, queue).is_err(),
                "{field}"
            );
        }
        assert!(signed_publication_identity(&accepted, Address::from([6; 20])).is_err());
        assert!(persist_root_inclusion(&path, &mut intent, inclusion, "merkleRoot", None).is_err());
        let original_finalized = intent["publicationReceipt"].clone();
        persist_root_inclusion(
            &path,
            &mut intent,
            inclusion,
            "merkleRoot",
            Some((60, B256::repeat_byte(8))),
        )?;
        assert_eq!(intent["publicationReceipt"], original_finalized);
        assert!(persist_root_inclusion(
            &path,
            &mut intent,
            (42, inclusion.1),
            "merkleRoot",
            Some(finalized)
        )
        .is_err());
        for (field, changed) in [
            ("queueProof", json!("0x0304")),
            ("root", json!(B256::ZERO)),
            ("sourceBlock", json!(22)),
            ("sourceIdentity", json!({"destinationChainId":31337})),
        ] {
            let mut changed_intent = accepted.clone();
            changed_intent[field] = changed;
            assert!(
                signed_publication_identity(&changed_intent, queue).is_err(),
                "{field}"
            );
        }
        // A competitor is separately recorded while the original retains ownership.
        let mut competing = signed.clone();
        competing["observedProgress"] = json!({"actualTransactionHash":B256::repeat_byte(8)});
        assert_eq!(signed_publication_identity(&competing, queue)?.hash, hash);
        competing["status"] = json!("observed");
        assert!(validate_root_intent(&competing, queue).is_err());
        for disposition in ["reverted", "nonceConsumed"] {
            let mut terminal = signed.clone();
            archive_root_original(
                &mut terminal,
                disposition,
                json!({"authenticatedEvidence":"policy fixture"}),
            )?;
            assert!(
                terminal["txHash"].is_null()
                    && terminal["rawTransaction"].is_null()
                    && terminal["nonce"].is_null()
                    && terminal["publicationReceipt"].is_null()
            );
            assert_eq!(terminal["terminalSubmissions"][0]["original"], signed);
            assert_eq!(terminal["originalDisposition"], disposition);
            validate_root_intent(&terminal, queue)?;
            let mut changed_terminal = terminal.clone();
            changed_terminal["terminalSubmissions"][0]["original"]["queueProof"] = json!("0x03");
            assert!(validate_root_intent(&changed_terminal, queue).is_err());
        }
        Ok(())
    }

    #[test]
    fn signed_root_anchor_or_proof_cannot_change_on_resume() {
        let expected = json!({
            "schemaVersion": 3, "sourceBlock": 21, "root": "0xroot", "kind": "merkleRoot",
            "localRehearsal": false, "sourceIdentity": {"genesisHash": "0xgenesis"},
            "acceptedAnchorTx": "0xoriginal", "acceptedAnchorClient": "0xclient",
            "acceptedCheckpoint": {"block": 25}, "sender": "0xpublisher",
            "proof": {"anchorBlock": 25}, "queueProof": "0xproof",
        });
        let mut saved = expected.clone();
        saved["nonce"] = json!("4");
        saved["rawTransaction"] = json!("0xsigned");
        saved["txHash"] = json!("0xoriginal-hash");
        assert!(ensure_immutable_publication(&saved, &expected).is_ok());
        for (field, changed) in [
            ("acceptedAnchorTx", json!("0xreanchored")),
            ("acceptedCheckpoint", json!({"block": 26})),
            ("proof", json!({"anchorBlock": 26})),
            ("queueProof", json!("0xreplaced")),
        ] {
            let mut recaptured = expected.clone();
            recaptured[field] = changed;
            assert!(
                ensure_immutable_publication(&saved, &recaptured).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn actor_and_maintenance_share_deployment_owner_across_journal_paths() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("deployment.json");
        let second = directory.path().join("same-deployment.json");
        let manifest = br#"{"ethereum":{"chainId":560048,"queue":"0x1111111111111111111111111111111111111111"}}"#;
        fs::write(&first, manifest)?;
        fs::write(&second, manifest)?;
        let owner = lock_token_deployment(&first)?;
        assert!(lock_token_deployment(&second).is_err());
        drop(owner);
        let maintenance = lock_token_deployment(&second)?;
        assert!(lock_token_deployment(&first).is_err());
        drop(maintenance);
        assert!(lock_token_deployment(&first).is_ok());
        Ok(())
    }

    #[test]
    fn unsigned_refresh_preserves_reserved_nonce_and_terminal_history() -> Result<()> {
        let mut saved = json!({"schemaVersion":4,"sourceBlock":21,"root":"0xroot","kind":"merkleRoot",
            "localRehearsal":true,"sourceIdentity":{"genesis":"unchanged"},"sender":"publisher",
            "acceptedAnchorTx":"old","acceptedCheckpoint":{"block":420},"proof":{"anchorBlock":420},
            "queueProof":"old-proof","nonce":"4","txHash":null,"rawTransaction":null,
            "terminalSubmissions":[{"original":"retained"}]});
        let mut next = saved.clone();
        next["acceptedAnchorTx"] = json!("new");
        next["acceptedCheckpoint"] = json!({"block":430});
        next["proof"] = json!({"anchorBlock":430});
        next["queueProof"] = json!("new-proof");
        let refreshed = refresh_unsigned_intent(&saved, &next)?;
        assert_eq!(refreshed["nonce"], "4");
        assert_eq!(
            refreshed["terminalSubmissions"],
            saved["terminalSubmissions"]
        );
        assert_eq!(refreshed["proof"], next["proof"]);
        assert_eq!(refreshed["queueProof"], "new-proof");
        saved["txHash"] = json!("original-hash");
        saved["rawTransaction"] = json!("original-bytes");
        assert!(refresh_unsigned_intent(&saved, &next).is_err());
        next["root"] = json!("changed-root");
        saved["txHash"] = Value::Null;
        assert!(refresh_unsigned_intent(&saved, &next).is_err());
        Ok(())
    }

    #[test]
    fn monotonic_progress_only_covers_empty_targets_not_historical_nonzero_roots() {
        let root = [7; 32];
        assert!(matching_progress(
            57_601,
            "emptyProgress",
            [0; 32],
            ("merkleRoot", 60_000, root)
        ));
        assert!(matching_progress(
            57_601,
            "emptyProgress",
            [0; 32],
            ("emptyProgress", 60_000, [0; 32])
        ));
        assert!(!matching_progress(
            57_601,
            "emptyProgress",
            [0; 32],
            ("merkleRoot", 57_600, root)
        ));
        for target in [41, 43] {
            assert!(!matching_progress(
                target,
                "merkleRoot",
                root,
                ("merkleRoot", 99, root)
            ));
            assert!(!matching_progress(
                target,
                "merkleRoot",
                root,
                ("emptyProgress", 99, [0; 32])
            ));
            assert!(matching_progress(
                target,
                "merkleRoot",
                root,
                ("merkleRoot", target, root)
            ));
            assert!(!matching_progress(
                target,
                "merkleRoot",
                root,
                ("merkleRoot", target, [8; 32])
            ));
        }
    }

    #[test]
    fn authenticated_queue_event_policy_rejects_changed_height_root_and_address() -> Result<()> {
        let queue = Address::repeat_byte(4);
        let root = [7; 32];
        let mut data = U256::from(41).to_be_bytes::<32>().to_vec();
        data.extend_from_slice(&root);
        let signature = B256::from(crate::keccak256(b"MerkleRoot(uint256,bytes32)"));
        let log =
            alloy::primitives::Log::new_unchecked(queue, vec![signature], data.clone().into());
        assert_eq!(
            queue_progress_event(&log, queue)?,
            Some(("merkleRoot", 41, root))
        );
        assert!(queue_progress_event(&log, Address::repeat_byte(5))?.is_none());
        let malformed =
            alloy::primitives::Log::new_unchecked(queue, vec![signature], vec![0; 63].into());
        assert!(queue_progress_event(&malformed, queue).is_err());
        let empty = alloy::primitives::Log::new_unchecked(
            queue,
            vec![
                B256::from(crate::keccak256(b"EmptyQueueProgress(uint256)")),
                B256::from(U256::from(60_000).to_be_bytes::<32>()),
            ],
            vec![].into(),
        );
        assert_eq!(
            queue_progress_event(&empty, queue)?,
            Some(("emptyProgress", 60_000, [0; 32]))
        );
        Ok(())
    }

    #[test]
    fn publisher_uses_a_distinct_follower_account() {
        let follower = alloy::primitives::Address::from([1; 20]);
        let publisher = alloy::primitives::Address::from([2; 20]);
        assert!(ensure_separate_evm_signers(follower, publisher).is_ok());
        assert!(ensure_separate_evm_signers(follower, follower).is_err());
    }

    #[test]
    fn publisher_rejects_changed_genesis_or_bridge_domain() {
        let genesis = [1; 32];
        let domain = [2; 32];
        let manifest = json!({
            "sourceGenesis": format!("0x{}", hex::encode(genesis)),
            "bridgeDomain": format!("0x{}", hex::encode(domain)),
        });
        assert!(ensure_source_identity(&manifest, genesis, domain, genesis, domain).is_ok());
        assert!(ensure_source_identity(&manifest, [3; 32], domain, [3; 32], domain).is_err());
        assert!(ensure_source_identity(&manifest, genesis, domain, genesis, [3; 32]).is_err());
        let wrong_genesis = json!({
            "sourceGenesis": format!("0x{}", hex::encode([3; 32])),
            "bridgeDomain": manifest["bridgeDomain"],
        });
        assert!(ensure_source_identity(&wrong_genesis, genesis, domain, genesis, domain).is_err());
        let wrong_domain = json!({
            "sourceGenesis": manifest["sourceGenesis"],
            "bridgeDomain": format!("0x{}", hex::encode([3; 32])),
        });
        assert!(ensure_source_identity(&wrong_domain, genesis, domain, genesis, domain).is_err());
    }

    #[test]
    fn reorged_or_changed_source_anchor_is_rejected() {
        let raw = [4, 5, 6];
        let hash = [8; 32];
        let raw_text = format!("0x{}", hex::encode(raw));
        assert!(ensure_recaptured_anchor(
            20,
            (25, hash, &raw_text),
            (25, hash, &raw),
            (25, hash, &raw),
            &raw,
        )
        .is_ok());
        assert!(ensure_recaptured_anchor(
            20,
            (25, hash, &raw_text),
            (25, hash, &raw),
            (25, [9; 32], &raw),
            &raw,
        )
        .is_err());
        assert!(ensure_recaptured_anchor(
            20,
            (25, hash, &raw_text),
            (25, hash, &raw),
            (25, hash, &raw),
            &[1, 2, 3],
        )
        .is_err());
    }
    #[test]
    fn provisional_receipts_and_reorgs_do_not_complete() {
        let hash = B256::from([1; 32]);
        assert_eq!(
            classify_receipt_finality(41, hash, true, 40, Some(hash)).unwrap(),
            ReceiptFinality::Mined
        );
        assert!(classify_receipt_finality(41, hash, true, 41, Some(B256::from([2; 32]))).is_err());
        assert!(classify_receipt_finality(41, hash, true, 41, None).is_err());
        assert_eq!(
            classify_receipt_finality(41, hash, true, 41, Some(hash)).unwrap(),
            ReceiptFinality::Accepted
        );
        assert_eq!(
            classify_receipt_finality(41, hash, false, 41, Some(hash)).unwrap(),
            ReceiptFinality::Reverted
        );
    }

    #[test]
    fn ambiguous_send_reconciles_only_the_reserved_nonce() {
        let intent = json!({
            "nonce": "4",
            "txHash": null,
            "status": "ambiguous send; reconcile root state and saved nonce before retry",
        });
        let nonce = saved_publication_nonce(&intent).unwrap();
        assert_eq!(nonce, 4);
        assert!(ensure_saved_nonce_available(nonce, 4, 4).is_ok());
        assert!(ensure_saved_nonce_available(nonce, 5, 5).is_err());
        assert!(ensure_saved_nonce_available(nonce, 4, 5).is_err());
        assert_eq!(intent["nonce"], "4");
        assert!(intent["txHash"].is_null());
    }
}
