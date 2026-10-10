use crate::{
    ethereum::{receipt_value, DestinationCheckpoint, Ethereum},
    rehearsal::{freshness_evidence, message_evidence, proof_evidence, queue_envelope},
    source::{self, AuthoritySet, CapturedCommitment, ObservedMessage, Source, SourceProof},
};
use alloy::{
    consensus::{Transaction, TxEnvelope},
    eips::Decodable2718,
    primitives::{Address, B256},
    providers::Provider,
    rpc::types::Filter,
};
use anyhow::{ensure, Context, Result};
use beefy_relay::{authority_addresses, authority_root, keccak256, Hash32, QueueSnapshot};
use futures::{stream, StreamExt};
use gear_rpc_client::{dto::Message, GearApi};
use serde_json::{json, Value};
use sp_core::Pair;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    time::Duration,
};
use subxt::utils::H256;

fn bytes(value: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(value))
}
fn decode(value: &Value) -> Result<Vec<u8>> {
    let text = value.as_str().context("missing hex evidence field")?;
    Ok(hex::decode(
        text.strip_prefix("0x").context("missing hex prefix")?,
    )?)
}
fn array<const N: usize>(value: &Value) -> Result<[u8; N]> {
    decode(value)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("wrong evidence byte length"))
}
pub(super) fn number(value: &Value) -> Result<u64> {
    value.as_u64().context("missing unsigned evidence field")
}
fn restore_message(value: &Value) -> Result<ObservedMessage> {
    let message = Message {
        nonce_be: array(&value["nonce"])?,
        source: array(&value["source"])?,
        destination: array(&value["destination"])?,
        payload: decode(&value["payload"])?,
    };
    let message_hash = array(&value["messageHash"])?;
    ensure!(
        crate::message_hash(&message) == message_hash,
        "saved message hash mismatch"
    );
    Ok(ObservedMessage {
        block: number(&value["sourceBlock"])?.try_into()?,
        block_hash: array(&value["sourceHash"])?,
        message,
        message_hash,
        inclusion: serde_json::from_value(value["messageProof"].clone())?,
        snapshot: QueueSnapshot::new(
            array(&value["bridgeDomain"])?,
            number(&value["sourceTimestampMs"])?,
            value["initialized"]
                .as_bool()
                .context("missing initialized flag")?,
            number(&value["queueId"])?,
            array(&value["queueRoot"])?,
        )?,
        retained_at_block: number(&value["retainedAtBlock"])?.try_into()?,
    })
}
// ponytail: rewrite the bounded demo journal; use a transactional store for a production relay.
fn lock_state(output: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(output.join("state.lock"))?;
    lock.lock()?;
    Ok(lock)
}

pub(crate) fn read_state_file(path: &Path) -> Result<Value> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let _lock = lock_state(directory)?;
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

pub(crate) fn validate_token_follower_journal(state: &Value, deployment: &Value) -> Result<()> {
    ensure!(
        state["schemaVersion"] == 3 && state["mode"] == "hoodi-token-follow",
        "unsupported token follower journal schema or mode"
    );
    ensure!(
        state.get("deployment") == Some(deployment),
        "token follower journal belongs to another deployment manifest"
    );
    ensure!(
        state["activeEthereum"] == deployment["ethereum"],
        "follower client differs from immutable deployment manifest"
    );
    Ok(())
}

fn save(output: &Path, state: &mut Value, ethereum: Option<&Ethereum>) -> Result<()> {
    let _lock = lock_state(output)?;
    if let Some(ethereum) = ethereum {
        state["transactions"] = json!(ethereum.transactions);
    }
    let temp = output.join("state.json.tmp");
    let mut file = std::io::BufWriter::new(File::create(&temp)?);
    serde_json::to_writer_pretty(&mut file, state)?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.get_ref().sync_all()?;
    fs::rename(temp, output.join("state.json"))?;
    File::open(output)?.sync_all()?;
    Ok(())
}
fn phase(output: &Path, state: &mut Value, ethereum: Option<&Ethereum>, name: &str) -> Result<()> {
    eprintln!("Hoodi: {name}");
    state["phase"] = json!(name);
    save(output, state, ethereum)
}
async fn bootstrap(
    source: &mut Source,
    witness: &mut Source,
    output: &Path,
    state: &mut Value,
) -> Result<(CapturedCommitment, QueueSnapshot)> {
    let target = source
        .api
        .block_hash_to_number(source.api.latest_finalized_block().await?)
        .await?;
    let anchor = loop {
        let anchor = source.next_commitment().await?;
        if anchor.block >= target && u64::from(anchor.block) > source.mmr_start_block {
            break anchor;
        }
    };
    let proof = source.proof(anchor.block - 1, &anchor).await?;
    let witness_finalized = witness
        .api
        .block_hash_to_number(witness.api.latest_finalized_block().await?)
        .await?;
    ensure!(
        witness_finalized >= anchor.block,
        "witness has not finalized bootstrap yet"
    );
    ensure!(
        witness.api.block_number_to_hash(anchor.block).await?.0 == anchor.block_hash,
        "witness finalized a different bootstrap block"
    );
    let (current, next) = witness.checkpoint_at_hash(H256(anchor.block_hash)).await?;
    let second = witness.proof(anchor.block - 1, &anchor).await?;
    ensure!(
        current == anchor.current
            && next == anchor.next
            && second.snapshot == proof.snapshot
            && second.raw_leaf == proof.raw_leaf,
        "bootstrap witnesses disagree"
    );
    state["bootstrap"] = json!({"block": anchor.block, "blockHash": bytes(anchor.block_hash), "rawScale": bytes(&anchor.raw), "current": anchor.current, "next": anchor.next, "freshnessProof": freshness_evidence(&proof)?});
    save(output, state, None)?;
    Ok((anchor, proof.snapshot))
}
async fn authenticated_token_anchor(
    source: &mut Source,
    witness: &mut Source,
    expected: &Value,
) -> Result<(CapturedCommitment, SourceProof)> {
    ensure!(
        expected["sourceGenesis"] == bytes(source.source_genesis)
            && expected["bridgeDomain"] == bytes(source.bridge_domain)
            && number(&expected["mmrStartBlock"])? == source.mmr_start_block
            && expected["beefyActivationBlock"] == source.beefy_activation_block,
        "bootstrap belongs to a different source domain or activation"
    );
    let anchor_block: u32 = number(&expected["block"])?.try_into()?;
    ensure!(
        u64::from(anchor_block) > source.mmr_start_block,
        "bootstrap does not cover a source MMR leaf"
    );
    let anchor = source
        .recapture(anchor_block, decode(&expected["signedCommitmentScale"])?)
        .await?;
    let witness_finalized = witness
        .api
        .block_hash_to_number(witness.api.latest_finalized_block().await?)
        .await?;
    ensure!(
        witness_finalized >= anchor_block,
        "witness has not finalized bootstrap"
    );
    ensure!(
        anchor.block_hash == array(&expected["blockHash"])?
            && anchor.validated.mmr_root == array(&expected["mmrRoot"])?
            && witness.api.block_number_to_hash(anchor_block).await?.0 == anchor.block_hash,
        "deployed bootstrap is not canonical on both Gear authorities"
    );
    let (current, next) = witness.checkpoint_at_hash(H256(anchor.block_hash)).await?;
    let first = source.proof(anchor_block - 1, &anchor).await?;
    let second = witness.proof(anchor_block - 1, &anchor).await?;
    ensure!(
        current == anchor.current
            && next == anchor.next
            && first.snapshot == second.snapshot
            && first.raw_leaf == second.raw_leaf
            && expected["current"] == serde_json::to_value(&anchor.current)?
            && expected["next"] == serde_json::to_value(&anchor.next)?
            && expected["freshnessSourceBlock"] == first.source
            && first.snapshot.source_timestamp_ms == number(&expected["sourceTimestampMs"])?,
        "deployed bootstrap proof disagrees between Gear authorities"
    );
    Ok((anchor, first))
}

fn bootstrap_record(anchor: &CapturedCommitment, proof: &SourceProof) -> Result<Value> {
    Ok(
        json!({"block":anchor.block, "blockHash":bytes(anchor.block_hash),
        "rawScale":bytes(&anchor.raw), "current":anchor.current, "next":anchor.next,
        "freshnessProof":freshness_evidence(proof)?}),
    )
}

fn client_bootstrap(state: &Value, client: Address) -> Result<&Value> {
    ensure!(
        client == deployment_client(state)?,
        "client differs from immutable deployment"
    );
    let bootstrap = &state["bootstrap"];
    ensure!(
        bootstrap.is_object(),
        "active client has no authenticated bootstrap journal"
    );
    Ok(bootstrap)
}

fn deployment_client(state: &Value) -> Result<Address> {
    let ethereum = if state["mode"] == "hoodi-message-demo" {
        &state["ethereum"]
    } else {
        &state["deployment"]["ethereum"]
    };
    ethereum["client"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("immutable deployment client is missing"))?
        .parse()
        .map_err(Into::into)
}

pub(super) fn journal_client(entry: &Value, legacy_client: Address) -> Result<Address> {
    match entry["clientAddress"].as_str() {
        Some(address) => Ok(address.parse()?),
        None if entry["clientAddress"].is_null() => Ok(legacy_client),
        None => Err(anyhow::anyhow!(
            "saved BEEFY journal client address is invalid"
        )),
    }
}

fn require_submission_client(
    submission: &Value,
    client: Address,
    legacy_client: Address,
) -> Result<()> {
    ensure!(
        journal_client(submission, legacy_client)? == client,
        "saved BEEFY submission belongs to a different client; refusing cross-client replay"
    );
    Ok(())
}

pub(super) fn require_saved_submission(
    submission: &Value,
    anchor: &CapturedCommitment,
    tx_hash: [u8; 32],
    expected_client: Address,
    legacy_client: Address,
) -> Result<()> {
    require_submission_client(submission, expected_client, legacy_client)?;
    ensure!(
        !submission.is_null()
            && submission["block"] == anchor.block
            && submission["blockHash"] == bytes(anchor.block_hash)
            && submission["root"] == bytes(anchor.validated.mmr_root)
            && submission["txHash"] == bytes(tx_hash)
            && submission["nonce"].as_u64().is_some()
            && submission["rawTransaction"].as_str().is_some(),
        "original commitment nonce, signed bytes, or identity is missing; refusing a replacement transaction"
    );
    let raw = decode(&submission["rawTransaction"])?;
    ensure!(
        keccak256(&raw) == tx_hash,
        "saved commitment bytes do not match their hash"
    );
    let mut encoded = raw.as_slice();
    let signed =
        TxEnvelope::decode_2718(&mut encoded).context("invalid saved commitment transaction")?;
    ensure!(
        encoded.is_empty()
            && signed.nonce() == number(&submission["nonce"])?
            && signed.to() == Some(expected_client),
        "saved commitment nonce or destination differs from signed bytes"
    );
    Ok(())
}

async fn wait_for_finalized_receipt(
    ethereum: &Ethereum,
    tx_hash: [u8; 32],
) -> Result<ethereum_client::FinalizedTransactionReceipt> {
    loop {
        if let Some(receipt) = ethereum.api.get_finalized_receipt(tx_hash.into()).await? {
            ensure!(
                receipt.receipt.transaction_hash == B256::from(tx_hash)
                    && receipt.receipt.status()
                    && receipt.receipt.block_number == Some(receipt.included_block_number)
                    && receipt.receipt.block_hash == Some(receipt.included_block_hash),
                "finalized BEEFY receipt identity or status is invalid"
            );
            return Ok(receipt);
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

fn journal_commitment(
    state: &mut Value,
    anchor: &CapturedCommitment,
    proof: &SourceProof,
    tx_hash: [u8; 32],
    destination: (u64, B256),
    client_address: Address,
) -> Result<()> {
    require_saved_submission(
        &state["submission"],
        anchor,
        tx_hash,
        client_address,
        deployment_client(state)?,
    )?;
    let destination_hash: [u8; 32] = destination.1.into();
    state["commitments"]
        .as_array()
        .context("missing commitment journal")?;
    let mut entry = json!({
            "clientAddress": format!("{client_address:#x}"),
            "block": anchor.block, "blockHash": bytes(anchor.block_hash), "rawScale": bytes(&anchor.raw),
            "current": anchor.current, "next": anchor.next, "freshnessProof": freshness_evidence(proof)?,
            "destinationBlock": destination.0, "destinationHash": bytes(destination_hash),
            "txHash": bytes(tx_hash), "finalized": false,
    });
    entry["submission"] = state["submission"].take();
    state["commitments"]
        .as_array_mut()
        .expect("validated commitment journal")
        .push(entry);
    state["follower"]["status"] = json!("healthy");
    state["follower"]["lastMinedUpdate"] = json!(anchor.block);
    state["follower"]["freshnessDeadlineMs"] =
        json!(proof.snapshot.source_timestamp_ms + 86_400_000);
    state["follower"]["lastError"] = Value::Null;
    state["error"] = Value::Null;
    Ok(())
}

async fn revalidate_finalized_commitments(
    source: &Source,
    witness: &Source,
    ethereum: &Ethereum,
    state: &Value,
    proof_directory: &Path,
) -> Result<()> {
    use std::collections::BTreeMap;
    tokio::time::timeout(Duration::from_secs(300), async {
        let legacy = deployment_client(state)?;
        let mut targets = BTreeMap::new();
        let mut source_targets = BTreeMap::new();
        let mut source_records = BTreeMap::<u64, &Value>::new();
        let mut original_inclusions = BTreeMap::new();
        let mut inclusions = BTreeMap::<(u64, B256), Vec<crate::ethereum::OriginalCommitmentReceipt>>::new();
        let bind = |targets: &mut BTreeMap<u64, B256>, number, hash| -> Result<()> {
            ensure!(targets.insert(number, hash).is_none_or(|old| old == hash), "original history contains conflicting block identities; HOLD");
            Ok(())
        };
        for entry in state["commitments"].as_array().context("missing commitment journal")?.iter().filter(|entry| entry["finalized"] == true) {
            let client = journal_client(entry, legacy)?;
            require_submission_client(&entry["submission"], client, legacy)?;
            let hash = B256::from(array::<32>(&entry["txHash"])?);
            let raw = decode(&entry["submission"]["rawTransaction"])?;
            let identity = crate::ethereum::signed_transaction_identity(&raw, hash, number(&entry["submission"]["nonce"])?, client)?;
            let mut encoded = raw.as_slice();
            let transaction = TxEnvelope::decode_2718(&mut encoded)?;
            ensure!(transaction.chain_id() == Some(number(&state["deployment"]["ethereum"]["chainId"])?)
                && entry["submission"]["txHash"] == entry["txHash"] && identity.to == Some(client),
                "original finalized signed transaction changed chain, hash or client binding; HOLD");
            let included = (number(&entry["destinationBlock"])? , B256::from(array::<32>(&entry["destinationHash"])?));
            bind(&mut targets, included.0, included.1)?;
            bind(&mut targets, number(&entry["finalizedBlock"])?, B256::from(array::<32>(&entry["finalizedHash"])?))?;
            let source_block: u32 = number(&entry["block"] )?.try_into()?;
            bind(&mut source_targets, u64::from(source_block), B256::from(array::<32>(&entry["blockHash"])?))?;
            if let Some(previous) = source_records.insert(u64::from(source_block), entry) {
                ensure!(previous["rawScale"] == entry["rawScale"] && previous["current"] == entry["current"] && previous["next"] == entry["next"],
                    "duplicate original source proof identity changed; HOLD");
            }
            crate::ethereum::validate_recorded_commitment_transaction(&raw, &decode(&entry["rawScale"] )?)?;
            let original = crate::ethereum::OriginalCommitmentReceipt { hash, raw, source_block,
                mmr_root: array::<32>(&entry["submission"]["root"] )?, client };
            if let Some(previous) = original_inclusions.insert(hash, included) {
                ensure!(previous == included, "original transaction changed canonical inclusion; HOLD");
            } else { inclusions.entry(included).or_default().push(original); }
        }
        let mut view = ethereum.api.verified_finalized_view_with_archive(&proof_directory.join("headers"),
            ethereum.finality_genesis().await?).await?;
        view.verify_blocks(ethereum.api.raw_provider(), &targets).await?;
        let (source_finalized, witness_finalized) = tokio::try_join!(
            async { source.api.block_hash_to_number(source.api.latest_finalized_block().await?).await },
            async { witness.api.block_hash_to_number(witness.api.latest_finalized_block().await?).await },
        )?;
        let identity = state.get("deployment").map(|deployment| &deployment["anchor"]).unwrap_or(state);
        ensure!(source.source_genesis == witness.source_genesis && source.bridge_domain == witness.bridge_domain
            && source.source_genesis == array::<32>(&identity["sourceGenesis"])?
            && source.bridge_domain == array::<32>(&identity["bridgeDomain"])?,
            "history source/witness identity differs from the immutable deployment; HOLD");
        let source_records = &source_records;
        let mut source_checks = stream::iter(source_targets.iter().map(|(&block, &hash)| async move {
            let block: u32 = block.try_into()?;
            ensure!(block <= source_finalized && block <= witness_finalized, "original source history is ahead of a finalized witness; HOLD");
            let entry = source_records.get(&u64::from(block)).context("original source proof is missing")?;
            let (captured, (current, next), first, second) = tokio::try_join!(
                source.recapture_at_finalized(block, decode(&entry["rawScale"] )?, source_finalized),
                witness.checkpoint_at(block), source.api.block_number_to_hash(block), witness.api.block_number_to_hash(block),
            )?;
            ensure!(first.0 == hash.0 && second == first && captured.block_hash == hash.0,
                "original finalized source commitment changed on source/witness; HOLD");
            ensure!(captured.current == current && captured.next == next
                && entry["current"] == serde_json::to_value(&current)? && entry["next"] == serde_json::to_value(&next)?,
                "original source proof authority history changed on source/witness; HOLD");
            require_saved_submission(&entry["submission"], &captured, array(&entry["txHash"] )?, journal_client(entry, legacy)?, legacy)?;
            Ok::<_, anyhow::Error>(())
        })).buffer_unordered(32);
        while let Some(result) = source_checks.next().await { result?; }
        let receipts_directory = proof_directory.join("receipts");
        let mut receipts = stream::iter(inclusions.iter().map(|(&(block, hash), originals)|
            crate::ethereum::authenticate_recorded_receipts(ethereum, &view, block, hash, originals, &receipts_directory)))
            .buffer_unordered(32);
        while let Some(result) = receipts.next().await { result?; }
        Ok(())
    }).await.context("saved finalized history did not revalidate within 300 seconds; HOLD; retain authenticated partial proof progress")?
}

/// Operational audit: never signs, broadcasts, changes actor status or rewrites
/// original journals. Only locally revalidated proof material is populated.
pub(super) async fn audit_history(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    deployment_manifest: &Path,
    state_path: &Path,
    proof_directory: &Path,
) -> Result<()> {
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    let state = read_state_file(state_path)?;
    validate_token_follower_journal(&state, &deployment)?;
    ensure!(
        source_rpc != witness_rpc,
        "history audit requires separate source/witness endpoints"
    );
    let ((source, witness), ethereum) = tokio::try_join!(
        async {
            Source::connect_pair(
                GearApi::new(source_rpc, 3).await?,
                GearApi::new(witness_rpc, 3).await?,
            )
            .await
        },
        Ethereum::connect_hoodi_readonly(ethereum_rpc, &state["activeEthereum"]),
    )?;
    source.validate_attachment(&deployment["anchor"]).await?;
    let started = std::time::Instant::now();
    revalidate_finalized_commitments(&source, &witness, &ethereum, &state, proof_directory).await?;
    let entries = state["commitments"]
        .as_array()
        .context("missing commitments")?;
    println!(
        "{}",
        json!({"status":"authenticated", "finalizedRecords":entries.iter().filter(|entry| entry["finalized"] == true).count(),
        "pendingRecords":entries.iter().filter(|entry| entry["finalized"] != true).count(), "elapsedMs":started.elapsed().as_millis(),
        "writesOriginalJournal":false, "proofDirectory":proof_directory})
    );
    Ok(())
}
// Only a mined observation may move. Signed identity, finalized history and root pins may not.
async fn authenticate_reinclusion(
    ethereum: &Ethereum,
    output: &Path,
    state: &Value,
    submission: &Value,
    anchor: &CapturedCommitment,
    finalized: &ethereum_client::FinalizedTransactionReceipt,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(300), async {
        let hash = finalized.receipt.transaction_hash;
        let hash_text = format!("{hash:#x}");
        let recorded = state["transactions"]
            .as_array()
            .context("missing transaction journal")?
            .iter()
            .find(|entry| entry["txHash"] == hash_text)
            .context("original mined receipt is missing; HOLD")?;
        for entry in state["commitments"]
            .as_array()
            .context("missing commitment journal")?
            .iter()
            .filter(|entry| entry["txHash"] == hash_text)
        {
            ensure!(
                entry["finalized"] == false
                    && entry["finalizedBlock"].is_null()
                    && entry["finalizedHash"].is_null()
                    && entry["firstInclusion"].is_null(),
                "finalized commitment history cannot be reincluded; HOLD"
            );
            ensure!(
                entry["destinationBlock"] == recorded["blockNumber"]
                    && entry["destinationHash"] == recorded["blockHash"],
                "original commitment and receipt observations disagree; HOLD"
            );
            ensure!(
                entry["current"] == serde_json::to_value(&anchor.current)?
                    && entry["next"] == serde_json::to_value(&anchor.next)?,
                "saved pending authority history changed; HOLD"
            );
        }
        ensure!(
            recorded["status"] == true
                && recorded["finality"].is_null()
                && recorded["firstInclusion"].is_null(),
            "previously finalized receipt cannot be reincluded; HOLD"
        );
        ensure!(
            state["mode"] == "hoodi-message-demo" || state["roots"].is_object(),
            "missing root publication inventory; HOLD"
        );
        let publications = output.join("root-publications");
        let mut registered_paths = std::collections::BTreeSet::new();
        if let Some(roots) = state.get("roots") {
            for root in roots
                .as_object()
                .context("root registration journal is malformed; HOLD")?
                .values()
            {
                ensure!(
                    matches!(
                        root["status"].as_str(),
                        Some("pending" | "mined" | "accepted")
                    ),
                    "malformed root registration status; HOLD"
                );
                let path = Path::new(
                    root["publication"]
                        .as_str()
                        .context("root publication path missing; HOLD")?,
                );
                ensure!(
                    path.parent() == Some(publications.as_path()),
                    "root publication lies outside its owned inventory; HOLD"
                );
                registered_paths.insert(path);
                let saved = match fs::read(path) {
                    Ok(saved) => saved,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound
                            && root["status"] == "pending" =>
                    {
                        continue
                    }
                    Err(error) => return Err(error.into()),
                };
                let intent: Value = serde_json::from_slice(&saved)?;
                let pinned = B256::from(array::<32>(&intent["acceptedAnchorTx"])?);
                let client: Address = intent["acceptedAnchorClient"]
                    .as_str()
                    .context("root anchor client missing; HOLD")?
                    .parse()?;
                if pinned == hash {
                    ensure!(
                        client == ethereum.client_address
                            && intent["acceptedCheckpoint"]["block"]
                                == finalized.included_block_number
                            && intent["acceptedCheckpoint"]["blockHash"]
                                == bytes(finalized.included_block_hash),
                        "saved root pins the original commitment inclusion; HOLD; do not reanchor"
                    );
                }
            }
        }
        if publications.try_exists()? {
            for entry in fs::read_dir(&publications)? {
                let entry = entry?;
                let path = entry.path();
                ensure!(
                    entry.file_type()?.is_file()
                        && path.extension().and_then(|extension| extension.to_str())
                            == Some("json")
                        && registered_paths.contains(path.as_path()),
                    "unindexed or incomplete root publication evidence; HOLD"
                );
            }
        }
        let raw = decode(&submission["rawTransaction"])?;
        crate::ethereum::signed_transaction_identity(
            &raw,
            hash,
            number(&submission["nonce"])?,
            ethereum.client_address,
        )?;
        let mut encoded = raw.as_slice();
        let signed = TxEnvelope::decode_2718(&mut encoded)?;
        ensure!(
            signed.chain_id() == Some(number(&state["deployment"]["ethereum"]["chainId"])?),
            "reincluded transaction changed chain; HOLD"
        );
        crate::ethereum::validate_recorded_commitment_transaction(&raw, &anchor.raw)?;
        let original = crate::ethereum::OriginalCommitmentReceipt {
            hash,
            raw,
            source_block: anchor.block,
            mmr_root: anchor.validated.mmr_root,
            client: ethereum.client_address,
        };
        let directory = output.join("finality-history");
        let mut view = ethereum
            .api
            .verified_finalized_view_with_archive(
                &directory.join("headers"),
                ethereum.finality_genesis().await?,
            )
            .await?;
        let targets = std::collections::BTreeMap::from([
            (
                finalized.included_block_number,
                finalized.included_block_hash,
            ),
            (
                finalized.finalized_block_number,
                finalized.finalized_block_hash,
            ),
        ]);
        view.verify_blocks(ethereum.api.raw_provider(), &targets)
            .await?;
        crate::ethereum::authenticate_recorded_receipts(
            ethereum,
            &view,
            finalized.included_block_number,
            finalized.included_block_hash,
            &[original],
            &directory.join("receipts"),
        )
        .await
    })
    .await
    .context(
        "reinclusion authentication did not finish within 300 seconds; HOLD original evidence",
    )?
}

async fn promote_finality(
    source: &mut Source,
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    let commitments = state["commitments"]
        .as_array()
        .context("missing commitment journal")?;
    if commitments.iter().all(|entry| entry["finalized"] == true) {
        return Ok(());
    }
    let finalized_number = ethereum.api.finalized_block_number().await?;
    let legacy_client = deployment_client(state)?;
    // Authenticate independent reads concurrently; apply journal changes in their original order.
    let pending = {
        let source = &*source;
        let ethereum = &*ethereum;
        stream::iter(
            commitments
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry["finalized"] != true)
                .map(|(index, entry)| async move {
                    let hash: [u8; 32] = array(&entry["txHash"])?;
                    let block: u32 = number(&entry["block"])?
                        .try_into()
                        .context("saved source commitment block overflows u32")?;
                    let anchor = source.recapture(block, decode(&entry["rawScale"])?).await?;
                    ensure!(
                        entry["blockHash"] == bytes(anchor.block_hash),
                        "source archive disagrees with pending commitment"
                    );
                    require_saved_submission(
                        &entry["submission"],
                        &anchor,
                        hash,
                        ethereum.client_address,
                        legacy_client,
                    )?;
                    let receipt = ethereum
                        .api
                        .raw_provider()
                        .get_transaction_receipt(hash.into())
                        .await?
                        .context(
                            "pending commitment receipt disappeared; hold the original transaction",
                        )?;
                    let included = receipt
                        .block_number
                        .context("pending receipt has no block number")?;
                    let included_hash = receipt
                        .block_hash
                        .context("pending receipt has no block hash")?;
                    ensure!(
                        receipt.transaction_hash == B256::from(hash)
                            && receipt.status()
                            && receipt.to == Some(ethereum.client_address),
                        "pending commitment identity or status changed; refusing further handovers"
                    );
                    let reincluded = entry["destinationBlock"] != included
                        || entry["destinationHash"] != bytes(included_hash);
                    crate::ethereum::wait_canonical_receipt(
                        ethereum.api.raw_provider(),
                        included,
                        included_hash,
                    )
                    .await?;
                    if included > finalized_number {
                        ensure!(
                            !reincluded,
                            "changed inclusion is not finalized; HOLD original observation"
                        );
                        return Ok(None);
                    }
                    let Some(finalized) = ethereum.api.get_finalized_receipt(hash.into()).await?
                    else {
                        ensure!(
                            !reincluded,
                            "changed inclusion has no finalized proof; HOLD original observation"
                        );
                        return Ok(None);
                    };
                    ensure!(
                        finalized.receipt.transaction_hash == B256::from(hash)
                            && finalized.receipt.status()
                            && finalized.included_block_number == included
                            && finalized.included_block_hash == included_hash
                            && finalized.receipt.block_number
                                == Some(finalized.included_block_number)
                            && finalized.receipt.block_hash == Some(finalized.included_block_hash),
                        "finalized BEEFY receipt differs from saved commitment"
                    );
                    Ok(Some((index, block, hash, anchor, finalized, reincluded)))
                }),
        )
        .buffered(16)
        .collect::<Vec<Result<_>>>()
        .await
    };
    let mut changed = false;
    let mut last_finalized = None;
    for result in pending {
        let Some((index, block, hash, anchor, finalized, reincluded)) = result? else {
            continue;
        };
        let entry = &state["commitments"][index];
        let proof = source.proof(block - 1, &anchor).await?;
        let destination = ethereum
            .verify_accepted_commitment(hash.into(), &anchor, &proof.snapshot)
            .await?;
        ensure!(
            destination.0 == finalized.included_block_number
                && destination.1 == finalized.included_block_hash,
            "finalized BEEFY receipt differs from authenticated destination checkpoint"
        );
        if reincluded {
            authenticate_reinclusion(
                ethereum,
                output,
                state,
                &entry["submission"],
                &anchor,
                &finalized,
            )
            .await?;
        }
        let first_inclusion = record_recovered_receipt(state, ethereum, &finalized)?;
        let destination_hash: [u8; 32] = finalized.included_block_hash.into();
        let finalized_hash: [u8; 32] = finalized.finalized_block_hash.into();
        let entry = &mut state["commitments"][index];
        if !first_inclusion.is_null() {
            entry["firstInclusion"] = first_inclusion;
        }
        entry["destinationBlock"] = json!(finalized.included_block_number);
        entry["destinationHash"] = json!(bytes(destination_hash));
        entry["finalized"] = json!(true);
        entry["finalizedBlock"] = json!(finalized.finalized_block_number);
        entry["finalizedHash"] = json!(bytes(finalized_hash));
        last_finalized = Some(entry["block"].clone());
        changed = true;
    }
    if let Some(block) = last_finalized {
        state["follower"]["lastSuccessfulUpdate"] = block.clone();
        state["follower"]["lastFinalizedUpdate"] = block;
    }
    if changed {
        save(output, state, Some(ethereum))?;
    }
    Ok(())
}

fn record_recovered_receipt(
    state: &mut Value,
    ethereum: &mut Ethereum,
    finalized: &ethereum_client::FinalizedTransactionReceipt,
) -> Result<Value> {
    let hash = bytes(finalized.receipt.transaction_hash);
    let mut entry = receipt_value("submitFiatShamir", &finalized.receipt, None);
    entry["finality"] = json!({
        "includedBlock": finalized.included_block_number,
        "includedBlockHash": format!("{:#x}", finalized.included_block_hash),
        "finalizedBlock": finalized.finalized_block_number,
        "finalizedBlockHash": format!("{:#x}", finalized.finalized_block_hash),
    });
    let transactions = state["transactions"]
        .as_array_mut()
        .context("missing transaction journal")?;
    if let Some(recorded) = transactions.iter_mut().find(|tx| tx["txHash"] == hash) {
        if !recorded["finality"].is_null() {
            ensure!(
                recorded["blockNumber"] == entry["blockNumber"]
                    && recorded["blockHash"] == entry["blockHash"],
                "finalized receipt inclusion changed; HOLD"
            );
            entry["finality"] = recorded["finality"].clone();
        }
        if let Some(first) = recorded
            .get("firstInclusion")
            .filter(|first| !first.is_null())
        {
            entry["firstInclusion"] = first.clone();
        } else if recorded["blockNumber"] != entry["blockNumber"]
            || recorded["blockHash"] != entry["blockHash"]
        {
            entry["firstInclusion"] = recorded.clone();
        }
        *recorded = entry.clone();
    } else {
        transactions.push(entry.clone());
    }
    let first_inclusion = entry["firstInclusion"].clone();
    if let Some(recorded) = ethereum
        .transactions
        .iter_mut()
        .find(|tx| tx["txHash"] == hash)
    {
        *recorded = entry;
    } else {
        ethereum.transactions.push(entry);
    }
    Ok(first_inclusion)
}

async fn accepted_tx(
    state: &Value,
    ethereum: &Ethereum,
    anchor: &CapturedCommitment,
) -> Result<[u8; 32]> {
    if state["submission"]["block"] == anchor.block {
        require_submission_client(
            &state["submission"],
            ethereum.client_address,
            deployment_client(state)?,
        )?;
        ensure!(
            state["submission"]["blockHash"] == bytes(anchor.block_hash),
            "interrupted submission has a different source hash"
        );
        if !state["submission"]["txHash"].is_null() {
            return array(&state["submission"]["txHash"]);
        }
    }
    for tx in state["transactions"]
        .as_array()
        .context("missing transaction journal")?
        .iter()
        .rev()
    {
        if tx["label"] != "submitFiatShamir" {
            continue;
        }
        for event in tx["events"].as_array().context("missing receipt events")? {
            if event["address"].as_str().is_none_or(|address| {
                !address.eq_ignore_ascii_case(&format!("{:#x}", ethereum.client_address))
            }) {
                continue;
            }
            let data = decode(&event["data"])?;
            if data.len() == 64
                && data[..32] == anchor.validated.mmr_root
                && data[32..56].iter().all(|byte| *byte == 0)
                && data[56..64] == u64::from(anchor.block).to_be_bytes()
            {
                return array(&tx["txHash"]);
            }
        }
    }
    let legacy_client = deployment_client(state)?;
    let mut from = 0;
    for entry in state["commitments"]
        .as_array()
        .context("missing commitment journal")?
        .iter()
        .rev()
    {
        let matches = journal_client(entry, legacy_client)? == ethereum.client_address;
        if matches {
            from = entry["destinationBlock"].as_u64().unwrap_or(0);
            break;
        }
    }
    let signature = B256::from(keccak256(b"NewMMRRoot(bytes32,uint64)"));
    let logs = ethereum
        .api
        .raw_provider()
        .get_logs(
            &Filter::new()
                .address(ethereum.client_address)
                .event_signature(signature)
                .from_block(from),
        )
        .await?;
    let mut accepted = None;
    for log in logs {
        let data = log.data().data.as_ref();
        if data.len() == 64
            && data[..32] == anchor.validated.mmr_root
            && data[32..56].iter().all(|byte| *byte == 0)
            && data[56..64] == u64::from(anchor.block).to_be_bytes()
        {
            let hash = log
                .transaction_hash
                .context("accepted event has no transaction hash")?;
            ensure!(
                accepted.replace(hash).is_none(),
                "multiple receipts match the accepted source commitment"
            );
        }
    }
    accepted.map(Into::into).context("destination is ahead of journal but accepted receipt is unavailable; refusing duplicate submission")
}

pub(crate) fn checkpoint_matches_source(
    destination: &DestinationCheckpoint,
    root: Hash32,
    timestamp_ms: u64,
    current: &AuthoritySet,
    next: &AuthoritySet,
) -> bool {
    destination.root == root
        && destination.source_timestamp_ms == timestamp_ms
        && destination.current_id == current.id
        && destination.current_len == current.keys.len() as u64
        && destination.current_root == current.root
        && destination.next_id == next.id
        && destination.next_len == next.keys.len() as u64
        && destination.next_root == next.root
}

fn client_heartbeat_due(source_timestamp_ms: u64, destination_timestamp_ms: u64) -> bool {
    destination_timestamp_ms.saturating_sub(source_timestamp_ms) >= 12 * 60 * 60 * 1_000
}

async fn record_mined_commitment(
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
    anchor: &CapturedCommitment,
    proof: &SourceProof,
    hash: [u8; 32],
) -> Result<()> {
    if state["roots"].is_object() {
        crate::tokens::ensure_mined_root_publications(ethereum, &state["roots"]).await?;
    }
    require_saved_submission(
        &state["submission"],
        anchor,
        hash,
        ethereum.client_address,
        deployment_client(state)?,
    )?;
    let recorded = state["transactions"]
        .as_array()
        .context("missing transaction journal")?
        .iter()
        .find(|tx| tx["txHash"] == state["submission"]["txHash"]);
    let mut recovered = None;
    let receipt = if let Some(recorded) = recorded {
        let receipt = ethereum
            .api
            .raw_provider()
            .get_transaction_receipt(hash.into())
            .await?
            .context("previously mined commitment disappeared; hold original inclusion")?;
        let recorded_hash: [u8; 32] = array(&recorded["blockHash"])?;
        ensure!(
            recorded["status"] == true,
            "original commitment receipt was unsuccessful; HOLD"
        );
        if receipt.block_number != Some(number(&recorded["blockNumber"])?)
            || receipt.block_hash != Some(recorded_hash.into())
        {
            let finalized = ethereum
                .api
                .get_finalized_receipt(hash.into())
                .await?
                .context("changed mined inclusion is not finalized; HOLD original receipt")?;
            ensure!(
                receipt.block_number == Some(finalized.included_block_number)
                    && receipt.block_hash == Some(finalized.included_block_hash),
                "reincluded commitment changed during finality authentication; HOLD"
            );
            authenticate_reinclusion(
                ethereum,
                output,
                state,
                &state["submission"],
                anchor,
                &finalized,
            )
            .await?;
            recovered = Some(finalized);
        }
        receipt
    } else {
        ethereum.resume_commitment(&state["submission"]).await?
    };
    let destination = ethereum
        .verify_accepted_commitment(hash.into(), anchor, &proof.snapshot)
        .await?;
    ensure!(
        receipt.transaction_hash == B256::from(hash)
            && receipt.block_number == Some(destination.0)
            && receipt.block_hash == Some(destination.1),
        "original commitment receipt changed during authentication"
    );
    if let Some(finalized) = &recovered {
        record_recovered_receipt(state, ethereum, finalized)?;
    }
    if !ethereum
        .transactions
        .iter()
        .any(|tx| tx["txHash"] == state["submission"]["txHash"])
    {
        ethereum
            .transactions
            .push(receipt_value("submitFiatShamir", &receipt, None));
    }
    journal_commitment(
        state,
        anchor,
        proof,
        hash,
        destination,
        ethereum.client_address,
    )?;
    save(output, state, Some(ethereum))
}

async fn finish_commitment_finality(
    source: &mut Source,
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    for entry in state["commitments"]
        .as_array()
        .context("missing commitment journal")?
    {
        if entry["finalized"] != true {
            wait_for_finalized_receipt(ethereum, array(&entry["txHash"])?).await?;
        }
    }
    promote_finality(source, ethereum, output, state).await
}

async fn advance(
    source: &mut Source,
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
    mut handover_only: bool,
    root_witness: Option<&Source>,
) -> Result<CapturedCommitment> {
    let before = ethereum.checkpoint().await?;
    let reserved_block = if state["submission"].is_null() {
        None
    } else {
        Some(u32::try_from(number(&state["submission"]["block"])?)?)
    };
    let target = source
        .api
        .block_hash_to_number(source.api.latest_finalized_block().await?)
        .await?;
    let mut freshness_check: Option<std::time::Instant> = None;
    let anchor = loop {
        let mut candidate = match source.next_commitment().await {
            Ok(candidate) => candidate,
            Err(error) if error.downcast_ref::<source::CommitmentPending>().is_some() => {
                state["follower"]["status"] = json!("catching-up");
                state["follower"]["lastError"] = json!(error.to_string());
                save(output, state, Some(ethereum))?;
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some(block) = reserved_block {
            ensure!(
                candidate.block <= block,
                "source archive passed the original reserved commitment; HOLD"
            );
        }
        // Leave half an epoch for destination inclusion without breaching the 64-block lag gate.
        handover_only &= u64::from(candidate.block).saturating_sub(before.block) < 32;
        if handover_only && freshness_check.is_none_or(|checked| checked.elapsed().as_secs() >= 60)
        {
            let block = ethereum
                .api
                .raw_provider()
                .get_block_by_number(alloy::rpc::types::BlockNumberOrTag::Latest)
                .await?
                .context("latest Ethereum block is missing during client freshness check")?;
            let timestamp_ms = block
                .header
                .timestamp
                .checked_mul(1_000)
                .context("Ethereum timestamp exceeds supported milliseconds")?;
            handover_only = !client_heartbeat_due(before.source_timestamp_ms, timestamp_ms);
            freshness_check = Some(std::time::Instant::now());
        }
        let root_target = if let Some(witness) = root_witness {
            scan_token_roots(source, witness, output, state).await?;
            let pending = pending_token_root_block(state)?;
            handover_only &= pending.is_none();
            pending
        } else {
            None
        };
        if u64::from(candidate.block) <= before.block {
            let bootstrap = client_bootstrap(state, ethereum.client_address)?;
            if u64::from(candidate.block) == before.block {
                let proof = source.proof(candidate.block - 1, &candidate).await?;
                let expected_root = if before.root == [0; 32] {
                    ensure!(
                        bootstrap["block"] == candidate.block,
                        "zero-root client is not at the signed bootstrap"
                    );
                    [0; 32]
                } else {
                    candidate.validated.mmr_root
                };
                ensure!(
                    checkpoint_matches_source(
                        &before,
                        expected_root,
                        proof.snapshot.source_timestamp_ms,
                        &candidate.current,
                        &candidate.next,
                    ),
                    "destination checkpoint is not canonical source history"
                );
            }
            let entries = state["commitments"]
                .as_array()
                .context("missing commitment journal")?;
            let legacy_client = deployment_client(state)?;
            let mut journaled = None;
            let mut last = bootstrap;
            for entry in entries {
                if journal_client(entry, legacy_client)? != ethereum.client_address {
                    continue;
                }
                last = entry;
                if entry["block"] == candidate.block {
                    journaled = Some(entry);
                }
            }
            let recorded =
                journaled.or_else(|| (bootstrap["block"] == candidate.block).then_some(bootstrap));
            if let Some(recorded) = recorded {
                let original = source
                    .recapture(candidate.block, decode(&recorded["rawScale"])?)
                    .await?;
                ensure!(
                    recorded["blockHash"] == bytes(candidate.block_hash)
                        && original.block_hash == candidate.block_hash
                        && original.validated.commitment_bytes
                            == candidate.validated.commitment_bytes
                        && original.current == candidate.current
                        && original.next == candidate.next,
                    "source archive disagrees with recorded BEEFY commitment at block {}",
                    candidate.block
                );
                // Both signature subsets have been authenticated. Keep the
                // original proof bytes as the journal/submission identity.
                candidate = original;
            }
            if journaled.is_none() && candidate.block > number(&last["block"])? as u32 {
                let last_current = number(&last["current"]["id"])?;
                ensure!(
                    candidate.current.id == last_current
                        || candidate.current.id == number(&last["next"]["id"])?,
                    "destination skipped an unjournaled authority handover"
                );
                if u64::from(candidate.block) < before.block {
                    ensure!(
                        candidate.current.id == last_current
                            || (reserved_block.map(u64::from) == Some(before.block)
                                && candidate.current.id == before.current_id),
                        "destination skipped an unjournaled authority handover"
                    );
                    continue;
                }
                let proof = source.proof(candidate.block - 1, &candidate).await?;
                let tx_hash = accepted_tx(state, ethereum, &candidate).await?;
                require_saved_submission(
                    &state["submission"],
                    &candidate,
                    tx_hash,
                    ethereum.client_address,
                    deployment_client(state)?,
                )?;
                promote_finality(source, ethereum, output, state).await?;
                record_mined_commitment(ethereum, output, state, &candidate, &proof, tx_hash)
                    .await?;
                eprintln!(
                    "Hoodi: reconciled accepted source block {}",
                    candidate.block
                );
                return Ok(candidate);
            }
            continue;
        }
        ensure!(
            candidate.current.id == before.current_id || candidate.current.id == before.next_id,
            "cannot skip a destination authority handover"
        );
        if let Some(block) = reserved_block {
            if candidate.block == block {
                if !state["submission"]["rawScale"].is_null() {
                    let original = source
                        .recapture(block, decode(&state["submission"]["rawScale"])?)
                        .await?;
                    ensure!(
                        original.block_hash == candidate.block_hash
                            && original.validated.commitment_bytes
                                == candidate.validated.commitment_bytes
                            && original.current == candidate.current
                            && original.next == candidate.next,
                        "reserved source proof conflicts with the authenticated commitment; HOLD"
                    );
                    candidate = original;
                }
                break candidate;
            }
            continue;
        }
        // Intermediate same-set roots need not be relayed; every handover must be.
        if candidate.current.id == before.next_id
            || (!handover_only
                && candidate.block >= target.saturating_sub(2)
                && root_target.is_none_or(|block| candidate.block > block))
        {
            break candidate;
        }
    };
    promote_finality(source, ethereum, output, state).await?;
    if state["roots"].is_object() {
        crate::tokens::ensure_mined_root_publications(ethereum, &state["roots"]).await?;
    }
    if !state["submission"].is_null() {
        require_submission_client(
            &state["submission"],
            ethereum.client_address,
            deployment_client(state)?,
        )?;
        ensure!(
            state["submission"]["block"] == anchor.block
                && state["submission"]["blockHash"] == bytes(anchor.block_hash)
                && state["submission"]["root"] == bytes(anchor.validated.mmr_root),
            "unresolved prior commitment differs from source archive"
        );
        ensure!(state["submission"]["txHash"].as_str().is_some() || state["submission"]["nonce"].as_u64().is_some(),
            "unresolved prior commitment has no transaction hash or reserved nonce; refusing duplicate submission");
    }
    if state["submission"]["txHash"].as_str().is_some() {
        let hash: [u8; 32] = array(&state["submission"]["txHash"])?;
        require_saved_submission(
            &state["submission"],
            &anchor,
            hash,
            ethereum.client_address,
            deployment_client(state)?,
        )?;
        let proof = source.proof(anchor.block - 1, &anchor).await?;
        record_mined_commitment(ethereum, output, state, &anchor, &proof, hash).await?;
        eprintln!(
            "Hoodi: reconciled interrupted source block {}",
            anchor.block
        );
        return Ok(anchor);
    }
    let proof = source.proof(anchor.block - 1, &anchor).await?;
    let prior = (!state["submission"].is_null()).then(|| state["submission"].clone());
    let client_address = ethereum.client_address;
    ethereum
        .submit_commitment(
            &anchor,
            &proof.leaf,
            &proof.snapshot,
            &proof.simplified,
            prior.as_ref(),
            |transaction, receipt| {
                state["submission"] = transaction.clone();
                state["submission"]["clientAddress"] = json!(format!("{client_address:#x}"));
                state["submission"]["blockHash"] = json!(bytes(anchor.block_hash));
                state["submission"]["root"] = json!(bytes(anchor.validated.mmr_root));
                if let Some(receipt) = receipt {
                    state["transactions"]
                        .as_array_mut()
                        .context("missing transaction journal")?
                        .push(receipt_value("submitFiatShamir", receipt, None));
                }
                save(output, state, None)
            },
        )
        .await?;
    let hash: [u8; 32] = array(&state["submission"]["txHash"])?;
    require_saved_submission(
        &state["submission"],
        &anchor,
        hash,
        ethereum.client_address,
        deployment_client(state)?,
    )?;
    record_mined_commitment(ethereum, output, state, &anchor, &proof, hash).await?;
    eprintln!(
        "Hoodi: mined block {} set {}; destination finality pending",
        anchor.block, anchor.current.id
    );
    Ok(anchor)
}
async fn prepare_messages(
    source: &mut Source,
    ethereum: &Ethereum,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    if state["messagesPrepared"] == true {
        return Ok(());
    }
    if !state["sourceOperation"].is_null() {
        let operation = state["sourceOperation"].clone();
        let index: usize = number(&operation["rotation"])
            .context("interrupted source operation is not a recoverable original rotation; HOLD")?
            .try_into()?;
        let prepared: source::PreparedRotation =
            serde_json::from_value(operation["prepared"].clone())
                .context("interrupted rotation has no original signed identity; HOLD")?;
        let rotation = source.reconcile_rotation(&prepared).await?.context(
            "original rotation has no finalized inclusion; HOLD without another handoff",
        )?;
        ensure!(
            state["rotations"]
                .as_array()
                .context("missing rotations")?
                .len()
                == index,
            "original rotation index changed; HOLD"
        );
        state["rotations"]
            .as_array_mut()
            .expect("checked rotations")
            .push(json!({"root":operation["root"], "extrinsic":rotation}));
        state["sourceOperation"] = Value::Null;
        save(output, state, Some(ethereum))?;
    }
    if state["stashFunding"].is_null() {
        state["sourceOperation"] = json!("fund stash");
        save(output, state, Some(ethereum))?;
        state["stashFunding"] = json!(source::fund_stash(&source.api).await?.0);
        state["sourceOperation"] = Value::Null;
        save(output, state, Some(ethereum))?;
    }
    for index in 0..2 {
        if index == 0
            && !state["messages"]
                .as_array()
                .context("missing messages")?
                .is_empty()
        {
            continue;
        }
        if state["rotations"]
            .as_array()
            .context("missing rotations")?
            .len()
            <= index
        {
            let uri = format!("//Alice//beefy-hoodi-{}", index + 1);
            let alice = sp_core::ecdsa::Pair::from_string(&uri, None)
                .map_err(|_| anyhow::anyhow!("invalid local rotation URI"))?;
            let bob = sp_core::ecdsa::Pair::from_string("//Bob", None)
                .map_err(|_| anyhow::anyhow!("invalid local Bob URI"))?;
            let root = authority_root(&authority_addresses(&[alice.public().0, bob.public().0])?)?;
            let prepared = source.prepare_rotation("Alice", &uri).await?;
            state["sourceOperation"] =
                json!({"rotation":index, "root":bytes(root), "prepared":prepared, "handoff":false});
            save(output, state, Some(ethereum))?;
            state["sourceOperation"]["handoff"] = json!(true);
            save(output, state, Some(ethereum))?;
            let rotation = source.submit_prepared_rotation(&prepared).await?;
            state["rotations"]
                .as_array_mut()
                .expect("checked rotations")
                .push(json!({"root": bytes(root), "extrinsic": rotation}));
            state["sourceOperation"] = Value::Null;
            save(output, state, Some(ethereum))?;
        }
        let root: Hash32 = array(&state["rotations"][index]["root"])?;
        if index == 0 {
            wait_for_root(source, root).await?;
            source::initialize_bridge(&source.api).await?;
        }
        if state["messages"]
            .as_array()
            .context("missing messages")?
            .len()
            <= index
        {
            state["sourceOperation"] = json!({"message": index});
            save(output, state, Some(ethereum))?;
            let payload = format!("beefy-hoodi-message-{index}");
            let message = source::send_message(
                &source.api,
                ethereum.receiver_address(),
                payload.as_bytes(),
                "//Charlie",
            )
            .await?;
            state["messages"]
                .as_array_mut()
                .expect("checked messages")
                .push(message_evidence(&message));
            state["sourceOperation"] = Value::Null;
            save(output, state, Some(ethereum))?;
        }
        if index == 1 {
            wait_for_root(source, root).await?;
        }
    }
    state["messagesPrepared"] = json!(true);
    phase(
        output,
        state,
        Some(ethereum),
        "source messages retained; safe to restart relay",
    )
}

async fn wait_for_root(source: &mut Source, root: Hash32) -> Result<()> {
    loop {
        let hash = source.api.latest_finalized_block().await?;
        if source.checkpoint_at_hash(hash).await?.0.root == root {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
async fn verify(
    source: &mut Source,
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    let first = restore_message(&state["messages"][0])?;
    let second = restore_message(&state["messages"][1])?;
    ensure!(
        first.snapshot.bridge_domain == source.bridge_domain
            && second.snapshot.bridge_domain == source.bridge_domain,
        "saved messages belong to another source"
    );
    ensure!(
        first.message.nonce_be != second.message.nonce_be
            && first.snapshot.queue_root != second.snapshot.queue_root,
        "messages must have distinct nonces and roots"
    );
    let second_root: Hash32 = array(&state["rotations"][1]["root"])?;
    let mut anchor = loop {
        let anchor = advance(source, ethereum, output, state, false, None).await?;
        if anchor.current.root == second_root && anchor.block > second.block {
            break anchor;
        }
    };
    for (index, message) in [&first, &second].into_iter().enumerate() {
        ensure!(
            source.api.block_number_to_hash(message.block).await?.0 == message.block_hash,
            "retained source message reorged"
        );
        if let Some(root) = ethereum
            .api
            .read_chainhead_merkle_root(message.block)
            .await?
        {
            ensure!(
                root == message.snapshot.queue_root,
                "stored root conflicts with retained message"
            );
        } else {
            let proof = source.proof(message.block, &anchor).await?;
            let envelope = queue_envelope(&proof, &anchor, message)?;
            if index == 1 && state["messages"][index]["staleAnchorRejected"] != true {
                state["messages"][index]["staleQueueInclusion"] =
                    proof_evidence(&proof, &anchor, &envelope);
                save(output, state, Some(ethereum))?;
                anchor = advance(source, ethereum, output, state, false, None).await?;
                ethereum
                    .reject_registration(message.block, message.snapshot.queue_root, envelope)
                    .await?;
                state["messages"][index]["staleAnchorRejected"] = json!(true);
            }
            let proof = source.proof(message.block, &anchor).await?;
            let envelope = queue_envelope(&proof, &anchor, message)?;
            state["messages"][index]["queueInclusion"] = proof_evidence(&proof, &anchor, &envelope);
            save(output, state, Some(ethereum))?;
            ethereum
                .register(message.block, message.snapshot.queue_root, envelope)
                .await?;
            save(output, state, Some(ethereum))?;
            ethereum
                .reject_early(message.block, &message.message, &message.inclusion)
                .await?;
            state["messages"][index]["earlyDeliveryRejected"] = json!(true);
            save(output, state, Some(ethereum))?;
        }
    }
    phase(
        output,
        state,
        Some(ethereum),
        "waiting real Hoodi queue maturity",
    )?;
    for (index, message) in [&first, &second].into_iter().enumerate() {
        if !ethereum.is_processed(message.message.nonce_be).await? {
            ethereum.wait_maturity(message.block).await?;
            ethereum
                .deliver(message.block, &message.message, &message.inclusion)
                .await?;
            save(output, state, Some(ethereum))?;
        }
        ethereum
            .reject_replay(message.block, &message.message, &message.inclusion)
            .await?;
        state["messages"][index]["delivered"] = json!(true);
        state["messages"][index]["replayRejected"] = json!(true);
        save(output, state, Some(ethereum))?;
    }
    let finalized = source.api.latest_finalized_block().await?;
    let queue_id = source.api.fetch_queue_merkle_root(finalized).await?.0;
    ensure!(
        queue_id > second.snapshot.queue_id,
        "natural queue clear not observed"
    );
    state["queueIdAfterClear"] = json!(queue_id);
    for rotation in state["rotations"].as_array().context("missing rotations")? {
        let root: Hash32 = array(&rotation["root"])?;
        ensure!(
            state["commitments"]
                .as_array()
                .context("missing commitments")?
                .iter()
                .any(|c| c["current"]["root"] == json!(root)),
            "changed authority root was not authenticated on Hoodi"
        );
    }
    ensure!(
        state["messages"][0]["earlyDeliveryRejected"] == true
            && state["messages"][1]["staleAnchorRejected"] == true,
        "negative-case evidence incomplete after interruption; do not claim a pass"
    );
    phase(
        output,
        state,
        Some(ethereum),
        "waiting Hoodi finality and canonical receipts",
    )?;
    ethereum.wait_finalized().await?;
    finish_commitment_finality(source, ethereum, output, state).await?;
    for message in [&first, &second] {
        ensure!(
            ethereum.is_processed(message.message.nonce_be).await?,
            "delivery disappeared before destination finality"
        );
    }
    state["finalizedEthereumBlock"] = json!(ethereum.api.finalized_block_number().await?);
    state["finalEthereum"] = ethereum.manifest().await?;
    state["status"] = json!("passed");
    state["error"] = Value::Null;
    phase(output, state, Some(ethereum), "verified")
}
pub async fn run(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    output: &Path,
    prepare_only: bool,
    follow: bool,
) -> Result<()> {
    ensure!(
        crate::local_source_rpc(source_rpc) && crate::local_source_rpc(witness_rpc),
        "public dev keys: source and witness RPCs must be loopback"
    );
    ensure!(
        source_rpc != witness_rpc,
        "two independent source endpoints required"
    );
    ensure!(
        ethereum_rpc.starts_with("wss://"),
        "Hoodi requires secure websocket RPC"
    );
    fs::create_dir_all(output)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(output.join("relay.lock"))?;
    lock.try_lock()
        .context("another relay owns this output directory")?;
    let resumed = output.join("state.json").exists();
    let mut state = if resumed {
        serde_json::from_slice(&fs::read(output.join("state.json"))?)?
    } else {
        json!({"schemaVersion": 1, "mode": "hoodi-message-demo", "status": "preparing", "messages": [], "rotations": [], "commitments": [], "transactions": []})
    };
    ensure!(
        state["schemaVersion"] == 1 && state["mode"] == "hoodi-message-demo",
        "unsupported Hoodi evidence format"
    );
    let (mut source, mut witness) = Source::connect_pair(
        GearApi::new(source_rpc, 3).await?,
        GearApi::new(witness_rpc, 3).await?,
    )
    .await?;
    ensure!(
        source.source_genesis == witness.source_genesis
            && source.bridge_domain == witness.bridge_domain
            && source.mmr_start_block == witness.mmr_start_block
            && source.beefy_activation_block == witness.beefy_activation_block,
        "source identity differs between authorities"
    );
    if resumed {
        ensure!(
            state["sourceGenesis"] == bytes(source.source_genesis)
                && state["bridgeDomain"] == bytes(source.bridge_domain)
                && state["mmrStartBlock"] == source.mmr_start_block,
            "source identity changed across restart"
        );
        if !state["bootstrap"].is_null() {
            let block: u32 = number(&state["bootstrap"]["block"])?.try_into()?;
            ensure!(
                source.api.block_number_to_hash(block).await?.0
                    == array::<32>(&state["bootstrap"]["blockHash"])?,
                "source database reset or bootstrap reorged"
            );
        }
    } else {
        state["sourceGenesis"] = json!(bytes(source.source_genesis));
        state["bridgeDomain"] = json!(bytes(source.bridge_domain));
        state["mmrStartBlock"] = json!(source.mmr_start_block);
        state["beefyActivationBlock"] = json!(source.beefy_activation_block);
        state["runtime"] = source::validate_legacy_runtime(&source.api).await?;
        state["witnessRuntime"] = source::validate_legacy_runtime(&witness.api).await?;
        save(output, &mut state, None)?;
    }
    let mut ethereum = if state["ethereum"].is_null() {
        ensure!(
            state["phase"] != "broadcasting deployment",
            "interrupted deployment: reconcile Foundry broadcast; refusing duplicate deployment"
        );
        let (anchor, snapshot) = bootstrap(&mut source, &mut witness, output, &mut state).await?;
        phase(output, &mut state, None, "broadcasting deployment")?;
        let ethereum = Ethereum::prepare_hoodi(
            output,
            &anchor,
            &snapshot,
            source.mmr_start_block,
            ethereum_rpc,
            wallet,
        )
        .await?;
        state["ethereum"] = ethereum.manifest().await?;
        save(output, &mut state, Some(&ethereum))?;
        ethereum
    } else {
        let mut ethereum =
            Ethereum::connect_hoodi(ethereum_rpc, wallet, &state["ethereum"]).await?;
        ethereum.transactions = serde_json::from_value(state["transactions"].clone())?;
        state["resumed"] = json!(true);
        ethereum
    };
    let result: Result<()> = async {
        prepare_messages(&mut source, &ethereum, output, &mut state).await?;
        if prepare_only {
            return Ok(());
        }
        if state["status"] != "passed" {
            verify(&mut source, &mut ethereum, output, &mut state).await?;
        }
        if follow {
            phase(
                output,
                &mut state,
                Some(&ethereum),
                "following finalized source commitments",
            )?;
            loop {
                advance(&mut source, &mut ethereum, output, &mut state, true, None).await?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        state["follower"]["status"] = json!("failed");
        state["follower"]["lastError"] = json!(format!("{error:#}"));
        state["error"] = json!(format!("{error:#}"));
        save(output, &mut state, Some(&ethereum))?;
    }
    result
}

fn retain_token_root(
    output: &Path,
    state: &mut Value,
    block: u32,
    block_hash: Hash32,
    queue_id: u64,
    queue_root: Hash32,
) -> Result<()> {
    let kind = if queue_root == [0; 32] {
        "emptyProgress"
    } else {
        "merkleRoot"
    };
    let key = format!("{block}-{}", hex::encode(queue_root));
    let roots = state["roots"]
        .as_object_mut()
        .context("missing root registration journal")?;
    for saved in roots.values().filter(|saved| saved["block"] == block) {
        ensure!(
            saved["blockHash"] == bytes(block_hash)
                && saved["queueId"] == queue_id
                && saved["queueRoot"] == bytes(queue_root)
                && saved["kind"].as_str().unwrap_or("merkleRoot") == kind,
            "finalized registration changed during backfill"
        );
    }
    if let serde_json::map::Entry::Vacant(entry) = roots.entry(key) {
        let publication = output
            .join("root-publications")
            .join(format!("{}.json", entry.key()));
        entry.insert(json!({
            "block": block, "blockHash": bytes(block_hash), "queueId": queue_id,
            "queueRoot": bytes(queue_root), "kind": kind, "status": "pending", "publication": publication,
        }));
    }
    // A crash here re-scans this block without losing or replacing the saved intent.
    save(output, state, None)
}

fn pending_token_root_block(state: &Value) -> Result<Option<u32>> {
    let mut latest = None;
    for root in state["roots"]
        .as_object()
        .context("missing root journal")?
        .values()
    {
        if root["status"] == "pending" {
            let block: u32 = number(&root["block"])?.try_into()?;
            latest = Some(latest.map_or(block, |previous: u32| previous.max(block)));
        }
    }
    Ok(latest)
}

async fn scan_token_roots(
    source: &Source,
    witness: &Source,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    let (source_head, witness_head) = tokio::try_join!(
        source.api.latest_finalized_block(),
        witness.api.latest_finalized_block()
    )?;
    let (source_height, witness_height) = tokio::try_join!(
        source.api.block_hash_to_number(source_head),
        witness.api.block_hash_to_number(witness_head)
    )?;
    let limit = source_height.min(witness_height);
    let cursor: u32 = number(&state["rootScan"]["block"])?.try_into()?;
    ensure!(
        limit >= cursor,
        "source or witness has not recovered the saved finalized cursor"
    );
    let saved_hash = H256(array(&state["rootScan"]["blockHash"])?);
    ensure!(
        source.api.block_number_to_hash(cursor).await? == saved_hash
            && witness.api.block_number_to_hash(cursor).await? == saved_hash,
        "saved root-scan cursor is no longer canonical on both authorities"
    );
    if cursor == limit {
        return Ok(());
    }
    for block in cursor
        .checked_add(1)
        .context("source scan cursor overflow")?..=limit
    {
        let hash = source.api.block_number_to_hash(block).await?;
        ensure!(
            witness.api.block_number_to_hash(block).await? == hash,
            "source and witness disagree at finalized root-scan block {block}"
        );
        if source::has_event(&source.api, hash, "GearEthBridge", "QueueMerkleRootChanged").await? {
            ensure!(
                source::has_event(
                    &witness.api,
                    hash,
                    "GearEthBridge",
                    "QueueMerkleRootChanged"
                )
                .await?,
                "witness is missing the finalized queue registration event"
            );
            let (first, second) = tokio::try_join!(
                source.api.fetch_queue_merkle_root(hash),
                witness.api.fetch_queue_merkle_root(hash)
            )?;
            ensure!(
                first == second,
                "finalized queue registration differs between authorities"
            );
            retain_token_root(output, state, block, hash.0, first.0, first.1 .0)?;
        }
        state["rootScan"] = json!({"block":block, "blockHash":bytes(hash.0)});
        if block % 32 == 0 {
            save(output, state, None)?;
        }
    }
    save(output, state, None)
}

const EMPTY_PROGRESS_MAX_DISTANCE: u64 = 57_600;

fn root_requires_empty_progress(max_block: u64, root_block: u64) -> bool {
    max_block != 0 && root_block.saturating_sub(max_block) > EMPTY_PROGRESS_MAX_DISTANCE
}

fn next_empty_progress_block(
    max_block: u64,
    finalized_height: u32,
    anchor_block: u32,
    next_root: Option<u32>,
) -> Option<u32> {
    if max_block == 0 {
        return None;
    }
    let window_end = max_block.checked_add(EMPTY_PROGRESS_MAX_DISTANCE)?;
    if u64::from(finalized_height) < window_end
        || next_root.is_some_and(|block| !root_requires_empty_progress(max_block, u64::from(block)))
    {
        return None;
    }
    let target = window_end
        .min(u64::from(finalized_height))
        .min(u64::from(anchor_block.saturating_sub(1)))
        .min(next_root.map_or(u64::MAX, |block| u64::from(block.saturating_sub(1))));
    (target > max_block && target <= u64::from(u32::MAX)).then_some(target as u32)
}

async fn schedule_empty_progress(
    source: &mut Source,
    witness: &mut Source,
    max_block: u64,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    if max_block == 0
        || !state["submission"].is_null()
        || state["roots"]
            .as_object()
            .context("missing root registration journal")?
            .values()
            .any(|root| root["kind"] == "emptyProgress" && root["status"] == "pending")
    {
        return Ok(());
    }
    let source_finalized = source.api.latest_finalized_block().await?;
    let witness_finalized = witness.api.latest_finalized_block().await?;
    let source_height = source.api.block_hash_to_number(source_finalized).await?;
    let witness_height = witness.api.block_hash_to_number(witness_finalized).await?;
    let finalized_height = source_height.min(witness_height);
    let active_client = state["activeEthereum"]["client"]
        .as_str()
        .context("active BEEFY client missing")?;
    let anchor_entry = state["commitments"]
        .as_array()
        .context("missing commitment journal")?
        .iter()
        .rev()
        .find(|entry| entry["clientAddress"].as_str() == Some(active_client));
    let Some(anchor_entry) = anchor_entry else {
        return Ok(());
    };
    let anchor_block: u32 = number(&anchor_entry["block"])?.try_into()?;
    let next_root = state["roots"]
        .as_object()
        .context("missing root registration journal")?
        .values()
        .filter(|root| root["status"] == "pending")
        .try_fold(None, |earliest: Option<u32>, root| {
            let block = number(&root["block"])?.try_into()?;
            Ok::<_, anyhow::Error>(Some(earliest.map_or(block, |earliest| earliest.min(block))))
        })?;
    let Some(target) =
        next_empty_progress_block(max_block, finalized_height, anchor_block, next_root)
    else {
        return Ok(());
    };

    let anchor = source
        .recapture(anchor_block, decode(&anchor_entry["rawScale"])?)
        .await?;
    let (first, second) = tokio::try_join!(
        source.proof(target, &anchor),
        witness.proof(target, &anchor)
    )?;
    ensure!(
        first.snapshot == second.snapshot
            && first.source_hash == second.source_hash
            && first.raw_leaf == second.raw_leaf,
        "source authorities disagree on the empty-progress snapshot proof"
    );
    if !first.snapshot.initialized || first.snapshot.queue_root != [0; 32] {
        return Ok(());
    }
    retain_token_root(
        output,
        state,
        target,
        first.source_hash,
        first.snapshot.queue_id,
        [0; 32],
    )
}

#[allow(clippy::too_many_arguments)]
async fn publish_token_roots(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    source: &mut Source,
    witness: &mut Source,
    ethereum: &Ethereum,
    deployment_manifest: &Path,
    output: &Path,
    state: &mut Value,
    reconcile_only: bool,
) -> Result<bool> {
    let mut max_block = ethereum.queue_max_block_number().await?;
    loop {
        if !reconcile_only {
            schedule_empty_progress(source, witness, max_block, output, state).await?;
        }
        let mut pending = state["roots"]
            .as_object()
            .context("missing root registration journal")?
            .iter()
            .filter(|(_, root)| root["status"] == "pending" || root["status"] == "mined")
            .map(|(key, root)| Ok((number(&root["block"])?, key.clone())))
            .collect::<Result<Vec<_>>>()?;
        pending.sort_unstable();
        let active_client = state["activeEthereum"]["client"]
            .as_str()
            .or_else(|| state["deployment"]["ethereum"]["client"].as_str())
            .context("active BEEFY client is missing from follower journal")?;
        let anchor = state["commitments"]
            .as_array()
            .context("missing accepted commitments")?
            .iter()
            .rev()
            .find(|entry| entry["clientAddress"].as_str() == Some(active_client))
            .map(|last| number(&last["block"]))
            .transpose()?;
        let mut uncovered = false;
        let mut progressed = false;
        for (block, key) in pending {
            let registration = &state["roots"][&key];
            let publication = Path::new(
                registration["publication"]
                    .as_str()
                    .context("root publication path missing")?,
            );
            let has_publication = publication.exists();
            if !has_publication
                && (!state["submission"].is_null() || anchor.is_none_or(|anchor| anchor <= block))
            {
                uncovered = true;
                continue;
            }
            if !has_publication && root_requires_empty_progress(max_block, block) {
                continue;
            }
            let publication_state = match crate::tokens::publish_root_locked(
                source_rpc,
                witness_rpc,
                ethereum_rpc,
                wallet,
                deployment_manifest,
                &output.join("state.json"),
                registration,
                publication,
            )
            .await
            {
                Ok(status) => status,
                Err(error)
                    if error
                        .downcast_ref::<ethereum_client::Error>()
                        .is_some_and(|error| {
                            matches!(error, ethereum_client::Error::FinalizedAncestryPending)
                        }) =>
                {
                    state["follower"]["status"] = json!("held-finality");
                    state["follower"]["lastError"] = json!(error.to_string());
                    save(output, state, None)?;
                    return Ok(uncovered);
                }
                Err(error) => return Err(error),
            };
            let status = match publication_state {
                crate::tokens::RootPublicationState::Accepted => "accepted",
                crate::tokens::RootPublicationState::Mined => "mined",
                crate::tokens::RootPublicationState::Pending => {
                    state["roots"][&key]["status"] = json!("pending");
                    state["follower"]["status"] = json!("held-root");
                    state["follower"]["lastError"] = json!(
                        "original signed root remains unmined; HOLD without advancing its anchor"
                    );
                    save(output, state, None)?;
                    return Ok(uncovered);
                }
            };
            max_block = max_block.max(block);
            progressed |= state["roots"][&key]["status"] != status;
            state["roots"][&key]["status"] = json!(status);
            save(output, state, None)?;
        }
        if !progressed {
            if matches!(
                state["follower"]["status"].as_str(),
                Some("held-root" | "held-finality")
            ) {
                state["follower"]["status"] = json!("catching-up");
                state["follower"]["lastError"] = Value::Null;
                save(output, state, None)?;
            }
            return Ok(uncovered);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn follow_tokens(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    root_wallet: &Path,
    deployment_manifest: &Path,
    output: &Path,
    reconcile_once: bool,
    local_rehearsal: bool,
) -> Result<()> {
    ensure!(
        crate::local_source_rpc(source_rpc) && crate::local_source_rpc(witness_rpc),
        "public dev keys: source RPC must be loopback"
    );
    ensure!(
        source_rpc != witness_rpc,
        "two independent source endpoints required"
    );
    ensure!(
        if local_rehearsal {
            crate::local_source_rpc(ethereum_rpc)
        } else {
            ethereum_rpc.starts_with("wss://")
        },
        "Hoodi requires wss; explicit local rehearsal requires a loopback websocket"
    );
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    ensure!(
        deployment["mode"] == "hoodi-token-stack",
        "not a token deployment manifest"
    );
    ensure!(
        deployment["localRehearsal"] == local_rehearsal,
        "deployment rehearsal mode differs from actor mode"
    );
    let _owner = crate::tokens::lock_token_deployment(deployment_manifest)?;
    fs::create_dir_all(output)?;
    let output_directory = fs::canonicalize(output)?;
    let output = output_directory.as_path();
    let state_path = output.join("state.json");
    let resumed = state_path.exists();
    let mut state = if resumed {
        let state = read_state_file(&state_path)?;
        validate_token_follower_journal(&state, &deployment)?;
        state
    } else {
        json!({"schemaVersion":3, "mode":"hoodi-token-follow", "deployment":deployment,
            "activeEthereum":deployment["ethereum"].clone(),
            "status":"following", "commitments":[], "transactions":[], "roots":{},
            "startupSequence":0, "rootScan":null, "localRehearsal":local_rehearsal})
    };
    let selected_ethereum_manifest = &deployment["ethereum"];
    ensure!(
        state["localRehearsal"] == local_rehearsal,
        "cannot change an actor between local rehearsal and live mode"
    );
    let follower_signer = crate::ethereum::hoodi_wallet_address(wallet)?;
    let publisher_signer = crate::ethereum::hoodi_wallet_address(root_wallet)?;
    ensure!(
        follower_signer != publisher_signer,
        "follower and root publisher must use separate accounts"
    );
    let follower_signer = format!("{follower_signer:#x}");
    let publisher_signer = format!("{publisher_signer:#x}");
    ensure!(
        !resumed
            || (state["followerSigner"] == follower_signer
                && state["rootPublisherSigner"] == publisher_signer),
        "actor signing identities changed across restart"
    );
    state["followerSigner"] = json!(follower_signer);
    state["rootPublisherSigner"] = json!(publisher_signer);
    state["startupSequence"] = json!(number(&state["startupSequence"])?
        .checked_add(1)
        .context("actor startup sequence overflow")?);
    state["follower"]["status"] = json!("starting");
    save(output, &mut state, None)?;
    let (mut source, mut witness) = Source::connect_pair(
        GearApi::new(source_rpc, 3).await?,
        GearApi::new(witness_rpc, 3).await?,
    )
    .await?;
    source.validate_attachment(&deployment["anchor"]).await?;
    ensure!(
        source.source_genesis == witness.source_genesis
            && source.bridge_domain == witness.bridge_domain
            && source.mmr_start_block == witness.mmr_start_block
            && source.beefy_activation_block == witness.beefy_activation_block
            && deployment["anchor"]["sourceGenesis"] == bytes(source.source_genesis)
            && deployment["anchor"]["bridgeDomain"] == bytes(source.bridge_domain)
            && deployment["anchor"]["mmrStartBlock"] == source.mmr_start_block
            && deployment["anchor"]["beefyActivationBlock"] == source.beefy_activation_block
            && deployment["ethereum"]["sourceGenesis"] == bytes(source.source_genesis)
            && deployment["ethereum"]["bridgeDomain"] == bytes(source.bridge_domain)
            && deployment["ethereum"]["mmrStartBlock"] == source.mmr_start_block,
        "token client, saved anchor, and independent authorities disagree on source identity"
    );
    let (anchor, first) =
        authenticated_token_anchor(&mut source, &mut witness, &deployment["anchor"]).await?;
    let anchor_block = anchor.block;
    let mut ethereum =
        Ethereum::connect_hoodi(ethereum_rpc, wallet, selected_ethereum_manifest).await?;
    ensure!(
        decode(&deployment["manager"])?.as_slice() == ethereum.receiver_address(),
        "declared ERC20 manager differs from client receiver"
    );
    ethereum
        .verify_token_bindings(array(&deployment["gearManager"])?)
        .await?;
    let bootstrap = bootstrap_record(&anchor, &first)?;
    ensure!(
        state["bootstrap"].is_null() || state["bootstrap"] == bootstrap,
        "saved bootstrap differs from signed source"
    );
    ethereum.transactions = serde_json::from_value(state["transactions"].clone())?;
    if state["bootstrap"].is_null() {
        let checkpoint = ethereum.checkpoint().await?;
        ensure!(
            checkpoint.block == u64::from(anchor_block)
                && checkpoint.root == [0; 32]
                && checkpoint.source_timestamp_ms == first.snapshot.source_timestamp_ms
                && checkpoint.current_id == anchor.current.id
                && checkpoint.current_len == anchor.current.keys.len() as u64
                && checkpoint.current_root == anchor.current.root
                && checkpoint.next_id == anchor.next.id
                && checkpoint.next_len == anchor.next.keys.len() as u64
                && checkpoint.next_root == anchor.next.root,
            "token client bootstrap does not equal authenticated source checkpoint"
        );
        state["bootstrap"] = bootstrap;
        let cursor = u32::try_from(source.mmr_start_block)?.saturating_sub(1);
        let cursor_hash = source.api.block_number_to_hash(cursor).await?;
        ensure!(
            witness.api.block_number_to_hash(cursor).await? == cursor_hash,
            "source and witness disagree at initial scan cursor"
        );
        state["rootScan"] = json!({"block":cursor, "blockHash":bytes(cursor_hash.0)});
        save(output, &mut state, Some(&ethereum))?;
    }
    let legacy_client = deployment_client(&state)?;
    if !state["submission"].is_null() {
        require_submission_client(&state["submission"], ethereum.client_address, legacy_client)?;
    }
    let mut last = client_bootstrap(&state, ethereum.client_address)?;
    for entry in state["commitments"]
        .as_array()
        .context("missing token commitment journal")?
        .iter()
        .rev()
    {
        let matches = match entry["clientAddress"].as_str() {
            Some(recorded) => recorded.parse::<Address>()? == ethereum.client_address,
            None => ethereum.client_address == legacy_client,
        };
        if matches {
            last = entry;
            break;
        }
    }
    let start: u32 = number(&last["block"])?.try_into()?;
    source.seek_from(start)?;
    source
        .enable_scan_journal(&output.join("source-scan.json"), start, &witness)
        .await?;
    if reconcile_once && !state["submission"].is_null() {
        let pending = number(&state["submission"]["block"])?;
        ensure!(
            ethereum.checkpoint().await?.block == pending,
            "reconcile-once requires pending submission already accepted by Hoodi"
        );
    }
    phase(
        output,
        &mut state,
        Some(&ethereum),
        "following isolated token client",
    )?;
    let result: Result<()> = async {
        tokio::time::timeout(Duration::from_secs(300), async {
            revalidate_finalized_commitments(
                &source,
                &witness,
                &ethereum,
                &state,
                &output.join("finality-history"),
            )
            .await?;
            promote_finality(&mut source, &mut ethereum, output, &mut state).await?;
            if !state["submission"].is_null() {
                advance(&mut source, &mut ethereum, output, &mut state, false, None).await?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context(
            "startup history authentication exceeded its original 300-second deadline; HOLD",
        )??;
        if reconcile_once {
            finish_commitment_finality(&mut source, &mut ethereum, output, &mut state).await?;
            loop {
                let roots = state["roots"].as_object().context("missing root journal")?;
                ensure!(
                    roots.values().all(|root| matches!(
                        root["status"].as_str(),
                        Some("pending" | "mined" | "accepted")
                    )),
                    "invalid root reconciliation status"
                );
                if roots.values().all(|root| root["status"] == "accepted") {
                    break;
                }
                let uncovered = publish_token_roots(
                    source_rpc,
                    witness_rpc,
                    ethereum_rpc,
                    root_wallet,
                    &mut source,
                    &mut witness,
                    &ethereum,
                    deployment_manifest,
                    output,
                    &mut state,
                    true,
                )
                .await?;
                ensure!(
                    !uncovered,
                    "pending root requires a newer checkpoint; resume the normal follower"
                );
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            return Ok(());
        }
        scan_token_roots(&source, &witness, output, &mut state).await?;
        publish_token_roots(
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            root_wallet,
            &mut source,
            &mut witness,
            &ethereum,
            deployment_manifest,
            output,
            &mut state,
            false,
        )
        .await?;

        loop {
            promote_finality(&mut source, &mut ethereum, output, &mut state).await?;
            scan_token_roots(&source, &witness, output, &mut state).await?;
            let uncovered = publish_token_roots(
                source_rpc,
                witness_rpc,
                ethereum_rpc,
                root_wallet,
                &mut source,
                &mut witness,
                &ethereum,
                deployment_manifest,
                output,
                &mut state,
                false,
            )
            .await?;
            if matches!(
                state["follower"]["status"].as_str(),
                Some("held-root" | "held-finality")
            ) {
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
            let handover_only = !uncovered && state["follower"]["status"] == "healthy";
            advance(
                &mut source,
                &mut ethereum,
                output,
                &mut state,
                handover_only,
                Some(&witness),
            )
            .await?;
        }
    }
    .await;
    if let Err(error) = &result {
        state["follower"]["status"] = json!("failed");
        state["follower"]["lastError"] = json!(format!("{error:#}"));
        save(output, &mut state, Some(&ethereum))?;
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use gear_rpc_client::dto::MerkleProof;

    #[test]
    fn idle_client_refreshes_before_expiry_without_a_root_or_handover() {
        let source = 1_000_000;
        assert!(!client_heartbeat_due(source, source - 1));
        assert!(!client_heartbeat_due(source, source + 43_199_999));
        assert!(client_heartbeat_due(source, source + 43_200_000));
        assert!(client_heartbeat_due(source, source + 86_400_000));
    }

    #[test]
    fn empty_progress_crosses_long_gaps_without_skipping_pending_roots() {
        let mut max = 1;
        let mut steps = 0;
        while let Some(next) = next_empty_progress_block(max, 200_000, 200_001, Some(200_000)) {
            assert!(u64::from(next) > max && u64::from(next) - max <= EMPTY_PROGRESS_MAX_DISTANCE);
            max = u64::from(next);
            steps += 1;
        }
        assert_eq!((steps, max), (3, 172_801));
        assert!(!root_requires_empty_progress(max, 200_000));
        assert!(!root_requires_empty_progress(0, 200_000));
        assert!(!root_requires_empty_progress(1, 57_601));
        assert!(root_requires_empty_progress(1, 57_602));
        assert_eq!(next_empty_progress_block(0, 200_000, 200_001, None), None);
        assert_eq!(
            next_empty_progress_block(1, 57_601, 57_601, None),
            Some(57_600)
        );
        assert_eq!(next_empty_progress_block(1, 57_600, 100_000, None), None);
        assert_eq!(next_empty_progress_block(1, 100_000, 0, None), None);
        assert_eq!(
            next_empty_progress_block(1, 100_000, 100_001, Some(57_601)),
            None
        );
        assert_eq!(
            next_empty_progress_block(u64::MAX, u32::MAX, u32::MAX, None),
            None
        );
    }

    #[test]
    fn journal_cannot_reuse_another_client_submission() -> Result<()> {
        let legacy = Address::from([0x11; 20]);
        let candidate = Address::from([0x22; 20]);
        let old = json!({"block": 420, "clientAddress": format!("{legacy:#x}")});
        let new = json!({"block": 420, "clientAddress": format!("{candidate:#x}")});

        assert_eq!(journal_client(&old, legacy)?, legacy);
        assert_eq!(journal_client(&new, legacy)?, candidate);
        require_submission_client(&old, legacy, legacy)?;
        assert!(require_submission_client(&old, candidate, legacy).is_err());
        assert!(require_submission_client(&json!({"nonce": 1}), candidate, legacy).is_err());
        assert!(journal_client(&json!({"clientAddress": 7}), legacy).is_err());
        let state = json!({"deployment":{"ethereum":{"client":format!("{legacy:#x}")}},
            "bootstrap":{"block":4}});
        assert_eq!(client_bootstrap(&state, legacy)?["block"], 4);
        assert!(client_bootstrap(&state, candidate).is_err());
        Ok(())
    }

    #[test]
    fn persisted_registration_survives_cursor_replay_without_replacing_publication() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut state = json!({"roots":{}, "rootScan":{"block":41, "blockHash":bytes([1;32])}});
        retain_token_root(directory.path(), &mut state, 42, [2; 32], 7, [3; 32])?;
        let mut restored = read_state_file(&directory.path().join("state.json"))?;
        let key = format!("42-{}", hex::encode([3; 32]));
        assert_eq!(restored["rootScan"]["block"], 41);
        assert_eq!(restored["roots"][&key]["queueId"], 7);
        assert_eq!(restored["roots"][&key]["status"], "pending");
        let publication = directory
            .path()
            .join("root-publications")
            .join(format!("{key}.json"));
        fs::create_dir_all(publication.parent().expect("publication parent"))?;
        fs::write(&publication, br#"{"nonce":"4","txHash":"0xsaved"}"#)?;
        retain_token_root(directory.path(), &mut restored, 42, [2; 32], 7, [3; 32])?;
        assert_eq!(
            fs::read(&publication)?,
            br#"{"nonce":"4","txHash":"0xsaved"}"#
        );
        assert_eq!(restored["roots"].as_object().expect("roots").len(), 1);
        restored["roots"][&key]["status"] = json!("accepted");
        retain_token_root(directory.path(), &mut restored, 42, [2; 32], 7, [3; 32])?;
        assert_eq!(restored["roots"][&key]["status"], "accepted");
        retain_token_root(directory.path(), &mut restored, 43, [4; 32], 7, [3; 32])?;
        assert_eq!(restored["roots"].as_object().expect("roots").len(), 2);
        assert!(
            retain_token_root(directory.path(), &mut restored, 42, [9; 32], 7, [3; 32]).is_err()
        );
        assert!(
            retain_token_root(directory.path(), &mut restored, 42, [2; 32], 7, [9; 32]).is_err()
        );
        assert_eq!(
            read_state_file(&directory.path().join("state.json"))?,
            restored
        );
        Ok(())
    }

    #[test]
    fn checkpoint_target_tracks_latest_unpublished_registration() -> Result<()> {
        let mut state = json!({"roots": {
            "older": {"block": 41, "status": "pending"},
            "latest": {"block": 43, "status": "pending"},
            "accepted": {"block": 99, "status": "accepted"},
        }});
        assert_eq!(pending_token_root_block(&state)?, Some(43));
        state["roots"]["latest"]["status"] = json!("accepted");
        assert_eq!(pending_token_root_block(&state)?, Some(41));
        state["roots"]["older"]["status"] = json!("accepted");
        assert_eq!(pending_token_root_block(&state)?, None);
        state["roots"]["missing"] = json!({"status": "pending"});
        assert!(pending_token_root_block(&state).is_err());
        Ok(())
    }

    #[test]
    fn follower_journal_rejects_old_schema_and_changed_deployment() {
        let deployment = json!({
            "mode": "hoodi-token-stack",
            "ethereum": {"chainId": 560048},
        });
        let valid = json!({
            "schemaVersion": 3,
            "mode": "hoodi-token-follow",
            "deployment": deployment.clone(),
            "activeEthereum": deployment["ethereum"].clone(),
        });
        assert!(validate_token_follower_journal(&valid, &deployment).is_ok());
        let mut changed_client = valid.clone();
        changed_client["activeEthereum"] = json!({"chainId": 31_337});
        assert!(validate_token_follower_journal(&changed_client, &deployment).is_err());

        let old_schema = json!({
            "schemaVersion": 2,
            "mode": "hoodi-token-follow",
            "deployment": deployment.clone(),
        });
        assert!(validate_token_follower_journal(&old_schema, &deployment).is_err());

        let other_deployment = json!({
            "mode": "hoodi-token-stack",
            "ethereum": {"chainId": 31_337},
        });
        let changed = json!({
            "schemaVersion": 3,
            "mode": "hoodi-token-follow",
            "deployment": other_deployment,
        });
        assert!(validate_token_follower_journal(&changed, &deployment).is_err());
    }

    #[tokio::test]
    #[ignore = "requires archived Gear RPC and real Hoodi follower history; no signer"]
    async fn live_finality_replay_preserves_original_transactions() -> Result<()> {
        let rpc = std::env::var("BEEFY_TEST_SOURCE_RPC")?;
        let ethereum_rpc = std::env::var("BEEFY_TEST_ETHEREUM_RPC")?;
        let journal = std::env::var("BEEFY_TEST_FINALITY_STATE")?;
        let mut state = read_state_file(Path::new(&journal))?;
        let mut entries: Vec<_> = state["commitments"]
            .as_array()
            .context("commitments")?
            .iter()
            .filter(|entry| entry["finalized"] == true)
            .rev()
            .take(3)
            .cloned()
            .collect();
        ensure!(
            entries.len() == 3,
            "fixture needs three finalized original transactions"
        );
        entries.reverse();
        let originals = entries.clone();
        for entry in &mut entries {
            entry["finalized"] = json!(false);
        }
        state["commitments"] = json!(entries);
        let mut source = Source::connect(GearApi::new(&rpc, 3).await?).await?;
        let mut ethereum =
            Ethereum::connect_hoodi_readonly(&ethereum_rpc, &state["activeEthereum"]).await?;
        ethereum.transactions = state["transactions"]
            .as_array()
            .context("transactions")?
            .clone();
        let directory = tempfile::tempdir()?;
        save(directory.path(), &mut state, None)?;
        let saved = fs::read(directory.path().join("state.json"))?;
        let mut forged = state.clone();
        forged["commitments"][0]["submission"]["txHash"] = json!(bytes([0u8; 32]));
        let before = forged.clone();
        ensure!(
            promote_finality(&mut source, &mut ethereum, directory.path(), &mut forged)
                .await
                .is_err()
                && forged == before
                && fs::read(directory.path().join("state.json"))? == saved,
            "forged signed identity changed pending history"
        );
        let started = std::time::Instant::now();
        promote_finality(&mut source, &mut ethereum, directory.path(), &mut state).await?;
        for (entry, original) in state["commitments"]
            .as_array()
            .context("commitments")?
            .iter()
            .zip(&originals)
        {
            ensure!(
                entry["finalized"] == true,
                "original inclusion was not promoted"
            );
            for key in [
                "block",
                "txHash",
                "submission",
                "destinationBlock",
                "destinationHash",
                "firstInclusion",
            ] {
                ensure!(
                    entry[key] == original[key],
                    "promotion changed original {key}"
                );
            }
        }
        ensure!(
            state["follower"]["lastFinalizedUpdate"] == originals[2]["block"]
                && read_state_file(&directory.path().join("state.json"))? == state,
            "promotion lost ordered durable progress"
        );
        println!(
            "{}",
            json!({"status":"authenticated", "originalTransactions":3,
            "elapsedMs":started.elapsed().as_millis(), "signer":false, "writesOriginalJournal":false})
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires archived Gear RPC, Foundry, and owned Anvil"]
    async fn interrupted_commitment_recovers_without_replacement() -> Result<()> {
        use alloy::{eips::Encodable2718, providers::WalletProvider};
        let rpc = std::env::var("BEEFY_TEST_SOURCE_RPC")?;
        let witness_rpc = std::env::var("BEEFY_TEST_WITNESS_RPC")?;
        let journal = std::env::var("BEEFY_TEST_SOURCE_JOURNAL")?;
        let recorded: Value = serde_json::from_slice(&fs::read(&journal)?)?;
        let bootstrap = &recorded["commitments"][0];
        let evidence = Path::new(&journal).with_extension("recovery");
        fs::create_dir(&evidence).context("native recovery evidence directory must be new")?;
        let directory = evidence.as_path();
        let (mut source, witness) = Source::connect_pair(
            GearApi::new(&rpc, 3).await?,
            GearApi::new(&witness_rpc, 3).await?,
        )
        .await?;
        let initial = source
            .recapture(
                number(&bootstrap["block"])?.try_into()?,
                decode(&bootstrap["rawScale"])?,
            )
            .await?;
        let initial_proof = source.proof(initial.block - 1, &initial).await?;
        let mut ethereum = Ethereum::prepare(
            directory,
            &initial,
            &initial_proof.snapshot,
            source.mmr_start_block,
        )
        .await?;
        source::initialize_bridge(&source.api).await?;
        let root_message = source::send_message(
            &source.api,
            ethereum.receiver_address(),
            b"native-root-recovery",
            "//Charlie",
        )
        .await?;
        let provider = ethereum.api.raw_provider().clone();
        provider
            .client()
            .request::<_, Value>("anvil_setIntervalMining", (0u64,))
            .await?;
        provider
            .client()
            .request::<_, Value>("evm_setAutomine", (true,))
            .await?;
        let deployment = json!({"anchor":{"sourceGenesis":bytes(source.source_genesis), "bridgeDomain":bytes(source.bridge_domain)}, "ethereum":ethereum.manifest().await?});
        let mut state = json!({"mode":"hoodi-message-demo", "sourceGenesis":bytes(source.source_genesis), "bridgeDomain":bytes(source.bridge_domain),
            "ethereum":deployment["ethereum"], "deployment":deployment, "bootstrap": bootstrap, "commitments": [], "transactions": ethereum.transactions, "submission": null});
        let mut finalized_reorg_snapshot = None;
        for interruption in 0..3 {
            let before = ethereum.checkpoint().await?.block;
            let anchor = loop {
                let candidate = source.next_commitment().await?;
                if u64::from(candidate.block) > before && candidate.block > root_message.block {
                    break candidate;
                }
            };
            if interruption == 2 {
                finalized_reorg_snapshot = Some(
                    provider
                        .client()
                        .request::<_, Value>("evm_snapshot", ())
                        .await?,
                );
            }
            let proof = source.proof(anchor.block - 1, &anchor).await?;
            state["submission"] = json!({"blockHash": bytes(anchor.block_hash), "root": bytes(anchor.validated.mmr_root)});
            let interrupted = ethereum
                .submit_commitment(
                    &anchor,
                    &proof.leaf,
                    &proof.snapshot,
                    &proof.simplified,
                    None,
                    |transaction, receipt| {
                        for (key, value) in transaction
                            .as_object()
                            .context("missing transaction fields")?
                        {
                            state["submission"][key] = value.clone();
                        }
                        save(directory, &mut state, None)?;
                        if (interruption == 0 && transaction["txHash"].is_null())
                            || (interruption == 1
                                && !transaction["txHash"].is_null()
                                && receipt.is_none())
                            || (interruption == 2 && receipt.is_some())
                        {
                            bail!("injected interruption at stage {interruption}");
                        }
                        Ok(())
                    },
                )
                .await;
            ensure!(
                interrupted
                    .as_ref()
                    .is_err_and(|error| error.to_string().contains("injected interruption")),
                "submission did not reach interruption stage {interruption}: {interrupted:?}"
            );
            let nonce = state["submission"]["nonce"]
                .as_u64()
                .context("nonce was not persisted")?;
            let saved_hash = state["submission"]["txHash"].as_str().map(str::to_owned);
            let sender = ethereum.api.raw_provider().default_signer_address();
            ensure!(
                ethereum
                    .api
                    .raw_provider()
                    .get_transaction_count(sender)
                    .await?
                    == nonce + u64::from(interruption == 2),
                "interruption consumed an unexpected nonce"
            );
            if interruption == 1 {
                let hash = saved_hash
                    .as_ref()
                    .context("signed transaction hash missing")?
                    .parse()?;
                ensure!(
                    ethereum
                        .api
                        .raw_provider()
                        .get_transaction_by_hash(hash)
                        .await?
                        .is_none(),
                    "transaction was broadcast before its signed bytes were durable"
                );
            }
            let mut restarted = Source::connect(GearApi::new(&rpc, 3).await?).await?;
            let mut restored: Value =
                serde_json::from_slice(&fs::read(directory.join("state.json"))?)?;
            let recovered = advance(
                &mut restarted,
                &mut ethereum,
                directory,
                &mut restored,
                false,
                None,
            )
            .await?;
            ensure!(
                recovered.block == anchor.block && restored["submission"].is_null(),
                "restart did not reconcile original commitment"
            );
            let commitments = restored["commitments"]
                .as_array()
                .context("missing commitments")?;
            ensure!(
                commitments.len() == interruption + 1,
                "recovery duplicated commitment"
            );
            let hash: [u8; 32] = array(&commitments[interruption]["txHash"])?;
            if let Some(saved_hash) = saved_hash {
                ensure!(
                    saved_hash == bytes(hash),
                    "recovery replaced the signed transaction"
                );
            }
            let original = ethereum
                .verify_accepted_commitment(hash.into(), &anchor, &proof.snapshot)
                .await?;
            ensure!(
                commitments[interruption]["destinationBlock"] == original.0
                    && commitments[interruption]["destinationHash"] == bytes(original.1),
                "recovery changed canonical receipt evidence"
            );
            ensure!(
                ethereum
                    .api
                    .raw_provider()
                    .get_transaction_count(sender)
                    .await?
                    == nonce + 1,
                "recovery allocated a replacement nonce"
            );
            ensure!(
                restored["transactions"]
                    .as_array()
                    .context("missing receipts")?
                    .iter()
                    .filter(|tx| tx["txHash"] == bytes(hash))
                    .count()
                    == 1,
                "recovery must retain exactly one authentic receipt"
            );
            ensure!(
                restored["commitments"][interruption]["finalized"] == false
                    && restored["follower"]["lastMinedUpdate"] == anchor.block
                    && restored["follower"]["lastSuccessfulUpdate"] != anchor.block,
                "mined recovery was incorrectly reported as finalized completion"
            );
            state = restored;
        }
        ensure!(
            state["commitments"]
                .as_array()
                .context("commitments")?
                .iter()
                .filter(|entry| entry["finalized"] == false)
                .count()
                >= 2,
            "successive commitments did not pipeline before finality"
        );
        let last_hash: [u8; 32] = array(&state["commitments"][2]["txHash"])?;
        ensure!(
            ethereum
                .api
                .get_finalized_receipt(last_hash.into())
                .await?
                .is_none(),
            "fixture did not preserve a genuinely unfinalized commitment"
        );
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x3",))
            .await?;
        finish_commitment_finality(&mut source, &mut ethereum, directory, &mut state).await?;
        ensure!(
            state["commitments"]
                .as_array()
                .context("commitments")?
                .iter()
                .all(|entry| entry["finalized"] == true)
                && state["follower"]["lastFinalizedUpdate"] == state["follower"]["lastMinedUpdate"]
                && state["follower"]["lastSuccessfulUpdate"]
                    == state["follower"]["lastFinalizedUpdate"],
            "canonical finality did not promote the original commitments"
        );
        revalidate_finalized_commitments(
            &source,
            &witness,
            &ethereum,
            &state,
            &directory.join("finality-history"),
        )
        .await?;
        let root_anchor = source
            .recapture(
                number(&state["commitments"][2]["block"])?.try_into()?,
                decode(&state["commitments"][2]["rawScale"])?,
            )
            .await?;
        let root_block = root_message.block;
        let root_proof = source.proof(root_block, &root_anchor).await?;
        let expected_root = root_proof.snapshot.queue_root;
        let encoded = crate::rehearsal::queue_envelope(&root_proof, &root_anchor, &root_message)?;
        let root_signer =
            alloy::signers::local::PrivateKeySigner::from_bytes(&B256::from([19; 32]))?;
        let root_sender = root_signer.address();
        provider
            .client()
            .request::<_, Value>("anvil_setBalance", (root_sender, "0xde0b6b3a7640000"))
            .await?;
        let root_provider = alloy::providers::ProviderBuilder::new()
            .wallet(alloy::network::EthereumWallet::from(root_signer))
            .connect_provider(provider.root().clone());
        let queue =
            ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, root_provider.clone());
        let request = queue
            .submitMerkleRoot(
                alloy::primitives::U256::from(root_block),
                B256::from(expected_root),
                encoded.into(),
            )
            .nonce(0)
            .into_transaction_request();
        let signed = root_provider.fill(request).await?;
        let raw_root = signed
            .as_envelope()
            .context("root transaction must be signed")?
            .encoded_2718();
        let root_hash = B256::from(beefy_relay::keccak256(&raw_root));
        let root_path = directory.join("root-publications").join("original.json");
        fs::create_dir_all(root_path.parent().context("root publication directory")?)?;
        let mut root_intent = json!({
            "schemaVersion":3, "sourceBlock":root_block, "root":bytes(expected_root), "kind":"merkleRoot",
            "sourceIdentity":{"destinationQueue":format!("{:#x}",ethereum.queue_address)},
            "acceptedAnchorClient":format!("{:#x}",ethereum.client_address),
            "acceptedAnchorTx":state["commitments"][2]["txHash"],
            "acceptedCheckpoint":{"block":state["commitments"][2]["destinationBlock"], "blockHash":state["commitments"][2]["destinationHash"]},
            "sender":format!("{root_sender:#x}"), "nonce":"0",
            "rawTransaction":format!("0x{}",hex::encode(&raw_root)), "txHash":format!("{root_hash:#x}"),
            "publicationReceipt":null
        });
        let roots = json!({"original":{
            "status":"pending", "publication":root_path, "block":root_block,
            "kind":"merkleRoot", "queueRoot":bytes(expected_root)
        }});
        fs::write(&root_path, serde_json::to_vec(&root_intent)?)?;
        ensure!(
            crate::tokens::ensure_mined_root_publications(&ethereum, &roots)
                .await
                .is_err(),
            "signed but unmined root must hold subsequent commitments"
        );
        ensure!(
            root_provider.get_transaction_count(root_sender).await? == 0,
            "barrier broadcast a pending root itself"
        );
        let root_snapshot: Value = provider.client().request("evm_snapshot", ()).await?;
        let receipt = root_provider
            .send_raw_transaction(&raw_root)
            .await?
            .get_receipt()
            .await?;
        ensure!(
            receipt.status() && receipt.transaction_hash == root_hash,
            "original root did not mine successfully"
        );
        root_intent["publicationReceipt"] = json!({"block":receipt.block_number.context("root block")?, "blockHash":format!("{:#x}",receipt.block_hash.context("root block hash")?), "kind":"merkleRoot", "finalized":false});
        fs::write(&root_path, serde_json::to_vec(&root_intent)?)?;
        let original_root_intent = fs::read(&root_path)?;
        ensure!(
            ethereum
                .api
                .get_finalized_receipt(root_hash)
                .await?
                .is_none(),
            "root fixture finalized before testing the mined barrier"
        );
        crate::tokens::ensure_mined_root_publications(&ethereum, &roots).await?;
        ensure!(
            fs::read(&root_path)? == original_root_intent,
            "mined barrier changed original publication evidence"
        );
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x3",))
            .await?;
        let finalized_root = ethereum
            .api
            .get_finalized_receipt(root_hash)
            .await?
            .context("original root did not reach canonical finality")?;
        ensure!(
            finalized_root.receipt.transaction_hash == root_hash
                && finalized_root.receipt.block_hash == receipt.block_hash,
            "finality replaced root identity or inclusion"
        );
        crate::tokens::ensure_mined_root_publications(&ethereum, &roots).await?;
        state["roots"] = roots.clone();

        let snapshot: Value = provider.client().request("evm_snapshot", ()).await?;
        let anchor = source.next_commitment().await?;
        let proof = source.proof(anchor.block - 1, &anchor).await?;
        ethereum
            .submit_commitment(
                &anchor,
                &proof.leaf,
                &proof.snapshot,
                &proof.simplified,
                None,
                |transaction, _| {
                    state["submission"] = transaction.clone();
                    state["submission"]["blockHash"] = json!(bytes(anchor.block_hash));
                    state["submission"]["root"] = json!(bytes(anchor.validated.mmr_root));
                    save(directory, &mut state, None)
                },
            )
            .await?;
        state["transactions"] = json!(ethereum.transactions);
        save(directory, &mut state, None)?;
        let interrupted_after_receipt = state.clone();
        let hash: [u8; 32] = array(&state["submission"]["txHash"])?;
        let nonce = number(&state["submission"]["nonce"])?;
        record_mined_commitment(&mut ethereum, directory, &mut state, &anchor, &proof, hash)
            .await?;
        let before_reorg = state.clone();
        let reverted: bool = provider.client().request("evm_revert", (snapshot,)).await?;
        ensure!(reverted, "owned Anvil snapshot did not revert");
        ensure!(
            promote_finality(&mut source, &mut ethereum, directory, &mut state)
                .await
                .is_err(),
            "reorged pending commitment must hold instead of promoting or replacing"
        );
        ensure!(
            state == before_reorg
                && provider
                    .get_transaction_count(provider.default_signer_address())
                    .await?
                    == nonce,
            "reorg handling changed original evidence or broadcast another transaction"
        );
        let mut interrupted = interrupted_after_receipt.clone();
        ensure!(
            record_mined_commitment(
                &mut ethereum,
                directory,
                &mut interrupted,
                &anchor,
                &proof,
                hash
            )
            .await
            .is_err(),
            "persisted mined receipt disappeared: recovery must hold without rebroadcast"
        );
        ensure!(
            interrupted == interrupted_after_receipt
                && provider
                    .get_transaction_count(provider.default_signer_address())
                    .await?
                    == nonce,
            "receipt-only interruption changed evidence or rebroadcast a reorged transaction"
        );
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x1",))
            .await?;
        let raw = decode(&interrupted["submission"]["rawTransaction"])?;
        provider
            .send_raw_transaction(&raw)
            .await?
            .get_receipt()
            .await?;
        ensure!(
            record_mined_commitment(
                &mut ethereum,
                directory,
                &mut interrupted,
                &anchor,
                &proof,
                hash
            )
            .await
            .is_err(),
            "persisted mined receipt was silently repinned after reinclusion"
        );
        ensure!(
            interrupted == interrupted_after_receipt,
            "reinclusion changed original evidence"
        );
        ensure!(
            promote_finality(&mut source, &mut ethereum, directory, &mut state)
                .await
                .is_err()
                && state == before_reorg,
            "unfinalized reinclusion must hold without changing original evidence"
        );
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x3",))
            .await?;
        let original_commitment = before_reorg["commitments"]
            .as_array()
            .context("commitments")?
            .last()
            .context("original commitment")?;
        let pin_block = root_block + 1;
        let pin_proof = source.proof(pin_block, &anchor).await?;
        let pin_encoded = beefy_relay::encode_queue_proof(
            &pin_proof.snapshot,
            u64::from(anchor.block),
            anchor.validated.mmr_root,
            &pin_proof.leaf,
            &pin_proof.simplified.items,
            pin_proof.simplified.proof_order,
        )?;
        let pin_kind = if pin_proof.snapshot.queue_root == [0; 32] {
            "emptyProgress"
        } else {
            "merkleRoot"
        };
        let pin_path = directory
            .join("root-publications")
            .join("pinned-reinclusion.json");
        let pin_intent = json!({"schemaVersion":3,"sourceBlock":pin_block,"root":bytes(pin_proof.snapshot.queue_root),"kind":pin_kind,
            "localRehearsal":true,"sourceIdentity":{"sourceGenesis":bytes(source.source_genesis),"bridgeDomain":bytes(source.bridge_domain),
                "sourceDomain":state["deployment"]["ethereum"]["sourceDomain"],"destinationChainId":31337,"destinationQueue":format!("{:#x}",ethereum.queue_address)},
            "acceptedAnchorTx":bytes(hash),"acceptedAnchorClient":format!("{:#x}",ethereum.client_address),
            "acceptedCheckpoint":{"block":original_commitment["destinationBlock"],"blockHash":original_commitment["destinationHash"]},
            "sender":format!("{root_sender:#x}"),"proof":crate::rehearsal::proof_evidence(&pin_proof,&anchor,&pin_encoded),"queueProof":bytes(&pin_encoded),
            "nonce":null,"rawTransaction":null,"txHash":null,"publicationReceipt":null});
        let pinned_bytes = serde_json::to_vec(&pin_intent)?;
        fs::write(&pin_path, &pinned_bytes)?;
        let original_disk_state = fs::read(directory.join("state.json"))?;
        for status in ["pending", "mined", "accepted"] {
            let mut held = before_reorg.clone();
            held["roots"]["pinned"] = json!({"block":pin_block,"kind":pin_kind,"queueRoot":bytes(pin_proof.snapshot.queue_root),"status":status,"publication":pin_path});
            let original = held.clone();
            ensure!(
                promote_finality(&mut source, &mut ethereum, directory, &mut held)
                    .await
                    .is_err()
                    && held == original
                    && fs::read(&pin_path)? == pinned_bytes
                    && fs::read(directory.join("state.json"))? == original_disk_state,
                "reinclusion changed a pinned root checkpoint or original commitment evidence"
            );
        }
        let mut unindexed = before_reorg.clone();
        ensure!(
            promote_finality(&mut source, &mut ethereum, directory, &mut unindexed)
                .await
                .is_err()
                && unindexed == before_reorg,
            "reinclusion ignored an unindexed publication"
        );
        let mut missing_inventory = before_reorg.clone();
        missing_inventory["mode"] = json!("hoodi-token-follow");
        missing_inventory
            .as_object_mut()
            .context("state")?
            .remove("roots");
        let original = missing_inventory.clone();
        ensure!(
            promote_finality(
                &mut source,
                &mut ethereum,
                directory,
                &mut missing_inventory
            )
            .await
            .is_err()
                && missing_inventory == original,
            "reinclusion accepted a missing token publication inventory"
        );
        // Retain the negative fixture outside the active inventory before the independent positive case.
        fs::rename(&pin_path, directory.join("rejected-root-pin.json"))?;

        promote_finality(&mut source, &mut ethereum, directory, &mut state).await?;
        let recovered = state["commitments"]
            .as_array()
            .context("commitments")?
            .last()
            .context("recovered commitment")?
            .clone();
        let previous = before_reorg["commitments"]
            .as_array()
            .context("commitments")?
            .last()
            .context("original commitment")?;
        ensure!(
            recovered["finalized"] == true
                && recovered["txHash"] == previous["txHash"]
                && recovered["submission"] == previous["submission"]
                && recovered["firstInclusion"]["blockNumber"] == previous["destinationBlock"]
                && recovered["firstInclusion"]["blockHash"] == previous["destinationHash"]
                && recovered["destinationHash"] != previous["destinationHash"],
            "finalized reinclusion must preserve the original signed identity and first observation"
        );
        let promoted = state.clone();
        promote_finality(&mut source, &mut ethereum, directory, &mut state).await?;
        ensure!(
            state == promoted,
            "repeated finality promotion replaced first evidence"
        );
        revalidate_finalized_commitments(
            &source,
            &witness,
            &ethereum,
            &state,
            &directory.join("finality-history"),
        )
        .await?;
        record_mined_commitment(
            &mut ethereum,
            directory,
            &mut interrupted,
            &anchor,
            &proof,
            hash,
        )
        .await?;
        promote_finality(&mut source, &mut ethereum, directory, &mut interrupted).await?;
        ensure!(
            interrupted["commitments"]
                .as_array()
                .context("commitments")?
                .last()
                == Some(&recovered)
                && provider
                    .get_transaction_count(provider.default_signer_address())
                    .await?
                    == nonce + 1,
            "receipt-only recovery changed the original transaction or lost reinclusion evidence"
        );

        let reverted: bool = provider
            .client()
            .request("evm_revert", (root_snapshot,))
            .await?;
        ensure!(
            reverted
                && crate::tokens::ensure_mined_root_publications(&ethereum, &roots)
                    .await
                    .is_err(),
            "disappeared root inclusion must hold the next commitment"
        );
        ensure!(
            root_provider.get_transaction_count(root_sender).await? == 0
                && fs::read(&root_path)? == original_root_intent,
            "root reorg replaced a nonce or rewrote evidence"
        );
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x1",))
            .await?;
        root_provider
            .send_raw_transaction(&raw_root)
            .await?
            .get_receipt()
            .await?;
        ensure!(
            crate::tokens::ensure_mined_root_publications(&ethereum, &roots)
                .await
                .is_err(),
            "re-included root silently changed its pinned inclusion"
        );
        ensure!(
            fs::read(&root_path)? == original_root_intent,
            "root reinclusion overwrote original evidence"
        );

        let original = state.clone();
        ensure!(
            provider
                .client()
                .request::<_, bool>(
                    "evm_revert",
                    (finalized_reorg_snapshot.context("finalized reorg snapshot missing")?,)
                )
                .await?,
            "finalized reorg snapshot did not revert"
        );
        ensure!(
            revalidate_finalized_commitments(
                &source,
                &witness,
                &ethereum,
                &state,
                &directory.join("finality-history")
            )
            .await
            .is_err(),
            "restart trusted a reorged finalized commitment"
        );
        ensure!(
            state == original,
            "finalized reorg changed the original journal"
        );
        Ok(())
    }
    #[test]
    fn matching_block_requires_all_authenticated_checkpoint_fields() {
        let current = AuthoritySet {
            id: 7,
            keys: vec![vec![1]],
            root: [2; 32],
        };
        let next = AuthoritySet {
            id: 8,
            keys: vec![vec![3], vec![4]],
            root: [5; 32],
        };
        let checkpoint = DestinationCheckpoint {
            block: 10,
            root: [6; 32],
            source_timestamp_ms: 1234,
            current_id: 7,
            current_len: 1,
            current_root: [2; 32],
            next_id: 8,
            next_len: 2,
            next_root: [5; 32],
        };
        assert!(checkpoint_matches_source(
            &checkpoint,
            [6; 32],
            1234,
            &current,
            &next
        ));
        assert!(!checkpoint_matches_source(
            &checkpoint,
            [0; 32],
            1234,
            &current,
            &next
        ));
        assert!(!checkpoint_matches_source(
            &checkpoint,
            [6; 32],
            1235,
            &current,
            &next
        ));
        for field in 0..6 {
            let mut changed = checkpoint.clone();
            match field {
                0 => changed.current_id += 1,
                1 => changed.current_len += 1,
                2 => changed.current_root = [0; 32],
                3 => changed.next_id += 1,
                4 => changed.next_len += 1,
                _ => changed.next_root = [0; 32],
            }
            assert!(!checkpoint_matches_source(
                &changed, [6; 32], 1234, &current, &next
            ));
        }
    }

    #[test]
    fn retained_message_survives_restart_and_rejects_payload_corruption() {
        let message = Message {
            nonce_be: [1; 32],
            source: [2; 32],
            destination: [3; 20],
            payload: b"retained".to_vec(),
        };
        let message_hash = crate::message_hash(&message);
        let original = ObservedMessage {
            block: 42,
            block_hash: [4; 32],
            message,
            message_hash,
            inclusion: MerkleProof {
                root: message_hash,
                proof: vec![],
                num_leaves: 1,
                leaf_index: 0,
            },
            snapshot: QueueSnapshot::new([5; 32], 1_800_000_000_000, true, 6, message_hash)
                .unwrap(),
            retained_at_block: 42,
        };
        let directory = tempfile::tempdir().unwrap();
        let mut state = json!({"messages": [message_evidence(&original)]});
        save(directory.path(), &mut state, None).unwrap();
        let saved: Value =
            serde_json::from_slice(&fs::read(directory.path().join("state.json")).unwrap())
                .unwrap();
        let restored = restore_message(&saved["messages"][0]).unwrap();
        assert_eq!(restored.message, original.message);
        assert_eq!(restored.snapshot, original.snapshot);
        assert_eq!(restored.block_hash, original.block_hash);
        let mut corrupted = saved["messages"][0].clone();
        corrupted["payload"] = json!("0x00");
        assert!(restore_message(&corrupted).is_err());
    }
}
