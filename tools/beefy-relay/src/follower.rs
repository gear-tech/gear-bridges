use crate::{
    ethereum::{receipt_value, DestinationCheckpoint, Ethereum},
    evidence::freshness_evidence,
    keccak256,
    source::{self, AuthoritySet, CapturedCommitment, Source, SourceProof},
    Hash32,
};
use alloy::{
    consensus::{Transaction, TxEnvelope},
    eips::Decodable2718,
    primitives::{Address, B256},
    providers::Provider,
    rpc::types::Filter,
};
use anyhow::{ensure, Context, Result};
use futures::{stream, StreamExt};
use gear_rpc_client::GearApi;
use serde_json::{json, Value};
use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    time::Duration,
};
use subxt::utils::H256;

const ROOT_PUBLICATION_RETRY_INTERVAL: Duration = Duration::from_secs(3);

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

/// The only transaction accessor for a retained destination commitment.
pub fn commitment_transaction(entry: &Value) -> Result<&Value> {
    let (key, other) = match entry["origin"].as_str() {
        Some("own") => ("submission", "acceptedTransaction"),
        Some("external") => ("acceptedTransaction", "submission"),
        _ => anyhow::bail!("missing or invalid commitment origin; HOLD"),
    };
    let transaction = entry
        .get(key)
        .filter(|value| value.is_object())
        .context("commitment signed transaction missing; HOLD")?;
    ensure!(
        entry[other].is_null(),
        "mixed own/external commitment identities; HOLD"
    );
    ensure!(
        transaction["txHash"] == entry["txHash"]
            && transaction["block"] == entry["block"]
            && transaction["blockHash"] == entry["blockHash"],
        "commitment transaction identity conflicts with history; HOLD"
    );
    ensure!(
        transaction["nonce"].as_u64().is_some() && transaction["rawTransaction"].as_str().is_some(),
        "commitment signature identity missing; HOLD"
    );
    let raw = decode(&transaction["rawTransaction"])?;
    let hash = B256::from(array::<32>(&transaction["txHash"])?);
    let client = transaction["clientAddress"]
        .as_str()
        .or_else(|| entry["clientAddress"].as_str())
        .context("transaction client missing; HOLD")?
        .parse()?;
    let (signed, identity) =
        crate::ethereum::signed_transaction(&raw, hash, number(&transaction["nonce"])?, client)?;
    if entry["origin"] == "external" || !transaction["sender"].is_null() {
        ensure!(
            transaction["sender"] == format!("{:#x}", identity.from),
            "transaction sender conflicts with signed bytes; HOLD"
        );
    }
    if entry["origin"] == "external" || !transaction["chainId"].is_null() {
        ensure!(
            signed.chain_id() == Some(number(&transaction["chainId"])?),
            "transaction chain conflicts with signed bytes; HOLD"
        );
    }
    if let Some(recorded) = entry["clientAddress"].as_str() {
        ensure!(
            recorded.parse::<Address>()? == client,
            "commitment clients conflict; HOLD"
        );
    }
    Ok(transaction)
}

fn journal_chain_id(state: &Value) -> Result<u64> {
    number(if state["mode"] == "hoodi-message-demo" {
        &state["ethereum"]["chainId"]
    } else {
        &state["deployment"]["ethereum"]["chainId"]
    })
}

/// Validate legacy evidence before assigning ownership provenance; never relabel bytes.
pub(crate) fn migrate_commitment_journal(state: &mut Value) -> Result<()> {
    ensure!(
        matches!(state["schemaVersion"].as_u64(), Some(3 | 4)),
        "unsupported follower schema; HOLD"
    );
    let legacy = state["schemaVersion"] == 3;
    let client = deployment_client(state)?;
    let chain = journal_chain_id(state)?;
    let signer = state["followerSigner"]
        .as_str()
        .map(str::parse::<Address>)
        .transpose()?;
    for entry in state["commitments"]
        .as_array()
        .context("missing commitments")?
    {
        ensure!(
            journal_client(entry, client)? == client,
            "commitment belongs to another deployment client; HOLD"
        );
        let transaction = if legacy {
            ensure!(
                entry["origin"].is_null() && entry["acceptedTransaction"].is_null(),
                "legacy provenance conflicts; HOLD"
            );
            &entry["submission"]
        } else {
            commitment_transaction(entry)?
        };
        require_submission_client(transaction, client, client)?;
        ensure!(
            transaction["txHash"] == entry["txHash"]
                && transaction["block"] == entry["block"]
                && transaction["blockHash"] == entry["blockHash"],
            "commitment identity conflicts; HOLD"
        );
        let raw = decode(&transaction["rawTransaction"])?;
        let (signed, identity) = crate::ethereum::signed_transaction(
            &raw,
            B256::from(array::<32>(&entry["txHash"])?),
            number(&transaction["nonce"])?,
            client,
        )?;
        ensure!(
            signed.chain_id() == Some(chain),
            "commitment transaction chain differs; HOLD"
        );
        if !transaction["sender"].is_null() {
            ensure!(
                transaction["sender"] == format!("{:#x}", identity.from),
                "commitment sender differs; HOLD"
            );
        }
        if !transaction["chainId"].is_null() {
            ensure!(
                transaction["chainId"] == chain,
                "commitment chain metadata differs; HOLD"
            );
        }
        crate::ethereum::validate_commitment_call(&signed, &decode(&entry["rawScale"])?)?;
        if legacy || entry["origin"] == "own" {
            ensure!(
                signer.is_none_or(|signer| signer == identity.from),
                "own commitment signer changed; HOLD"
            );
        } else {
            ensure!(
                entry["finalized"] == true,
                "external commitment lacks authenticated finality; HOLD"
            );
        }
    }
    ensure!(
        state["terminalSubmissions"].is_array()
            || (legacy && state["terminalSubmissions"].is_null()),
        "terminal submission history malformed; HOLD"
    );
    if legacy {
        for entry in state["commitments"]
            .as_array_mut()
            .expect("validated commitments")
        {
            if entry["clientAddress"].is_null() {
                entry["clientAddress"] = json!(format!("{client:#x}"));
            }
            entry["origin"] = json!("own");
        }
        if state["terminalSubmissions"].is_null() {
            state["terminalSubmissions"] = json!([]);
        }
        state["schemaVersion"] = json!(4);
    }
    Ok(())
}

pub(crate) fn validate_token_follower_journal(state: &Value, deployment: &Value) -> Result<()> {
    ensure!(
        matches!(state["schemaVersion"].as_u64(), Some(3 | 4))
            && ((deployment["mode"] == "mainnet-token-stack"
                && state["mode"] == "mainnet-token-follow")
                || (deployment["mode"] == "hoodi-token-stack"
                    && state["mode"] == "hoodi-token-follow")),
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

#[derive(Debug)]
struct FollowerPersistenceFailed;
impl std::fmt::Display for FollowerPersistenceFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("follower journal persistence failed; HOLD without retry")
    }
}
impl std::error::Error for FollowerPersistenceFailed {}

fn save(output: &Path, state: &mut Value, ethereum: Option<&Ethereum>) -> Result<()> {
    save_state(output, state, ethereum).context(FollowerPersistenceFailed)
}

fn save_state(output: &Path, state: &mut Value, ethereum: Option<&Ethereum>) -> Result<()> {
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
pub(crate) async fn authenticated_token_anchor(
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
    use ethereum_client::signed_transaction::{
        decode_signed_transaction, SignedTransactionErrors, SignedTransactionExpected,
    };
    let (signed, identity) = decode_signed_transaction(
        &raw,
        SignedTransactionExpected {
            hash: tx_hash.into(),
            nonce: number(&submission["nonce"])?,
            destination: expected_client,
            chain_id: None,
            signer: None,
        },
        SignedTransactionErrors {
            hash: "saved commitment bytes do not match their hash",
            decode: "invalid saved commitment transaction",
            trailing: None,
            identity: "saved commitment nonce or destination differs from signed bytes",
            signature: "invalid saved transaction signature",
            signer: "saved sender conflicts with signed bytes; HOLD",
        },
    )?;
    if !submission["sender"].is_null() {
        ensure!(
            submission["sender"] == format!("{:#x}", identity.from),
            "saved sender conflicts with signed bytes; HOLD"
        );
    }
    if !submission["chainId"].is_null() {
        ensure!(
            signed.chain_id() == Some(number(&submission["chainId"])?),
            "saved chain conflicts with signed bytes; HOLD"
        );
    }
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
                    && receipt.receipt.block_number == Some(receipt.included_block_number)
                    && receipt.receipt.block_hash == Some(receipt.included_block_hash),
                "finalized BEEFY receipt identity is invalid"
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
            "origin": "own",
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

fn require_authority_successor(previous: &Value, current: &AuthoritySet) -> Result<()> {
    let current = serde_json::to_value(current)?;
    ensure!(
        previous["current"] == current || previous["next"] == current,
        "destination skipped or changed an authenticated authority handover; HOLD"
    );
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
    revalidate_terminal_submissions(source, witness, ethereum, state, proof_directory).await?;
    if !state["submission"].is_null() {
        let submission = &state["submission"];
        require_submission_client(
            submission,
            ethereum.client_address,
            deployment_client(state)?,
        )?;
        let block: u32 = number(&submission["block"])?.try_into()?;
        let anchor = source
            .recapture(block, decode(&submission["rawScale"])?)
            .await?;
        let witness_finalized = witness
            .api
            .block_hash_to_number(witness.api.latest_finalized_block().await?)
            .await?;
        ensure!(
            witness_finalized >= block
                && witness.api.block_number_to_hash(block).await?.0 == anchor.block_hash
                && submission["blockHash"] == bytes(anchor.block_hash)
                && submission["root"] == bytes(anchor.validated.mmr_root),
            "active original source identity changed; HOLD"
        );
        if !submission["txHash"].is_null() {
            require_saved_submission(
                submission,
                &anchor,
                array(&submission["txHash"])?,
                ethereum.client_address,
                deployment_client(state)?,
            )?;
            let raw = decode(&submission["rawTransaction"])?;
            let signed = crate::ethereum::recorded_commitment_transaction(&raw, &anchor.raw)?;
            ensure!(
                signed.chain_id() == Some(journal_chain_id(state)?),
                "active original chain changed; HOLD"
            );
        } else {
            ensure!(
                submission["rawTransaction"].is_null() && submission["nonce"].as_u64().is_some(),
                "unsigned reservation has a mixed identity; HOLD"
            );
        }
    }
    tokio::time::timeout(Duration::from_secs(300), async {
        let legacy = deployment_client(state)?;
        let mut targets = BTreeMap::new();
        let mut checkpoints = Vec::new();
        let mut source_targets = BTreeMap::new();
        let mut source_records = BTreeMap::<u64, &Value>::new();
        let mut original_inclusions = BTreeMap::new();
        let mut inclusions = BTreeMap::<(u64, B256), Vec<crate::ethereum::OriginalCommitmentReceipt>>::new();
        let bind = |targets: &mut BTreeMap<u64, B256>, number, hash| -> Result<()> {
            ensure!(targets.insert(number, hash).is_none_or(|old| old == hash), "original history contains conflicting block identities; HOLD");
            Ok(())
        };
        let mut previous = client_bootstrap(state,ethereum.client_address)?;
        for entry in state["commitments"].as_array().context("missing commitment journal")? {
            let client = journal_client(entry, legacy)?;
            ensure!(client == legacy && entry["finalized"].is_boolean(), "retained commitment client or finality marker changed; HOLD");
            ensure!(number(&entry["block"])? > number(&previous["block"])?, "retained destination commitments are not in source order; HOLD");
            let current: AuthoritySet = serde_json::from_value(entry["current"].clone())?;
            require_authority_successor(previous,&current)?;
            previous = entry;
            let recorded_transaction = commitment_transaction(entry)?;
            require_submission_client(recorded_transaction, client, legacy)?;
            let hash = B256::from(array::<32>(&entry["txHash"])?);
            let raw = decode(&recorded_transaction["rawTransaction"])?;
            let (transaction, identity) = crate::ethereum::signed_transaction(&raw, hash, number(&recorded_transaction["nonce"])?, client)?;
            if entry["origin"] == "own" && !state["followerSigner"].is_null() {ensure!(state["followerSigner"] == format!("{:#x}",identity.from), "own finalized transaction changed follower signer; HOLD");}
            ensure!(transaction.chain_id() == Some(journal_chain_id(state)?)
                && recorded_transaction["txHash"] == entry["txHash"] && identity.to == Some(client),
                "original finalized signed transaction changed chain, hash or client binding; HOLD");
            let included = (number(&entry["destinationBlock"])? , B256::from(array::<32>(&entry["destinationHash"])?));
            if entry["finalized"] == true {
                ensure!(number(&entry["finalizedBlock"])? >= included.0, "saved finality predates original inclusion; HOLD");
                bind(&mut targets,included.0,included.1)?;
                bind(&mut targets,number(&entry["finalizedBlock"])?,B256::from(array::<32>(&entry["finalizedHash"])?))?;
                if let Some(checkpoint) = entry.get("checkpoint") {
                    checkpoints.push(serde_json::from_value(checkpoint.clone())?);
                }
            }
            let source_block: u32 = number(&entry["block"] )?.try_into()?;
            bind(&mut source_targets, u64::from(source_block), B256::from(array::<32>(&entry["blockHash"])?))?;
            if let Some(previous) = source_records.insert(u64::from(source_block), entry) {
                ensure!(previous["rawScale"] == entry["rawScale"] && previous["current"] == entry["current"] && previous["next"] == entry["next"],
                    "duplicate original source proof identity changed; HOLD");
            }
            crate::ethereum::validate_commitment_call(&transaction, &decode(&entry["rawScale"] )?)?;
            if entry["finalized"] != true {continue;}
            let original = crate::ethereum::OriginalCommitmentReceipt { hash, raw, source_block,
                mmr_root: array::<32>(&recorded_transaction["root"] )?, client };
            if let Some(previous) = original_inclusions.insert(hash, included) {
                ensure!(previous == included, "original transaction changed canonical inclusion; HOLD");
            } else { inclusions.entry(included).or_default().push(original); }
        }
        let mut view = ethereum.api.verified_finalized_view_with_archive(&proof_directory.join("headers"),
            ethereum.finality_genesis().await?).await?;
        view.verify_checkpoints_with_blocks(ethereum.api.raw_provider(), &checkpoints, &targets).await?;
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
            require_saved_submission(commitment_transaction(entry)?, &captured, array(&entry["txHash"] )?, journal_client(entry, legacy)?, legacy)?;
            Ok::<_, anyhow::Error>(())
        })).buffer_unordered(32);
        while let Some(result) = source_checks.next().await { result?; }
        let receipts_directory = proof_directory.join("receipts");
        for entry in state["commitments"].as_array().context("missing commitment journal")?
            .iter().filter(|entry| entry["finalized"] == true) {
            if let Some(retained) = entry.get("receiptProof") {
                let hash = B256::from(array::<32>(&entry["txHash"])?);
                let included = (number(&entry["destinationBlock"])?,
                    B256::from(array::<32>(&entry["destinationHash"])?));
                let proof = crate::ethereum::recover_receipt_proof(ethereum.api.raw_provider(),
                    &mut view, included, hash, Some(retained), None, &receipts_directory).await?;
                proof.verify_original_inclusion(&view,
                    &decode(&commitment_transaction(entry)?["rawTransaction"])?, hash)?;
                fs::create_dir_all(&receipts_directory)?;
                gear_common::durable::immutable_json(&receipts_directory.join(format!("{hash:x}.json")), &proof)?;
            }
        }
        let mut receipts = stream::iter(inclusions.iter().map(|(&(block, hash), originals)|
            crate::ethereum::authenticate_recorded_receipts(ethereum, &view, block, hash, originals, &receipts_directory)))
            .buffer_unordered(32);
        while let Some(result) = receipts.next().await { result?; }
        view.recheck(ethereum.api.raw_provider()).await?;
        for entry in state["commitments"].as_array().context("missing commitment journal")?
            .iter().filter(|entry| entry["finalized"] == true) {
            crate::ethereum::retain_proof_qualification(proof_directory, entry,
                &json!({"transactionHash":entry["txHash"],"checkpoint":view.checkpoint()}))?;
        }
        Ok(())
    }).await.context("saved finalized history did not revalidate within 300 seconds; HOLD; retain authenticated partial proof progress")?
}

/// Operational audit: never signs, broadcasts, changes actor status or rewrites
/// original journals. Only locally revalidated proof material is populated.
pub async fn audit_history(
    source_rpcs: (&str, &str),
    ethereum_rpcs: (&str, &cli_utils::EthereumFinalityArgs),
    deployment_manifest: &Path,
    state_path: &Path,
    proof_directory: &Path,
    owned_fixture_network: Option<&Path>,
    public_network: crate::ethereum::PublicEthereumNetwork,
) -> Result<()> {
    let (source_rpc, witness_rpc) = source_rpcs;
    let (ethereum_rpc, finality) = ethereum_rpcs;
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    let fixture = crate::ethereum::approved_fixture_network(
        owned_fixture_network,
        &deployment,
        source_rpc,
        witness_rpc,
        ethereum_rpc,
        public_network,
    )?;
    let mut state = read_state_file(state_path)?;
    migrate_commitment_journal(&mut state)?;
    validate_token_follower_journal(&state, &deployment)?;
    ensure!(
        source_rpc != witness_rpc,
        "history audit requires separate source/witness endpoints"
    );
    let ((mut source, mut witness), ethereum) = tokio::try_join!(
        async {
            Source::connect_pair(
                GearApi::new(source_rpc, 3).await?,
                GearApi::new(witness_rpc, 3).await?,
            )
            .await
        },
        Ethereum::connect_readonly(
            ethereum_rpc,
            finality,
            &state["activeEthereum"],
            fixture.as_ref(),
            public_network
        ),
    )?;
    source.validate_attachment(&deployment["anchor"]).await?;
    let started = std::time::Instant::now();
    let (bootstrap, proof) =
        authenticated_token_anchor(&mut source, &mut witness, &deployment["anchor"]).await?;
    ensure!(
        state["bootstrap"] == bootstrap_record(&bootstrap, &proof)?,
        "saved source bootstrap changed; HOLD"
    );
    let (checkpoint, _) = ethereum
        .authenticate_bootstrap(proof_directory, &state["bootstrapDeployment"], None)
        .await?;
    ensure!(
        checkpoint.block == u64::from(bootstrap.block)
            && checkpoint_matches_source(
                &checkpoint,
                [0; 32],
                proof.snapshot.source_timestamp_ms,
                &bootstrap.current,
                &bootstrap.next
            ),
        "immutable deployment bootstrap changed; HOLD"
    );
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

pub(crate) fn validate_root_publication_inventory(
    publications: &Path,
    registered_paths: &std::collections::BTreeSet<&Path>,
) -> Result<()> {
    if publications.try_exists()? {
        for entry in fs::read_dir(publications)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() && entry.file_name() == "finality-history" {
                continue;
            }
            ensure!(
                file_type.is_file()
                    && path.extension().and_then(|extension| extension.to_str()) == Some("json")
                    && registered_paths.contains(path.as_path()),
                "unindexed or incomplete root publication evidence; HOLD"
            );
        }
    }
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
                        Some("pending" | "mined" | "accepted" | "observed")
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
                ensure!(
                    intent["rawTransaction"].is_null() == intent["txHash"].is_null(),
                    "root has a mixed signed identity; HOLD"
                );
                if intent["rawTransaction"].is_null() {
                    continue;
                }
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
        validate_root_publication_inventory(&publications, &registered_paths)?;
        let raw = decode(&submission["rawTransaction"])?;
        let (signed, _) = crate::ethereum::signed_transaction(
            &raw,
            hash,
            number(&submission["nonce"])?,
            ethereum.client_address,
        )?;
        ensure!(
            signed.chain_id() == Some(number(&state["deployment"]["ethereum"]["chainId"])?),
            "reincluded transaction changed chain; HOLD"
        );
        crate::ethereum::validate_commitment_call(&signed, &anchor.raw)?;
        let original = crate::ethereum::OriginalCommitmentReceipt {
            hash,
            raw,
            source_block: anchor.block,
            mmr_root: anchor.validated.mmr_root,
            client: ethereum.client_address,
        };
        let directory = output.join("finality-history");
        let view = finalized_receipt_view(ethereum, &directory, finalized).await?;
        crate::ethereum::retain_finalized_receipt(
            finalized,
            &view,
            &original,
            &directory.join("receipts"),
        )
    })
    .await
    .context(
        "reinclusion authentication did not finish within 300 seconds; HOLD original evidence",
    )?
}

async fn finalized_receipt_view(
    ethereum: &Ethereum,
    directory: &Path,
    finalized: &ethereum_client::FinalizedTransactionReceipt,
) -> Result<ethereum_client::FinalizedView> {
    let mut view = ethereum
        .api
        .verified_finalized_view_with_archive(
            &directory.join("headers"),
            ethereum.finality_genesis().await?,
        )
        .await?;
    view.verify_blocks(
        ethereum.api.raw_provider(),
        &std::collections::BTreeMap::from([
            (
                finalized.included_block_number,
                finalized.included_block_hash,
            ),
            (
                finalized.finalized_block_number,
                finalized.finalized_block_hash,
            ),
        ]),
    )
    .await?;
    Ok(view)
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
    let chain_id = journal_chain_id(state)?;
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
                        commitment_transaction(entry)?,
                        &anchor,
                        hash,
                        ethereum.client_address,
                        legacy_client,
                    )?;
                    let saved_transaction = commitment_transaction(entry)?;
                    let raw = decode(&saved_transaction["rawTransaction"])?;
                    let (signed, _) = crate::ethereum::signed_transaction(
                        &raw,
                        hash.into(),
                        number(&saved_transaction["nonce"])?,
                        ethereum.client_address,
                    )?;
                    ensure!(
                        signed.chain_id() == Some(chain_id),
                        "pending commitment chain changed; HOLD"
                    );
                    let receipt = match ethereum
                        .api
                        .raw_provider()
                        .get_transaction_receipt(hash.into())
                        .await?
                    {
                        Some(receipt) => receipt,
                        None => {
                            let mut view = ethereum
                                .api
                                .verified_finalized_view_with_archive(
                                    &output.join("finality-history/headers"),
                                    ethereum.finality_genesis().await?,
                                )
                                .await?;
                            let (number, pin_hash) = (view.block_number(), view.block_hash());
                            let pin = ethereum
                                .account_nonce_at(
                                    &raw,
                                    &mut view,
                                    number,
                                    pin_hash,
                                    None,
                                    &output.join("finality-history"),
                                )
                                .await?;
                            if pin["pinnedNonceConsumed"] == true {
                                return Ok(Some((index, block, hash, anchor, None, false, pin)));
                            }
                            anyhow::bail!(
                                "previously mined commitment disappeared; hold original inclusion"
                            );
                        }
                    };
                    let included = receipt
                        .block_number
                        .context("pending receipt has no block number")?;
                    let included_hash = receipt
                        .block_hash
                        .context("pending receipt has no block hash")?;
                    ensure!(
                        receipt.transaction_hash == B256::from(hash)
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
                            && finalized.included_block_number == included
                            && finalized.included_block_hash == included_hash
                            && finalized.receipt.block_number
                                == Some(finalized.included_block_number)
                            && finalized.receipt.block_hash == Some(finalized.included_block_hash),
                        "finalized BEEFY receipt differs from saved commitment"
                    );
                    Ok(Some((
                        index,
                        block,
                        hash,
                        anchor,
                        Some(finalized),
                        reincluded,
                        Value::Null,
                    )))
                }),
        )
        .buffered(16)
        .collect::<Vec<Result<_>>>()
        .await
    };
    if pending.iter().all(|result| matches!(result, Ok(None))) {
        return Ok(());
    }
    // Stage only mutable journal sections: failed originals and removed ownership commit together.
    let mut staged = json!({"commitments":state["commitments"],"follower":state["follower"],"transactions":state["transactions"],"terminalSubmissions":state["terminalSubmissions"]});
    let previous_transactions = ethereum.transactions.clone();
    let result = async {
    let mut last_finalized = None;
    let mut failed = Vec::new();
    for result in pending {
        let Some((index, block, hash, anchor, finalized, reincluded, nonce_disposition)) = result? else {
            continue;
        };
        let entry = &staged["commitments"][index];
        let finalized = match finalized {
            Some(finalized) if finalized.receipt.status() => finalized,
            finalized => {
                let terminal = if let Some(finalized) = finalized {
                    ensure!(entry["origin"] == "own", "external accepted checkpoint reverted; HOLD");
                    let submission = commitment_transaction(entry)?.clone();
                    let raw = decode(&submission["rawTransaction"])?;
                    let (_, identity) = crate::ethereum::signed_transaction(&raw,hash.into(),number(&submission["nonce"])?,ethereum.client_address)?;
                    ensure!(finalized.receipt.from == identity.from && finalized.receipt.to == identity.to, "failed reincluded original identity changed; HOLD");
                    let directory = output.join("finality-history");
                    finalized_receipt_view(ethereum, &directory, &finalized).await?;
                    let receipt = &finalized.inclusion;
                    ensure!(receipt.raw_transaction == raw && !receipt.receipt.status(), "failed reincluded original proof changed; HOLD");
                    json!({"submission":submission,"outcome":"finalized-revert","minedObservation":entry,
                        "failureEvidence":{"receipt":finalized.receipt,"includedBlock":finalized.included_block_number,"includedBlockHash":bytes(finalized.included_block_hash),"finalizedBlock":finalized.finalized_block_number,"finalizedBlockHash":bytes(finalized.finalized_block_hash),"inclusion":finalized.inclusion,"checkpoint":finalized.checkpoint}})
                } else {
                    ensure!(entry["origin"] == "own", "external accepted checkpoint lost canonical receipt; HOLD");
                    json!({"submission":commitment_transaction(entry)?,"outcome":"held-consumed-nonce","nonceDisposition":nonce_disposition,"minedObservation":entry})
                };
                archive_terminal_submission(&mut staged,terminal)?;
                if let Some(accepted) = accepted_tx(state,ethereum,&anchor).await? {
                    ensure!(accepted != hash, "failed original conflicts with accepted commitment identity; HOLD");
                    let proof = source.proof(block-1,&anchor).await?;
                    let external = authenticate_external_commitment(ethereum,output,state,&anchor,&proof,accepted).await?;
                    staged["commitments"][index] = external;
                    let previous = staged["follower"]["lastObservedFinalizedUpdate"].as_u64().unwrap_or(0);
                    staged["follower"]["lastObservedFinalizedUpdate"] = json!(u64::from(block).max(previous));
                    last_finalized = Some(json!(block));
                } else {failed.push(index);}
                continue;
            }
        };
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
                commitment_transaction(entry)?,
                &anchor,
                &finalized,
            )
            .await?;
        }
        let raw = decode(&commitment_transaction(entry)?["rawTransaction"])?;
        crate::ethereum::recorded_commitment_transaction(&raw,&anchor.raw)?;
        let directory = output.join("finality-history");
        let view = finalized_receipt_view(ethereum, &directory, &finalized).await?;
        let original = crate::ethereum::OriginalCommitmentReceipt {hash:hash.into(),raw,source_block:anchor.block,mmr_root:anchor.validated.mmr_root,client:ethereum.client_address};
        crate::ethereum::retain_finalized_receipt(&finalized, &view, &original, &directory.join("receipts"))?;
        let first_inclusion = record_recovered_receipt(&mut staged, ethereum, &finalized)?;
        let destination_hash: [u8; 32] = finalized.included_block_hash.into();
        let finalized_hash: [u8; 32] = finalized.finalized_block_hash.into();
        let entry = &mut staged["commitments"][index];
        if !first_inclusion.is_null() {
            entry["firstInclusion"] = first_inclusion;
        }
        entry["destinationBlock"] = json!(finalized.included_block_number);
        entry["destinationHash"] = json!(bytes(destination_hash));
        entry["finalized"] = json!(true);
        entry["finalizedBlock"] = json!(finalized.finalized_block_number);
        entry["finalizedHash"] = json!(bytes(finalized_hash));
        let own = entry["origin"] == "own";
        let block = entry["block"].clone();
        if own {staged["follower"]["lastSuccessfulUpdate"] = block.clone();}
        else {staged["follower"]["lastObservedFinalizedUpdate"] = block.clone();}
        last_finalized = Some(block);
    }
    let changed = last_finalized.is_some() || !failed.is_empty();
    if let Some(block) = last_finalized {
        let previous = staged["follower"]["lastFinalizedUpdate"].as_u64().unwrap_or(0);
        staged["follower"]["lastFinalizedUpdate"] = json!(number(&block)?.max(previous));
    }
    if !failed.is_empty() {
        for index in failed.into_iter().rev() {staged["commitments"].as_array_mut().context("missing commitments")?.remove(index);}
    }
    Ok::<_,anyhow::Error>(changed)
    }.await;
    let changed = match result {
        Ok(changed) => changed,
        Err(error) => {
            ethereum.transactions = previous_transactions;
            return Err(error);
        }
    };
    if changed {
        for key in [
            "commitments",
            "follower",
            "transactions",
            "terminalSubmissions",
        ] {
            state[key] = staged[key].take();
        }
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
) -> Result<Option<[u8; 32]>> {
    let legacy = deployment_client(state)?;
    let mut from = state["bootstrapDeployment"]["block"].as_u64().unwrap_or(0);
    for entry in state["commitments"]
        .as_array()
        .context("missing commitments")?
    {
        if journal_client(entry, legacy)? == ethereum.client_address
            && number(&entry["block"])? < u64::from(anchor.block)
        {
            from = from.max(number(&entry["destinationBlock"])?);
        }
    }
    let provider = ethereum.api.raw_provider();
    let to = provider.get_block_number().await?;
    accepted_tx_in_range(
        from,
        to,
        anchor.validated.mmr_root,
        anchor.block,
        |start, end| async move {
            provider
                .get_logs(
                    &Filter::new()
                        .address(ethereum.client_address)
                        .event_signature(B256::from(keccak256(b"NewMMRRoot(bytes32,uint64)")))
                        .from_block(start)
                        .to_block(end),
                )
                .await
                .with_context(|| format!("eth_getLogs accepted commitment {} at client {:#x}, Ethereum range {start}..={end} (snapshot {to})", anchor.block, ethereum.client_address))
        },
    )
    .await
}

async fn accepted_tx_in_range<F, Fut>(
    mut from: u64,
    to: u64,
    root: Hash32,
    source_block: u32,
    mut fetch: F,
) -> Result<Option<[u8; 32]>>
where
    F: FnMut(u64, u64) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<alloy::rpc::types::Log>>>,
{
    let mut accepted = None;
    while from <= to {
        let end = from.saturating_add(1_999).min(to);
        for log in fetch(from, end).await? {
            let data = log.data().data.as_ref();
            if data.len() == 64
                && data[..32] == root
                && data[32..56].iter().all(|byte| *byte == 0)
                && data[56..64] == u64::from(source_block).to_be_bytes()
            {
                ensure!(
                    !log.removed
                        && log.block_hash.is_some()
                        && log.transaction_index.is_some()
                        && log.log_index.is_some(),
                    "accepted event coordinates missing; HOLD"
                );
                ensure!(
                    log.block_number
                        .is_some_and(|number| number >= from && number <= end),
                    "accepted event is outside requested range; HOLD"
                );
                let hash = log
                    .transaction_hash
                    .context("accepted event transaction missing")?;
                ensure!(
                    accepted.replace(hash).is_none(),
                    "multiple accepted commitment identities; HOLD"
                );
            }
        }
        // Scan the complete fixed snapshot: a later batch may contain a conflict.
        if end == to {
            break;
        }
        from = end + 1;
    }
    Ok(accepted.map(Into::into))
}

async fn authenticate_external_commitment(
    ethereum: &Ethereum,
    output: &Path,
    state: &Value,
    anchor: &CapturedCommitment,
    proof: &SourceProof,
    hash: [u8; 32],
) -> Result<Value> {
    let finalized = wait_for_finalized_receipt(ethereum, hash).await?;
    let directory = output.join("finality-history");
    let view = finalized_receipt_view(ethereum, &directory, &finalized).await?;
    let receipt = &finalized.inclusion;
    let mut encoded = receipt.raw_transaction.as_slice();
    let signed = TxEnvelope::decode_2718(&mut encoded)?;
    let (_, identity) = crate::ethereum::signed_transaction(
        &receipt.raw_transaction,
        hash.into(),
        signed.nonce(),
        ethereum.client_address,
    )?;
    ensure!(
        signed.chain_id() == Some(journal_chain_id(state)?)
            && finalized.receipt.from == identity.from
            && finalized.receipt.to == identity.to
            && receipt.receipt.status(),
        "external signed receipt domain or status changed; HOLD"
    );
    crate::ethereum::validate_commitment_call(&signed, &anchor.raw)?;
    let original = crate::ethereum::OriginalCommitmentReceipt {
        hash: hash.into(),
        raw: receipt.raw_transaction.clone(),
        source_block: anchor.block,
        mmr_root: anchor.validated.mmr_root,
        client: ethereum.client_address,
    };
    crate::ethereum::retain_finalized_receipt(
        &finalized,
        &view,
        &original,
        &directory.join("receipts"),
    )?;
    let destination = ethereum
        .verify_accepted_commitment(hash.into(), anchor, &proof.snapshot)
        .await?;
    ensure!(
        destination
            == (
                finalized.included_block_number,
                finalized.included_block_hash
            ),
        "external accepted checkpoint inclusion changed; HOLD"
    );
    let transaction = json!({"clientAddress":format!("{:#x}",ethereum.client_address),"block":anchor.block,"blockHash":bytes(anchor.block_hash),"root":bytes(anchor.validated.mmr_root),"txHash":bytes(hash),"nonce":identity.nonce,"rawTransaction":bytes(&receipt.raw_transaction),"sender":format!("{:#x}",identity.from),"chainId":signed.chain_id()});
    let entry = json!({"origin":"external","acceptedTransaction":transaction,"clientAddress":format!("{:#x}",ethereum.client_address),
        "block":anchor.block,"blockHash":bytes(anchor.block_hash),"rawScale":bytes(&anchor.raw),"current":anchor.current,"next":anchor.next,"freshnessProof":freshness_evidence(proof)?,
        "txHash":bytes(hash),"destinationBlock":destination.0,"destinationHash":bytes(destination.1),"finalized":true,
        "finalizedBlock":finalized.finalized_block_number,"finalizedHash":bytes(finalized.finalized_block_hash)});
    commitment_transaction(&entry)?;
    Ok(entry)
}

async fn record_external_commitment(
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
    anchor: &CapturedCommitment,
    proof: &SourceProof,
    hash: [u8; 32],
) -> Result<()> {
    if ethereum
        .api
        .get_finalized_receipt(hash.into())
        .await?
        .is_none()
    {
        state["follower"]["status"] = json!("held-finality");
        state["follower"]["lastError"] =
            json!("external checkpoint awaits authenticated destination finality");
        save(output, state, Some(ethereum))?;
    }
    let entry =
        authenticate_external_commitment(ethereum, output, state, anchor, proof, hash).await?;
    state["commitments"]
        .as_array_mut()
        .context("missing commitments")?
        .push(entry);
    state["follower"]["lastFinalizedUpdate"] = json!(anchor.block);
    state["follower"]["lastObservedFinalizedUpdate"] = json!(anchor.block);
    state["follower"]["status"] = json!("healthy");
    state["follower"]["freshnessDeadlineMs"] =
        json!(proof.snapshot.source_timestamp_ms + 86_400_000);
    if state["submission"]["rawTransaction"].is_null() {
        state["submission"] = Value::Null;
    }
    save(output, state, Some(ethereum))
}

pub fn checkpoint_matches_source(
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

fn same_set_commitment_due(
    handover_only: bool,
    checkpoint_block: u64,
    candidate_block: u32,
    target: u32,
    root_target: Option<u32>,
) -> bool {
    (!handover_only || root_target.is_some_and(|block| u64::from(block) >= checkpoint_block))
        && candidate_block >= target.saturating_sub(2)
        && root_target.is_none_or(|block| candidate_block > block)
}

fn client_commitment_due(
    handover_only: bool,
    checkpoint: (u64, u64),
    candidate: (u32, u64),
    target: u32,
    root_target: Option<u32>,
) -> bool {
    candidate.1 == checkpoint.1
        || same_set_commitment_due(
            handover_only,
            checkpoint.0,
            candidate.0,
            target,
            root_target,
        )
}

fn covered_root_ready_for_publication(
    state: &Value,
    checkpoint: u64,
    pending: Option<u32>,
    maintenance_elapsed: Duration,
) -> bool {
    pending.is_some_and(|block| u64::from(block) < checkpoint)
        && (state["rootPublication"]["status"] != "held"
            || maintenance_elapsed >= ROOT_PUBLICATION_RETRY_INTERVAL)
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
        crate::root_publication::validate_registered_root_publications(ethereum, &state["roots"])
            .await?;
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
    loop {
        promote_finality(source, ethereum, output, state).await?;
        if state["commitments"]
            .as_array()
            .context("missing commitments")?
            .iter()
            .all(|entry| entry["finalized"] == true)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

fn archive_terminal_submission(state: &mut Value, terminal: Value) -> Result<()> {
    if state["terminalSubmissions"].is_null() {
        state["terminalSubmissions"] = json!([]);
    }
    let hash = terminal["submission"]["txHash"].clone();
    let terminals = state["terminalSubmissions"]
        .as_array_mut()
        .context("invalid terminal submission history")?;
    ensure!(
        !terminals
            .iter()
            .any(|entry| entry["submission"]["txHash"] == hash),
        "duplicate original terminal disposition; HOLD"
    );
    terminals.push(terminal);
    let failed = terminals
        .iter()
        .filter(|entry| entry["outcome"] == "finalized-revert")
        .count();
    let held = terminals
        .iter()
        .filter(|entry| entry["outcome"] == "held-consumed-nonce")
        .count();
    state["follower"]["failedOriginalSubmissions"] = json!(failed);
    state["follower"]["heldOriginalSubmissions"] = json!(held);
    if state["submission"]["txHash"] == hash {
        state["submission"] = Value::Null;
    }
    Ok(())
}

async fn reconcile_original_submission(
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    if state["submission"]["rawTransaction"].is_null() {
        return Ok(());
    }
    let submission = state["submission"].clone();
    let hash = B256::from(array::<32>(&submission["txHash"])?);
    let raw = decode(&submission["rawTransaction"])?;
    let (signed, identity) = crate::ethereum::signed_transaction(
        &raw,
        hash,
        number(&submission["nonce"])?,
        ethereum.client_address,
    )?;
    if !state["followerSigner"].is_null() {
        ensure!(
            state["followerSigner"] == format!("{:#x}", identity.from),
            "active original follower signer changed; HOLD"
        );
    }
    ensure!(
        signed.chain_id() == Some(journal_chain_id(state)?),
        "original submission chain changed; HOLD"
    );
    let directory = output.join("finality-history");
    let mut view = ethereum
        .api
        .verified_finalized_view_with_archive(
            &directory.join("headers"),
            ethereum.finality_genesis().await?,
        )
        .await?;
    let mut terminal = json!({"submission":submission,"outcome":null});
    if let Some(finalized) = ethereum.api.get_finalized_receipt(hash).await? {
        ensure!(
            finalized.receipt.from == identity.from
                && finalized.receipt.to == identity.to
                && finalized.receipt.transaction_hash == hash,
            "original finalized receipt identity changed; HOLD"
        );
        if finalized.receipt.status() {
            return Ok(());
        }
        view.verify_blocks(
            ethereum.api.raw_provider(),
            &std::collections::BTreeMap::from([
                (
                    finalized.included_block_number,
                    finalized.included_block_hash,
                ),
                (
                    finalized.finalized_block_number,
                    finalized.finalized_block_hash,
                ),
            ]),
        )
        .await?;
        let receipt = &finalized.inclusion;
        ensure!(
            receipt.raw_transaction == raw && !receipt.receipt.status(),
            "original failed receipt proof differs; HOLD"
        );
        terminal["outcome"] = json!("finalized-revert");
        terminal["failureEvidence"] = json!({"receipt":finalized.receipt,"includedBlock":finalized.included_block_number,"includedBlockHash":bytes(finalized.included_block_hash),"finalizedBlock":finalized.finalized_block_number,"finalizedBlockHash":bytes(finalized.finalized_block_hash),"inclusion":finalized.inclusion,"checkpoint":finalized.checkpoint});
    } else {
        let (number, hash) = (view.block_number(), view.block_hash());
        let pin = ethereum
            .account_nonce_at(&raw, &mut view, number, hash, None, &directory)
            .await?;
        if pin["pinnedNonceConsumed"] != true {
            return Ok(());
        }
        terminal["outcome"] = json!("held-consumed-nonce");
        terminal["nonceDisposition"] = pin;
    }
    archive_terminal_submission(state, terminal)?;
    save(output, state, Some(ethereum))
}

async fn revalidate_terminal_submissions(
    source: &Source,
    witness: &Source,
    ethereum: &Ethereum,
    state: &Value,
    directory: &Path,
) -> Result<()> {
    let Some(terminals) = state["terminalSubmissions"].as_array() else {
        ensure!(
            state["schemaVersion"] == 3,
            "terminal history missing; HOLD"
        );
        return Ok(());
    };
    let mut view = ethereum
        .api
        .verified_finalized_view_with_archive(
            &directory.join("headers"),
            ethereum.finality_genesis().await?,
        )
        .await?;
    let mut seen = std::collections::BTreeSet::new();
    for entry in terminals {
        let submission = &entry["submission"];
        let hash = B256::from(array::<32>(&submission["txHash"])?);
        ensure!(
            seen.insert(hash) && state["submission"]["txHash"] != submission["txHash"],
            "original terminal ownership duplicated; HOLD"
        );
        let raw = decode(&submission["rawTransaction"])?;
        let (signed, identity) = crate::ethereum::signed_transaction(
            &raw,
            hash,
            number(&submission["nonce"])?,
            ethereum.client_address,
        )?;
        if !state["followerSigner"].is_null() {
            ensure!(
                state["followerSigner"] == format!("{:#x}", identity.from),
                "terminal original follower signer changed; HOLD"
            );
        }
        ensure!(
            signed.chain_id() == Some(journal_chain_id(state)?),
            "terminal submission chain changed; HOLD"
        );
        crate::ethereum::validate_commitment_call(&signed, &decode(&submission["rawScale"])?)?;
        let block: u32 = number(&submission["block"])?.try_into()?;
        let anchor = source
            .recapture(block, decode(&submission["rawScale"])?)
            .await?;
        require_saved_submission(
            submission,
            &anchor,
            hash.into(),
            ethereum.client_address,
            deployment_client(state)?,
        )?;
        ensure!(
            witness.api.block_number_to_hash(block).await?.0 == anchor.block_hash,
            "terminal source commitment changed on witness; HOLD"
        );
        ensure!(
            witness
                .api
                .block_hash_to_number(witness.api.latest_finalized_block().await?)
                .await?
                >= block,
            "terminal source history is not finalized on witness; HOLD"
        );
        match entry["outcome"].as_str() {
            Some("finalized-revert") => {
                let evidence = &entry["failureEvidence"];
                let included = number(&evidence["includedBlock"])?;
                let block_hash = B256::from(array::<32>(&evidence["includedBlockHash"])?);
                ensure!(
                    number(&evidence["finalizedBlock"])? >= included,
                    "terminal finality predates inclusion; HOLD"
                );
                view.verify_blocks(
                    ethereum.api.raw_provider(),
                    &std::collections::BTreeMap::from([
                        (included, block_hash),
                        (
                            number(&evidence["finalizedBlock"])?,
                            B256::from(array::<32>(&evidence["finalizedBlockHash"])?),
                        ),
                    ]),
                )
                .await?;
                let receipt = crate::ethereum::recover_receipt_proof(
                    ethereum.api.raw_provider(),
                    &mut view,
                    (included, block_hash),
                    hash,
                    evidence.get("inclusion"),
                    evidence.get("checkpoint"),
                    &directory.join("receipts"),
                )
                .await?;
                ensure!(
                    receipt.raw_transaction == raw && !receipt.receipt.status(),
                    "terminal failed original proof changed; HOLD"
                );
                let saved: alloy::rpc::types::TransactionReceipt =
                    serde_json::from_value(evidence["receipt"].clone())?;
                ensure!(
                    !saved.status()
                        && saved.transaction_hash == hash
                        && saved.from == identity.from
                        && saved.to == identity.to
                        && saved.block_number == Some(included)
                        && saved.block_hash == Some(block_hash),
                    "terminal failure metadata changed; HOLD"
                );
                ensure!(
                    saved.transaction_index == Some(receipt.index)
                        && saved.into_primitives_receipt().inner == receipt.receipt,
                    "terminal receipt bytes differ from authenticated proof; HOLD"
                );
            }
            Some("held-consumed-nonce") => {
                let pin = &entry["nonceDisposition"];
                let block = number(&pin["block"])?;
                let hash = B256::from(array::<32>(&pin["blockHash"])?);
                let qualified = ethereum
                    .account_nonce_at(&raw, &mut view, block, hash, Some(pin), directory)
                    .await?;
                ensure!(
                    qualified["pinnedNonceConsumed"] == true && pin["pinnedNonceConsumed"] == true,
                    "terminal nonce disposition changed; HOLD"
                );
                // A held original remains unresolved even if an RPC later recovers its receipt.
                if let Some(receipt) = ethereum.api.get_finalized_receipt(identity.hash).await? {
                    ensure!(
                        receipt.receipt.from == identity.from && receipt.receipt.to == identity.to,
                        "recovered held original identity changed; HOLD"
                    );
                    view.verify_blocks(
                        ethereum.api.raw_provider(),
                        &std::collections::BTreeMap::from([
                            (receipt.included_block_number, receipt.included_block_hash),
                            (receipt.finalized_block_number, receipt.finalized_block_hash),
                        ]),
                    )
                    .await?;
                    receipt
                        .inclusion
                        .verify_original_inclusion(&view, &raw, identity.hash)?;
                }
            }
            _ => anyhow::bail!("invalid original terminal disposition; HOLD"),
        }
        view.recheck(ethereum.api.raw_provider()).await?;
        crate::ethereum::retain_proof_qualification(
            directory,
            entry,
            &json!({"transactionHash":bytes(identity.hash),"checkpoint":view.checkpoint()}),
        )?;
    }
    Ok(())
}

async fn advance(
    source: &mut Source,
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
    mut handover_only: bool,
    root_witness: Option<&Source>,
) -> Result<Option<CapturedCommitment>> {
    let maintenance_started = std::time::Instant::now();
    'progress: loop {
        reconcile_original_submission(ethereum, output, state).await?;
        // This pin is saved only after authenticated destination finality.
        if let Some(accepted) = state["follower"]["lastFinalizedUpdate"].as_u64() {
            source.acknowledge_finalized(u32::try_from(accepted)?)?;
        }
        let before = ethereum.checkpoint().await?;
        let reserved_block = if state["submission"].is_null()
            || number(&state["submission"]["block"])? <= before.block
        {
            None
        } else {
            Some(u32::try_from(number(&state["submission"]["block"])?)?)
        };
        let target = source
            .api
            .block_hash_to_number(source.api.latest_finalized_block().await?)
            .await?;
        let mut freshness_check: Option<std::time::Instant> = None;
        // Freeze one discovered demand batch; later traffic must not move its target forever.
        let mut root_target = None;
        let anchor = loop {
            let mut candidate = match source.next_commitment().await {
                Ok(candidate) => candidate,
                Err(error) if error.downcast_ref::<source::CommitmentPending>().is_some() => {
                    state["follower"]["status"] = json!("catching-up");
                    state["follower"]["lastError"] = json!(error.to_string());
                    save(output, state, Some(ethereum))?;
                    if root_witness.is_some()
                        && handover_only
                        && state["submission"].is_null()
                        && covered_root_ready_for_publication(
                            state,
                            before.block,
                            pending_token_root_block(state)?,
                            maintenance_started.elapsed(),
                        )
                    {
                        return Ok(None);
                    }
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
            if handover_only
                && freshness_check.is_none_or(|checked| checked.elapsed().as_secs() >= 60)
            {
                promote_finality(source, ethereum, output, state).await?;
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
            if let Some(witness) = root_witness {
                scan_token_roots(source, witness, output, state).await?;
                let pending = pending_token_root_block(state)?;
                if handover_only
                    && covered_root_ready_for_publication(
                        state,
                        before.block,
                        pending,
                        maintenance_started.elapsed(),
                    )
                    && state["submission"].is_null()
                    && candidate.current.id == before.current_id
                    && u64::from(candidate.block) > before.block
                {
                    // Retry covered roots at a safe source boundary; handovers and heartbeats retain priority.
                    return Ok(None);
                }
                if root_target.is_none() {
                    root_target = pending.filter(|block| u64::from(*block) >= before.block);
                }
            }
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
                let recorded = journaled
                    .or_else(|| (bootstrap["block"] == candidate.block).then_some(bootstrap));
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
                    require_authority_successor(last, &candidate.current)?;
                    let Some(tx_hash) = accepted_tx(state, ethereum, &candidate).await? else {
                        ensure!(
                            u64::from(candidate.block) < before.block,
                            "latest destination checkpoint has no accepted signed receipt; HOLD"
                        );
                        continue;
                    };
                    let proof = source.proof(candidate.block - 1, &candidate).await?;
                    if let Some(witness) = root_witness {
                        let second = witness.proof(candidate.block - 1, &candidate).await?;
                        ensure!(
                            proof.snapshot == second.snapshot
                                && proof.raw_leaf == second.raw_leaf
                                && witness.api.block_number_to_hash(candidate.block).await?.0
                                    == candidate.block_hash,
                            "external commitment differs on witness; HOLD"
                        );
                    }
                    if state["submission"]["txHash"] == bytes(tx_hash) {
                        record_mined_commitment(
                            ethereum, output, state, &candidate, &proof, tx_hash,
                        )
                        .await?;
                    } else {
                        record_external_commitment(
                            ethereum, output, state, &candidate, &proof, tx_hash,
                        )
                        .await?;
                    }
                    reconcile_original_submission(ethereum, output, state).await?;
                    return Ok(Some(candidate));
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
            if client_commitment_due(
                handover_only,
                (before.block, before.next_id),
                (candidate.block, candidate.current.id),
                target,
                root_target,
            ) {
                break candidate;
            }
            if candidate.block >= target.saturating_sub(2)
                && root_target.is_none()
                && state["submission"].is_null()
                && matches!(
                    state["follower"]["status"].as_str(),
                    Some("starting" | "catching-up")
                )
            {
                // Caught up does not require buying an update or extending the accepted expiry.
                state["follower"]["status"] = json!("healthy");
                state["follower"]["lastError"] = Value::Null;
                save(output, state, Some(ethereum))?;
            }
        };
        promote_finality(source, ethereum, output, state).await?;
        if state["roots"].is_object() {
            crate::root_publication::validate_registered_root_publications(
                ethereum,
                &state["roots"],
            )
            .await?;
        }
        if !state["submission"]["rawTransaction"].is_null()
            && state["submission"]["block"] != anchor.block
        {
            state["follower"]["status"] = json!("held-original");
            state["follower"]["lastError"] = json!("external progress authenticated; original unconsumed signature retains signer ownership");
            save(output, state, Some(ethereum))?;
            while !state["submission"].is_null() {
                tokio::time::sleep(Duration::from_secs(3)).await;
                reconcile_original_submission(ethereum, output, state).await?;
                if ethereum.checkpoint().await?.block >= u64::from(anchor.block) {
                    if let Some(accepted) = accepted_tx(state, ethereum, &anchor).await? {
                        let previous = state["commitments"]
                            .as_array()
                            .context("commitments")?
                            .last()
                            .unwrap_or(client_bootstrap(state, ethereum.client_address)?);
                        require_authority_successor(previous, &anchor.current)?;
                        let proof = source.proof(anchor.block - 1, &anchor).await?;
                        if let Some(witness) = root_witness {
                            let second = witness.proof(anchor.block - 1, &anchor).await?;
                            ensure!(
                                proof.snapshot == second.snapshot
                                    && proof.raw_leaf == second.raw_leaf
                                    && witness.api.block_number_to_hash(anchor.block).await?.0
                                        == anchor.block_hash,
                                "permissionless progress changed on witness; HOLD"
                            );
                        }
                        record_external_commitment(
                            ethereum, output, state, &anchor, &proof, accepted,
                        )
                        .await?;
                        reconcile_original_submission(ethereum, output, state).await?;
                        return Ok(Some(anchor));
                    }
                    ensure!(
                        ethereum.checkpoint().await?.block > u64::from(anchor.block),
                        "latest external checkpoint lacks accepted signed evidence; HOLD"
                    );
                    continue 'progress;
                }
            }
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
            return Ok(Some(anchor));
        }
        let proof = source.proof(anchor.block - 1, &anchor).await?;
        let prior = (!state["submission"].is_null()).then(|| state["submission"].clone());
        let client_address = ethereum.client_address;
        let submission_result = ethereum
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
            .await;
        if let Err(error) = submission_result {
            if error.is::<FollowerPersistenceFailed>() {
                return Err(error);
            }
            if let Some(accepted) = accepted_tx(state, ethereum, &anchor).await? {
                if state["submission"]["txHash"] != bytes(accepted) {
                    record_external_commitment(ethereum, output, state, &anchor, &proof, accepted)
                        .await?;
                    reconcile_original_submission(ethereum, output, state).await?;
                    return Ok(Some(anchor));
                }
            }
            if !state["submission"]["txHash"].is_null() {
                let hash: [u8; 32] = array(&state["submission"]["txHash"])?;
                if ethereum
                    .api
                    .raw_provider()
                    .get_transaction_receipt(hash.into())
                    .await?
                    .is_some_and(|receipt| !receipt.status())
                {
                    wait_for_finalized_receipt(ethereum, hash).await?;
                }
            }
            reconcile_original_submission(ethereum, output, state).await?;
            if state["submission"].is_null() {
                return Ok(Some(anchor));
            }
            return Err(error);
        }
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
        return Ok(Some(anchor));
    }
}
async fn follow_external_progress(
    source: &mut Source,
    witness: &Source,
    ethereum: &mut Ethereum,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    loop {
        let retained = state["commitments"]
            .as_array()
            .context("missing commitments")?
            .last()
            .map(|entry| number(&entry["block"]))
            .transpose()?
            .unwrap_or(number(
                &client_bootstrap(state, ethereum.client_address)?["block"],
            )?);
        if ethereum.checkpoint().await?.block <= retained {
            return Ok(());
        }
        advance(source, ethereum, output, state, false, Some(witness)).await?;
    }
}

fn retain_token_root(
    output: &Path,
    state: &mut Value,
    block: u32,
    block_hash: Hash32,
    queue_id: u64,
    queue_root: Hash32,
) -> Result<()> {
    if state["rootMinimum"]
        .as_u64()
        .is_some_and(|floor| u64::from(block) < floor)
    {
        return Ok(());
    }
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
        if root["status"] == "pending" || root["status"] == "mined" {
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
        let (registration, witnessed_registration) = tokio::try_join!(
            source::has_event(&source.api, hash, "GearEthBridge", "QueueMerkleRootChanged"),
            source::has_event(
                &witness.api,
                hash,
                "GearEthBridge",
                "QueueMerkleRootChanged"
            )
        )?;
        ensure!(
            registration == witnessed_registration,
            "source and witness disagree on the finalized queue registration event at block {block}"
        );
        let at_floor = u64::from(block) == number(&state["rootMinimum"])?;
        if registration || at_floor {
            let (first, second) = tokio::try_join!(
                source::queue_snapshot_at(&source.api, hash),
                source::queue_snapshot_at(&witness.api, hash)
            )?;
            ensure!(
                first == second,
                "finalized queue registration differs between authorities"
            );
            ensure!(
                !registration || first.is_some(),
                "queue registration precedes bridge initialization at block {block}; HOLD"
            );
            if let Some((queue_id, root)) = first {
                if registration || root.0 != [0; 32] {
                    retain_token_root(output, state, block, hash.0, queue_id, root.0)?;
                }
            }
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

fn next_queue_progress_block(
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

async fn schedule_queue_progress(
    source: &mut Source,
    witness: &mut Source,
    max_block: u64,
    output: &Path,
    state: &mut Value,
) -> Result<()> {
    if max_block == 0
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
        .filter(|root| {
            root["status"] == "pending"
                && root["block"]
                    .as_u64()
                    .is_some_and(|block| block > max_block)
        })
        .try_fold(None, |earliest: Option<u32>, root| {
            let block = number(&root["block"])?.try_into()?;
            Ok::<_, anyhow::Error>(Some(earliest.map_or(block, |earliest| earliest.min(block))))
        })?;
    let Some(target) =
        next_queue_progress_block(max_block, finalized_height, anchor_block, next_root)
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
        "source authorities disagree on the queue-progress snapshot proof"
    );
    if !first.snapshot.initialized {
        return Ok(());
    }
    retain_token_root(
        output,
        state,
        target,
        first.source_hash,
        first.snapshot.queue_id,
        first.snapshot.queue_root,
    )
}

fn record_root_publication_error(
    output: &Path,
    state: &mut Value,
    key: &str,
    error: anyhow::Error,
) -> Result<()> {
    let finality_pending = matches!(
        error.downcast_ref::<ethereum_client::Error>(),
        Some(ethereum_client::Error::FinalizedAncestryPending)
    );
    if !finality_pending && !error.is::<crate::root_publication::RootPublicationHeld>() {
        return Err(error);
    }
    state["rootPublication"] = json!({
        "status":"held", "root":key, "lastError":format!("{error:#}")
    });
    state["follower"]["status"] = json!(if finality_pending {
        "held-finality"
    } else {
        "held-root"
    });
    state["follower"]["lastError"] = json!(format!("{error:#}"));
    save(output, state, None)
}

#[allow(clippy::too_many_arguments)]
async fn publish_token_roots(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    finality: &cli_utils::EthereumFinalityArgs,
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
            schedule_queue_progress(source, witness, max_block, output, state).await?;
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
            if !has_publication && anchor.is_none_or(|anchor| anchor <= block) {
                uncovered = true;
                continue;
            }
            if !has_publication && root_requires_empty_progress(max_block, block) {
                continue;
            }
            let publication_state = match crate::root_publication::publish_root_locked(
                source_rpc,
                witness_rpc,
                ethereum_rpc,
                finality,
                wallet,
                deployment_manifest,
                &output.join("state.json"),
                registration,
                publication,
                ethereum.owned_fixture_network.as_ref(),
                ethereum.public_network,
            )
            .await
            {
                Ok(status) => status,
                Err(error) => {
                    record_root_publication_error(output, state, &key, error)?;
                    return Ok(uncovered);
                }
            };
            let status = match publication_state {
                crate::root_publication::RootPublicationState::Accepted => "accepted",
                crate::root_publication::RootPublicationState::Mined => "mined",
                crate::root_publication::RootPublicationState::Observed => "observed",
                crate::root_publication::RootPublicationState::Pending => {
                    record_pending_root_publication(ethereum, output, state, &key).await?;
                    return Ok(uncovered);
                }
            };
            max_block = max_block.max(block);
            progressed |= state["roots"][&key]["status"] != status;
            state["roots"][&key]["status"] = json!(status);
            state["rootPublication"] = Value::Null;
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

async fn record_pending_root_publication(
    ethereum: &Ethereum,
    output: &Path,
    state: &mut Value,
    key: &str,
) -> Result<()> {
    let publication = Path::new(
        state["roots"][key]["publication"]
            .as_str()
            .context("root publication path missing")?,
    );
    let intent: Value = serde_json::from_slice(&fs::read(publication)?)?;
    crate::root_publication::validate_root_publication(ethereum, publication, &intent).await?;
    state["roots"][key]["status"] = json!("pending");
    if intent["observedProgress"].is_object() {
        state["follower"]["status"] = json!("catching-up");
        state["follower"]["lastError"] = Value::Null;
        state["rootPublication"] = Value::Null;
    } else {
        state["follower"]["status"] = json!("held-root");
        state["rootPublication"] = json!({"status":"held", "root":key,
            "lastError":"original publication remains held; independent client maintenance continues"});
        state["follower"]["lastError"] =
            json!("original root remains held; retain its exact proof and anchor while following the client");
    }
    save(output, state, None)
}

// Reconnect only at the retained-state boundary: the failed session (including all
// providers) is dropped before waiting, while the deployment owner remains held.
async fn run_token_sessions<F, Fut>(
    output: &Path,
    reconcile_once: bool,
    progressed: &Cell<bool>,
    mut session: F,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut delay = Duration::from_secs(1);
    loop {
        let error = match session().await {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if reconcile_once || !is_transient_follower_error(&error) {
            return Err(error);
        }
        if progressed.replace(false) {
            delay = Duration::from_secs(1);
        }
        let mut state = read_state_file(&output.join("state.json"))?;
        state["follower"]["status"] = json!("reconnecting");
        state["follower"]["lastError"] = json!(format!("{error:#}"));
        // Persistence failure supersedes the transport failure, never another retry.
        save(output, &mut state, None)?;
        eprintln!(
            "BEEFY: reconnecting authenticated token session in {}s: {error:#}",
            delay.as_secs()
        );
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

fn is_transient_follower_error(error: &anyhow::Error) -> bool {
    if error.is::<FollowerPersistenceFailed>() {
        return false;
    }
    if ethereum_client::is_transient_rpc_error(error) {
        return true;
    }
    error.chain().any(|cause| {
        if let Some(error) = cause.downcast_ref::<gsdk::Error>() {
            return match error {
                gsdk::Error::Subxt(error) => is_transient_subxt_error(error),
                gsdk::Error::SubxtRpc(error) => is_transient_source_rpc_error(error),
                gsdk::Error::ClientError(error) => is_transient_source_client_error(error),
                _ => false,
            };
        }
        if let Some(error) = cause.downcast_ref::<subxt::Error>() {
            return is_transient_subxt_error(error);
        }
        cause
            .downcast_ref::<subxt_rpcs::Error>()
            .is_some_and(is_transient_source_rpc_error)
    })
}

fn is_transient_subxt_error(error: &subxt::Error) -> bool {
    match error {
        subxt::Error::Rpc(subxt::error::RpcError::ClientError(error)) => {
            is_transient_source_rpc_error(error)
        }
        subxt::Error::Rpc(subxt::error::RpcError::SubscriptionDropped) => true,
        _ => false,
    }
}

fn is_transient_source_rpc_error(error: &subxt_rpcs::Error) -> bool {
    if error.is_disconnected_will_reconnect() {
        return true;
    }
    match error {
        subxt_rpcs::Error::Client(error) => {
            // Subxt can nest its client envelope during connection establishment.
            if let Some(error) = error.downcast_ref::<subxt_rpcs::Error>() {
                return is_transient_source_rpc_error(error);
            }
            error
                .downcast_ref::<subxt::ext::jsonrpsee::core::ClientError>()
                .is_some_and(is_transient_source_client_error)
        }
        _ => false,
    }
}

fn is_transient_source_client_error(error: &subxt::ext::jsonrpsee::core::ClientError) -> bool {
    use subxt::ext::jsonrpsee::core::ClientError;
    match error {
        ClientError::RequestTimeout => true,
        // A terminated websocket backend must be rebuilt even when its close
        // reason (for example a WebSocket close frame) has no nested OS error.
        ClientError::RestartNeeded(error) => {
            matches!(error.as_ref(), ClientError::Transport(_))
                || is_transient_source_client_error(error)
        }
        ClientError::Transport(error) => {
            ethereum_client::error::is_transient_network_error(error.as_ref())
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn follow_tokens(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    finality: &cli_utils::EthereumFinalityArgs,
    wallet: &Path,
    root_wallet: &Path,
    deployment_manifest: &Path,
    output: &Path,
    reconcile_once: bool,
    local_rehearsal: bool,
    owned_fixture_network: Option<&Path>,
    client_deployment_transaction: Option<B256>,
    public_network: crate::ethereum::PublicEthereumNetwork,
) -> Result<()> {
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    let fixture = crate::ethereum::approved_fixture_network(
        owned_fixture_network,
        &deployment,
        source_rpc,
        witness_rpc,
        ethereum_rpc,
        public_network,
    )?;
    ensure!(
        !(local_rehearsal && fixture.is_some()),
        "owned fixture is not a local rehearsal"
    );
    ensure!(
        !source_rpc.is_empty() && !witness_rpc.is_empty(),
        "source and witness RPCs are required"
    );
    ensure!(
        source_rpc != witness_rpc,
        "two independent source endpoints required"
    );
    ensure!(
        if local_rehearsal || fixture.is_some() {
            crate::local_source_rpc(ethereum_rpc)
        } else {
            ethereum_rpc.starts_with("wss://") || ethereum_rpc.starts_with("https://")
        },
        "Public Ethereum requires HTTPS or WSS; explicit local rehearsal requires a loopback websocket"
    );
    ensure!(
        deployment["mode"] == public_network.deployment_mode(),
        "not a token deployment manifest"
    );
    ensure!(
        deployment["localRehearsal"] == local_rehearsal,
        "deployment rehearsal mode differs from actor mode"
    );
    let _owner = crate::root_publication::lock_token_deployment(deployment_manifest)?;
    fs::create_dir_all(output)?;
    let output_directory = fs::canonicalize(output)?;
    let output = output_directory.as_path();
    let state_path = output.join("state.json");
    let progressed = Cell::new(false);
    run_token_sessions(output, reconcile_once, &progressed, || async {
    let resumed = state_path.exists();
    let mut state = if resumed {
        let state = read_state_file(&state_path)?;
        validate_token_follower_journal(&state, &deployment)?;
        state
    } else {
        json!({"schemaVersion":4, "mode":public_network.follower_mode(), "deployment":deployment,
            "activeEthereum":deployment["ethereum"].clone(),
            "status":"following", "commitments":[], "transactions":[], "roots":{}, "terminalSubmissions":[],
            "startupSequence":0, "rootScan":null, "localRehearsal":local_rehearsal})
    };
    migrate_commitment_journal(&mut state)?;
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
    let mut ethereum = Ethereum::connect(
        ethereum_rpc,
        finality,
        wallet,
        selected_ethereum_manifest,
        fixture.as_ref(),
        public_network,
    )
    .await?;
    ethereum.load_execution_approval(deployment_manifest, finality).await?;
    ethereum.qualify_execution_source(&source).await?;
    ethereum.qualify_execution_source(&witness).await?;
    ethereum.api.enable_finality_archive(
        &output.join("finality-history/headers"), ethereum.api.finality_identity()?.1,
    ).await?;
    ensure!(
        decode(&deployment["manager"])?.as_slice() == ethereum.receiver_address.as_slice(),
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
    let (checkpoint, evidence) = ethereum
        .authenticate_bootstrap(
            &output.join("finality-history"),
            &state["bootstrapDeployment"],
            client_deployment_transaction,
        )
        .await?;
    state["bootstrapDeployment"] = evidence;
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
    if state["bootstrap"].is_null() {
        state["bootstrap"] = bootstrap;
        let floor = ethereum.queue_root_minimum().await?;
        ensure!(
            floor > 0 && floor <= u64::from(u32::MAX),
            "queue immutable root minimum invalid; HOLD"
        );
        let cursor = u32::try_from(floor)?.saturating_sub(1);
        state["rootMinimum"] = json!(floor);
        let cursor_hash = source.api.block_number_to_hash(cursor).await?;
        ensure!(
            witness.api.block_number_to_hash(cursor).await? == cursor_hash,
            "source and witness disagree at initial scan cursor"
        );
        state["rootScan"] = json!({"block":cursor, "blockHash":bytes(cursor_hash.0)});
        save(output, &mut state, Some(&ethereum))?;
    }
    ensure!(
        state["rootMinimum"].is_null()
            || number(&state["rootMinimum"])? == ethereum.queue_root_minimum().await?,
        "immutable queue floor changed; HOLD"
    );
    if state["rootMinimum"].is_null() {
        let floor = ethereum.queue_root_minimum().await?;
        let cursor = u32::try_from(floor)?
            .checked_sub(1)
            .context("invalid immutable root floor")?;
        let hash = source.api.block_number_to_hash(cursor).await?;
        ensure!(
            witness.api.block_number_to_hash(cursor).await? == hash,
            "source/witness floor differs; HOLD"
        );
        state["rootMinimum"] = json!(floor);
        state["rootScan"] = json!({"block":cursor,"blockHash":bytes(hash.0)});
        save(output, &mut state, None)?;
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
                advance(
                    &mut source,
                    &mut ethereum,
                    output,
                    &mut state,
                    false,
                    Some(&witness),
                )
                .await?;
            }
            follow_external_progress(&mut source, &witness, &mut ethereum, output, &mut state)
                .await?;
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
                        Some("pending" | "mined" | "accepted" | "observed")
                    )),
                    "invalid root reconciliation status"
                );
                if roots
                    .values()
                    .all(|root| root["status"] == "accepted" || root["status"] == "observed")
                {
                    break;
                }
                let uncovered = publish_token_roots(
                    source_rpc,
                    witness_rpc,
                    ethereum_rpc,
                    finality,
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
        let scanned = number(&state["rootScan"]["block"])?;
        scan_token_roots(&source, &witness, output, &mut state).await?;
        if number(&state["rootScan"]["block"])? > scanned {
            progressed.set(true);
        }
        publish_token_roots(
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            finality,
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
            follow_external_progress(&mut source, &witness, &mut ethereum, output, &mut state)
                .await?;
            let scanned = number(&state["rootScan"]["block"])?;
            scan_token_roots(&source, &witness, output, &mut state).await?;
            if number(&state["rootScan"]["block"])? > scanned {
                progressed.set(true);
            }
            let uncovered = publish_token_roots(
                source_rpc,
                witness_rpc,
                ethereum_rpc,
                finality,
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
            let handover_only = !uncovered;
            if advance(
                &mut source,
                &mut ethereum,
                output,
                &mut state,
                handover_only,
                Some(&witness),
            )
            .await?.is_some() {
                progressed.set(true);
            }
        }
    }
    .await;
    if let Err(error) = &result {
        if !error.is::<FollowerPersistenceFailed>() {
            state["follower"]["status"] = json!("failed");
            state["follower"]["lastError"] = json!(format!("{error:#}"));
            save(output, &mut state, Some(&ethereum))?;
        }
    }
    result
    }).await
}
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    fn accepted_log(number: u64, hash: u8) -> alloy::rpc::types::Log {
        let mut data = vec![7; 32];
        data.extend_from_slice(&[0; 24]);
        data.extend_from_slice(&42u64.to_be_bytes());
        alloy::rpc::types::Log {
            inner: alloy::primitives::Log::new_unchecked(Address::ZERO, vec![], data.into()),
            block_number: Some(number),
            block_hash: Some(B256::repeat_byte(1)),
            transaction_hash: Some(B256::repeat_byte(hash)),
            transaction_index: Some(0),
            log_index: Some(0),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn accepted_scan_covers_fixed_snapshot_in_bounded_inclusive_batches() -> Result<()> {
        for included in [2_099, 2_100, 12_100] {
            let mut ranges = Vec::new();
            let accepted = accepted_tx_in_range(100, 12_100, [7; 32], 42, |start, end| {
                ranges.push((start, end));
                std::future::ready(Ok(if (start..=end).contains(&included) {
                    vec![accepted_log(included, 9)]
                } else {
                    vec![]
                }))
            })
            .await?;
            assert_eq!(accepted, Some([9; 32]));
            assert_eq!(
                ranges,
                vec![
                    (100, 2_099),
                    (2_100, 4_099),
                    (4_100, 6_099),
                    (6_100, 8_099),
                    (8_100, 10_099),
                    (10_100, 12_099),
                    (12_100, 12_100),
                ]
            );
        }
        let accepted = accepted_tx_in_range(u64::MAX, u64::MAX, [7; 32], 42, |start, end| {
            assert_eq!((start, end), (u64::MAX, u64::MAX));
            std::future::ready(Ok(vec![accepted_log(start, 9)]))
        })
        .await?;
        assert_eq!(accepted, Some([9; 32]));
        Ok(())
    }

    #[tokio::test]
    async fn accepted_scan_rejects_duplicate_identities_across_batches() {
        let error = accepted_tx_in_range(100, 4_100, [7; 32], 42, |start, _| {
            std::future::ready(Ok(vec![accepted_log(
                start,
                if start == 100 { 9 } else { 10 },
            )]))
        })
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("multiple accepted commitment identities"));
    }

    #[tokio::test]
    async fn accepted_scan_holds_on_invalid_coordinates_and_later_rpc_failure() {
        let original = accepted_log(100, 9);
        let mut missing = original.clone();
        missing.block_number = None;
        let mut removed = original.clone();
        removed.removed = true;
        for log in [missing, removed, accepted_log(2_100, 9)] {
            assert!(accepted_tx_in_range(100, 2_099, [7; 32], 42, |_, _| {
                std::future::ready(Ok(vec![log.clone()]))
            })
            .await
            .is_err());
        }
        let error = accepted_tx_in_range(100, 2_100, [7; 32], 42, |start, _| {
            std::future::ready(if start == 100 {
                Ok(vec![original.clone()])
            } else {
                Err(
                    alloy::transports::TransportErrorKind::http_error(400, "range rejected".into())
                        .into(),
                )
            })
        })
        .await
        .unwrap_err();
        assert!(!is_transient_follower_error(&error));
    }

    fn disconnected_source() -> anyhow::Error {
        subxt::Error::Rpc(subxt::error::RpcError::ClientError(
            subxt_rpcs::Error::DisconnectedWillReconnect("test disconnect".into()),
        ))
        .into()
    }

    #[tokio::test]
    async fn reconnect_is_terminal_for_holds_permanent_errors_and_reconcile_once() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let progress = Cell::new(false);
        for error in [
            anyhow::anyhow!("source identity changed; HOLD"),
            anyhow::anyhow!("finalized hash changed; HOLD"),
            ethereum_client::Error::FinalizedAncestryConflict.into(),
            alloy::transports::TransportErrorKind::http_error(401, "timeout".into()).into(),
            subxt_rpcs::Error::User(subxt_rpcs::UserError::method_not_found()).into(),
            std::io::Error::from(std::io::ErrorKind::AddrNotAvailable).into(),
            disconnected_source().context(FollowerPersistenceFailed),
            anyhow::anyhow!("RPC transport error: connection reset"),
        ] {
            let calls = Cell::new(0);
            let error = std::cell::RefCell::new(Some(error));
            assert!(run_token_sessions(directory.path(), false, &progress, || {
                calls.set(calls.get() + 1);
                let error = error
                    .borrow_mut()
                    .take()
                    .expect("terminal error must not retry");
                async { Err(error) }
            })
            .await
            .is_err());
            assert_eq!(calls.get(), 1);
        }
        assert!(
            run_token_sessions(directory.path(), true, &progress, || async {
                Err(disconnected_source())
            })
            .await
            .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn reconnect_preserves_cancellation_and_failed_journal_writes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut state = json!({"follower":{}});
        save(directory.path(), &mut state, None)?;
        let progress = Cell::new(false);
        let calls = Cell::new(0);
        let result = tokio::time::timeout(
            Duration::from_millis(20),
            run_token_sessions(directory.path(), false, &progress, || {
                calls.set(calls.get() + 1);
                async { Err(disconnected_source()) }
            }),
        )
        .await;
        assert!(
            result.is_err(),
            "cancellation must escape the reconnect wait"
        );
        assert_eq!(calls.get(), 1);
        let original = fs::read(directory.path().join("state.json"))?;
        fs::create_dir(directory.path().join("state.json.tmp"))?;
        let error = run_token_sessions(directory.path(), false, &progress, || async {
            Err(disconnected_source())
        })
        .await
        .unwrap_err();
        assert!(error.is::<FollowerPersistenceFailed>());
        assert_eq!(fs::read(directory.path().join("state.json"))?, original);
        Ok(())
    }

    #[test]
    fn reconnect_classifies_only_typed_source_transports() {
        use subxt::ext::jsonrpsee::core::ClientError;
        for error in [
            disconnected_source(),
            subxt::Error::Rpc(subxt::error::RpcError::SubscriptionDropped).into(),
            gsdk::Error::ClientError(ClientError::RequestTimeout).into(),
            subxt_rpcs::Error::Client(Box::new(ClientError::Transport(Box::new(
                std::io::Error::from(std::io::ErrorKind::AddrNotAvailable),
            ))))
            .into(),
        ] {
            assert!(is_transient_follower_error(&error));
        }
        let rejection = ClientError::Call(subxt::ext::jsonrpsee::types::ErrorObjectOwned::owned(
            -32603,
            "timeout, connection reset",
            None::<()>,
        ));
        assert!(!is_transient_follower_error(
            &gsdk::Error::ClientError(rejection).into()
        ));
    }

    #[tokio::test]
    #[ignore = "requires paired owned native archive RPCs and SOURCE_SCAN_QUEUED_BLOCK; read-only fault injection"]
    async fn native_root_scan_faults_preserve_cursor() -> Result<()> {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use subxt_rpcs::client::{
            RawRpcFuture, RawRpcSubscription, RawValue, RpcClient, RpcClientT,
        };

        // Forward the existing archive, changing only the pinned event-storage response.
        struct EventRpc {
            upstream: RpcClient,
            hash: H256,
            events_key: String,
            hide: Option<bool>,
            observations: Arc<AtomicUsize>,
        }
        impl RpcClientT for EventRpc {
            fn request_raw<'a>(
                &'a self,
                method: &'a str,
                params: Option<Box<RawValue>>,
            ) -> RawRpcFuture<'a, Box<RawValue>> {
                Box::pin(async move {
                    if method == "chain_getFinalizedHead" {
                        return serde_json::value::to_raw_value(&self.hash)
                            .map_err(subxt_rpcs::Error::Deserialization);
                    }
                    let arguments: Value =
                        serde_json::from_str(params.as_ref().map_or("[]", |p| p.get()))
                            .map_err(subxt_rpcs::Error::Deserialization)?;
                    if matches!(method, "state_getStorage" | "state_getStorageAt")
                        && arguments[0] == self.events_key
                        && arguments[1] == json!(self.hash)
                    {
                        self.observations.fetch_add(1, Ordering::SeqCst);
                        match self.hide {
                            Some(true) => {
                                return serde_json::value::to_raw_value("0x00")
                                    .map_err(subxt_rpcs::Error::Deserialization)
                            }
                            None => {
                                return Err(subxt_rpcs::Error::User(
                                    subxt_rpcs::UserError::method_not_found(),
                                ))
                            }
                            Some(false) => {}
                        }
                    }
                    self.upstream.request_raw(method, params).await
                })
            }
            fn subscribe_raw<'a>(
                &'a self,
                sub: &'a str,
                params: Option<Box<RawValue>>,
                unsub: &'a str,
            ) -> RawRpcFuture<'a, RawRpcSubscription> {
                self.upstream.subscribe_raw(sub, params, unsub)
            }
        }

        let rpc = std::env::var("BEEFY_TEST_SOURCE_RPC")?;
        let witness_rpc = std::env::var("BEEFY_TEST_WITNESS_RPC")?;
        ensure!(
            rpc != witness_rpc
                && crate::local_source_rpc(&rpc)
                && crate::local_source_rpc(&witness_rpc),
            "paired owned loopback archives required"
        );
        let queued: u32 = std::env::var("SOURCE_SCAN_QUEUED_BLOCK")?.parse()?;
        ensure!(queued > 1, "queued block must have a predecessor");
        let (mut source, mut witness) = Source::connect_pair(
            GearApi::new(&rpc, 3).await?,
            GearApi::new(&witness_rpc, 3).await?,
        )
        .await?;
        let primary_api = source.api.clone();
        let witness_api = witness.api.clone();
        let hash = primary_api.block_number_to_hash(queued).await?;
        let previous = primary_api.block_number_to_hash(queued - 1).await?;
        ensure!(
            source::has_event(
                &primary_api,
                hash,
                "GearEthBridge",
                "QueueMerkleRootChanged"
            )
            .await?
                && source::has_event(
                    &witness_api,
                    hash,
                    "GearEthBridge",
                    "QueueMerkleRootChanged"
                )
                .await?,
            "fixture requires a genuine finalized queue registration"
        );
        let events_key =
            bytes([sp_core::twox_128(b"System"), sp_core::twox_128(b"Events")].concat());
        for at_floor in [false, true] {
            for (primary, secondary) in [
                (Some(true), Some(false)),
                (Some(false), Some(true)),
                (Some(true), None),
                (None, Some(true)),
            ] {
                let observations = Arc::new(AtomicUsize::new(0));
                for (target, upstream, hide) in [
                    (&mut source, &primary_api, primary),
                    (&mut witness, &witness_api, secondary),
                ] {
                    target.api = GearApi {
                        api: gsdk::Api::from_rpc_client(RpcClient::new(EventRpc {
                            upstream: upstream.api.rpc().clone(),
                            hash,
                            events_key: events_key.clone(),
                            hide,
                            observations: observations.clone(),
                        }))
                        .await?,
                    };
                }
                let directory = tempfile::tempdir()?;
                let mut state = json!({"rootMinimum":if at_floor {queued} else {1},"rootScan":{"block":queued - 1,"blockHash":bytes(previous.0)},"roots":{}});
                save(directory.path(), &mut state, None)?;
                let original = state.clone();
                let disk = fs::read(directory.path().join("state.json"))?;
                let error = scan_token_roots(&source, &witness, directory.path(), &mut state)
                    .await
                    .unwrap_err();
                ensure!(
                    observations.load(Ordering::SeqCst) > 0,
                    "fault did not reach actual event RPC"
                );
                if primary.is_some() && secondary.is_some() {
                    ensure!(
                        error
                            .to_string()
                            .contains("disagree on the finalized queue registration event"),
                        "unexpected scan failure: {error:#}"
                    );
                    ensure!(
                        observations.load(Ordering::SeqCst) == 2,
                        "both event observations were not requested"
                    );
                }
                ensure!(
                    state == original && fs::read(directory.path().join("state.json"))? == disk,
                    "fault advanced cursor or retained a root"
                );
            }
        }
        for (target, upstream) in [(&mut source, &primary_api), (&mut witness, &witness_api)] {
            target.api = GearApi {
                api: gsdk::Api::from_rpc_client(RpcClient::new(EventRpc {
                    upstream: upstream.api.rpc().clone(),
                    hash,
                    events_key: events_key.clone(),
                    hide: Some(false),
                    observations: Arc::new(AtomicUsize::new(0)),
                }))
                .await?,
            };
        }
        let directory = tempfile::tempdir()?;
        let mut state = json!({"rootMinimum":queued,"rootScan":{"block":queued - 1,"blockHash":bytes(previous.0)},"roots":{}});
        scan_token_roots(&source, &witness, directory.path(), &mut state).await?;
        ensure!(
            number(&state["rootScan"]["block"])? >= u64::from(queued)
                && state["roots"]
                    .as_object()
                    .context("roots")?
                    .values()
                    .any(|root| root["block"] == queued),
            "matching real observations did not retain the queued root"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires independent archived native RPCs and initialization/queued block pins; read-only"]
    async fn live_native_root_scan_crosses_uninitialized_floor() -> Result<()> {
        let rpc = std::env::var("SOURCE_IDENTITY_RPC")?;
        let witness_rpc = std::env::var("SOURCE_IDENTITY_WITNESS_RPC")?;
        ensure!(
            rpc != witness_rpc
                && crate::local_source_rpc(&rpc)
                && crate::local_source_rpc(&witness_rpc),
            "independent loopback read-only endpoints required"
        );
        let genesis: [u8; 32] = decode(&json!(std::env::var("SOURCE_SCAN_GENESIS")?))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid genesis"))?;
        let initialized: u32 = std::env::var("SOURCE_SCAN_INITIALIZED_BLOCK")?.parse()?;
        let queued: u32 = std::env::var("SOURCE_SCAN_QUEUED_BLOCK")?.parse()?;
        ensure!(
            initialized > 1 && queued > initialized,
            "invalid native boundary pins"
        );
        let (source, witness) = Source::connect_pair(
            GearApi::new(&rpc, 3).await?,
            GearApi::new(&witness_rpc, 3).await?,
        )
        .await?;
        ensure!(
            source.source_genesis == genesis && witness.source_genesis == genesis,
            "native test genesis differs; HOLD"
        );
        for block in [1, initialized - 1, initialized, queued] {
            let hash = source.api.block_number_to_hash(block).await?;
            ensure!(
                witness.api.block_number_to_hash(block).await? == hash,
                "native boundary fork"
            );
            let (first, second) = tokio::try_join!(
                source::queue_snapshot_at(&source.api, hash),
                source::queue_snapshot_at(&witness.api, hash)
            )?;
            ensure!(first == second, "native boundary snapshot differs");
            if block < initialized {
                ensure!(
                    first.is_none(),
                    "pre-initialization became an initialized zero root"
                );
            } else {
                let (_, root) = first.context("initialized native root is absent")?;
                ensure!(
                    (root.0 == [0; 32]) == (block == initialized),
                    "unexpected native root"
                );
            }
            println!("native boundary block={block} hash={hash:?} snapshot={first:?}");
        }
        let directory = tempfile::tempdir()?;
        let mut state =
            json!({"rootMinimum":1,"rootScan":{"block":0,"blockHash":bytes(genesis)},"roots":{}});
        scan_token_roots(&source, &witness, directory.path(), &mut state).await?;
        ensure!(
            number(&state["rootScan"]["block"])? >= u64::from(queued),
            "scan did not cover the genuine queued block"
        );
        let roots = state["roots"]
            .as_object()
            .context("root inventory missing")?;
        ensure!(
            roots.values().all(|root| root["block"] != 1),
            "invented floor root"
        );
        ensure!(
            roots
                .values()
                .any(|root| root["block"] == queued && root["kind"] == "merkleRoot"),
            "genuine queued epoch was lost"
        );
        println!(
            "native scan cursor={} roots={}",
            state["rootScan"],
            roots.len()
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires paired owned native archive RPCs; read-only scheduler smoke"]
    async fn live_idle_scheduler_skips_authenticated_same_set_distance() -> Result<()> {
        let rpc = std::env::var("BEEFY_TEST_SOURCE_RPC")?;
        let witness_rpc = std::env::var("BEEFY_TEST_WITNESS_RPC")?;
        ensure!(
            crate::local_source_rpc(&rpc) && crate::local_source_rpc(&witness_rpc),
            "scheduler smoke requires owned loopback source RPCs"
        );
        let (mut source, witness) = Source::connect_pair(
            GearApi::new(&rpc, 3).await?,
            GearApi::new(&witness_rpc, 3).await?,
        )
        .await?;
        let head = source
            .api
            .block_hash_to_number(source.api.latest_finalized_block().await?)
            .await?;
        source.seek_from(head.saturating_sub(128).max(1))?;
        let mut initial = source.next_commitment().await?;
        let mut skipped = 0;
        loop {
            let candidate = source.next_commitment().await?;
            let second = witness
                .recapture(candidate.block, candidate.raw.clone())
                .await?;
            ensure!(
                candidate.block_hash == second.block_hash
                    && candidate.validated.mmr_root == second.validated.mmr_root
                    && candidate.current == second.current
                    && candidate.next == second.next,
                "scheduler candidate differs on independent witness; HOLD"
            );
            if candidate.current.id != initial.current.id {
                // A handover is mandatory, never counted as an idle same-set skip.
                ensure!(
                    candidate.current == initial.next,
                    "ordered source skipped handover"
                );
                initial = candidate;
                skipped = 0;
                continue;
            }
            ensure!(
                !same_set_commitment_due(
                    true,
                    u64::from(initial.block),
                    candidate.block,
                    candidate.block,
                    None
                ),
                "idle same-set update became payable"
            );
            skipped += 1;
            if candidate.block.saturating_sub(initial.block) >= 32 {
                let proof = source.proof(candidate.block - 1, &candidate).await?;
                let second = witness.proof(candidate.block - 1, &candidate).await?;
                ensure!(
                    proof.snapshot == second.snapshot && proof.raw_leaf == second.raw_leaf,
                    "scheduler freshness proof differs on witness; HOLD"
                );
                ensure!(
                    same_set_commitment_due(
                        true,
                        u64::from(initial.block),
                        candidate.block,
                        candidate.block,
                        Some(candidate.block - 1)
                    ),
                    "covering demand batch was skipped"
                );
                println!(
                    "{}",
                    json!({"sourceGenesis":bytes(source.source_genesis),
                    "runtimeCadence":source.identity.runtime["babe"],
                    "initialBlock":initial.block,"candidateBlock":candidate.block,
                    "sameSetSkipped":skipped,"authoritySet":candidate.current.id,
                    "candidateHash":bytes(candidate.block_hash),"idlePaidUpdates":0,
                    "freshness":freshness_evidence(&proof)?,
                    "signedCommitmentScale":bytes(&candidate.raw),"mmrRoot":bytes(candidate.validated.mmr_root),
                    "coveringDemandAccepted":true,"sourceWrites":false,"ethereumWrites":false})
                );
                return Ok(());
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires paired owned native empty-queue archives, original bootstrap, and owned Anvil"]
    async fn live_native_empty_heartbeat_before_original_expiry() -> Result<()> {
        let rpc = std::env::var("BEEFY_TEST_SOURCE_RPC")?;
        let witness_rpc = std::env::var("BEEFY_TEST_WITNESS_RPC")?;
        ensure!(
            crate::local_source_rpc(&rpc) && crate::local_source_rpc(&witness_rpc),
            "native heartbeat smoke requires owned loopback source RPCs"
        );
        let journal = std::env::var("BEEFY_TEST_SOURCE_JOURNAL")?;
        let recorded: Value = serde_json::from_slice(&fs::read(&journal)?)?;
        let bootstrap = &recorded["commitments"][0];
        let directory = Path::new(&journal).with_extension("heartbeat");
        fs::create_dir(&directory).context("heartbeat evidence directory must be new")?;
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
        ensure!(
            initial_proof.snapshot.initialized && initial_proof.snapshot.queue_root == [0; 32],
            "native heartbeat requires an authenticated initialized-empty bootstrap"
        );
        let mut ethereum = Ethereum::prepare(&directory, &initial, &initial_proof, &source).await?;
        let before = ethereum.checkpoint().await?;
        let original_expiry_ms = before.source_timestamp_ms + 24 * 60 * 60 * 1_000;
        let due_seconds = before.source_timestamp_ms.div_ceil(1_000) + 12 * 60 * 60;
        let provider = ethereum.api.raw_provider().clone();
        // Advance only the disposable destination clock; native slots and source timestamps stay real.
        provider
            .client()
            .request::<_, Value>("evm_setNextBlockTimestamp", (due_seconds,))
            .await?;
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x1",))
            .await?;
        ensure!(
            client_heartbeat_due(before.source_timestamp_ms, due_seconds * 1_000),
            "original source timestamp did not make the heartbeat due"
        );
        source.seek_from(
            initial
                .block
                .checked_add(1)
                .context("source height overflow")?,
        )?;
        let anchor = source.next_commitment().await?;
        let witnessed = witness.recapture(anchor.block, anchor.raw.clone()).await?;
        ensure!(
            anchor.block_hash == witnessed.block_hash
                && anchor.current == witnessed.current
                && anchor.next == witnessed.next
                && anchor.current.id == before.current_id,
            "native heartbeat must be an independently witnessed same-set update"
        );
        let proof = source.proof(anchor.block - 1, &anchor).await?;
        let second = witness.proof(anchor.block - 1, &anchor).await?;
        ensure!(
            proof.snapshot == second.snapshot
                && proof.raw_leaf == second.raw_leaf
                && proof.snapshot.initialized
                && proof.snapshot.queue_root == [0; 32],
            "heartbeat snapshot is not a witnessed native initialized-empty queue"
        );
        let mut state = json!({"submission":null});
        ethereum
            .submit_commitment(
                &anchor,
                &proof.leaf,
                &proof.snapshot,
                &proof.simplified,
                None,
                |transaction, receipt| {
                    state["submission"] = transaction.clone();
                    if let Some(receipt) = receipt {
                        state["receipt"] = serde_json::to_value(receipt)?;
                    }
                    save(&directory, &mut state, None)
                },
            )
            .await?;
        let tx_hash = B256::from(array::<32>(&state["submission"]["txHash"])?);
        let (destination_block, destination_hash) = ethereum
            .verify_accepted_commitment(tx_hash, &anchor, &proof.snapshot)
            .await?;
        let inclusion = provider
            .get_block_by_hash(destination_hash)
            .await?
            .context("native heartbeat inclusion block")?;
        ensure!(
            inclusion.header.timestamp * 1_000 < original_expiry_ms,
            "native heartbeat was not included before the original source deadline"
        );
        let after = ethereum.checkpoint().await?;
        ensure!(
            after.source_timestamp_ms == proof.snapshot.source_timestamp_ms
                && after.source_timestamp_ms < inclusion.header.timestamp * 1_000,
            "heartbeat renewed freshness from destination time instead of authenticated source time"
        );
        state["evidence"] = json!({
            "sourceGenesis":bytes(source.source_genesis),"sourceWrites":false,
            "destinationClockAdvanceSeconds":12 * 60 * 60,
            "bootstrapBlock":initial.block,"block":anchor.block,
            "signedCommitmentScale":bytes(&anchor.raw),"mmrRoot":bytes(anchor.validated.mmr_root),
            "freshness":freshness_evidence(&proof)?,
            "originalSourceTimestampMs":before.source_timestamp_ms,
            "originalExpiryMs":original_expiry_ms,
            "destinationBlock":destination_block,"destinationHash":bytes(destination_hash),
            "inclusionTimestampMs":inclusion.header.timestamp * 1_000,"txHash":bytes(tx_hash)
        });
        save(&directory, &mut state, None)?;
        println!("native_empty_heartbeat={}", state["evidence"]);
        Ok(())
    }

    #[test]
    fn idle_scheduler_skips_same_set_blocks_without_losing_root_batch_or_heartbeat() {
        for block in [132, 164, 2_500, 14_400] {
            assert!(
                !same_set_commitment_due(true, 100, block, block, None),
                "idle source distance must not trigger a paid same-set update at {block}"
            );
        }
        assert!(same_set_commitment_due(false, 100, 132, 132, None));
        assert!(!same_set_commitment_due(false, 100, 140, 150, Some(139)));
        assert!(!same_set_commitment_due(false, 100, 150, 150, Some(150)));
        assert!(same_set_commitment_due(false, 100, 151, 150, Some(150)));
        assert!(!same_set_commitment_due(true, 200, 300, 300, Some(199)));
        assert!(same_set_commitment_due(true, 200, 300, 300, Some(299)));
        assert!(!same_set_commitment_due(true, 200, 299, 300, Some(299)));
    }

    #[test]
    fn idle_client_refreshes_before_expiry_without_a_root_or_handover() {
        let source = 1_000_000;
        assert!(!client_heartbeat_due(source, source - 1));
        assert!(!client_heartbeat_due(source, source + 43_199_999));
        assert!(client_heartbeat_due(source, source + 43_200_000));
        assert!(client_heartbeat_due(source, source + 86_400_000));
    }

    #[tokio::test]
    async fn queue_publication_hold_preserves_intent_and_schedules_heartbeat_and_handover(
    ) -> Result<()> {
        use alloy::{eips::Encodable2718, signers::local::PrivateKeySigner, sol_types::SolCall};
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(7))?;
        let input = ethereum_client::abi::IMessageQueue::submitMerkleRootCall {
            blockNumber: alloy::primitives::U256::from(99),
            merkleRoot: B256::repeat_byte(9),
            proof: vec![1, 2].into(),
        }
        .abi_encode();
        let raw = crate::ethereum::signed_test_transaction(
            &signer,
            4,
            Address::repeat_byte(4),
            100_000,
            input,
        )
        .await?
        .encoded_2718();
        let directory = tempfile::tempdir()?;
        let publication = directory.path().join("root.json");
        let intent = json!({
            "nonce":"4", "txHash":bytes(keccak256(&raw)), "rawTransaction":bytes(&raw),
            "acceptedAnchorTx":bytes([2;32]), "queueProof":"0x0102",
            "proof":{"anchorBlock":100,"anchorRoot":bytes([3;32])},
        });
        fs::write(&publication, serde_json::to_vec(&intent)?)?;
        let original_bytes = fs::read(&publication)?;
        let mut state = json!({
            "follower":{"status":"healthy"}, "roots":{
                "original":{"block":99,"status":"pending","publication":publication}
            }
        });
        let error = anyhow::anyhow!("MessageQueue challenge prevents original submission")
            .context(crate::root_publication::RootPublicationHeld);
        record_root_publication_error(directory.path(), &mut state, "original", error)?;
        let restored = read_state_file(&directory.path().join("state.json"))?;
        assert_eq!(restored["rootPublication"]["status"], "held");
        assert_eq!(restored["roots"]["original"]["status"], "pending");
        let pending = pending_token_root_block(&restored)?;
        assert!(!covered_root_ready_for_publication(
            &restored,
            100,
            pending,
            Duration::ZERO,
        ));
        assert!(!covered_root_ready_for_publication(
            &restored,
            100,
            pending,
            ROOT_PUBLICATION_RETRY_INTERVAL - Duration::from_nanos(1),
        ));
        assert!(covered_root_ready_for_publication(
            &restored,
            100,
            pending,
            ROOT_PUBLICATION_RETRY_INTERVAL,
        ));
        assert!(!covered_root_ready_for_publication(
            &restored,
            99,
            pending,
            ROOT_PUBLICATION_RETRY_INTERVAL,
        ));
        let mut unheld = restored.clone();
        unheld["rootPublication"] = Value::Null;
        assert!(covered_root_ready_for_publication(
            &unheld,
            100,
            pending,
            Duration::ZERO,
        ));
        // Ordinary same-set work stays idle, but the actual scheduler still selects
        // the twelve-hour heartbeat and every mandatory next-set handover.
        assert!(!client_commitment_due(true, (100, 8), (132, 7), 132, None));
        assert!(client_commitment_due(
            !client_heartbeat_due(1_000, 43_201_000),
            (100, 8),
            (132, 7),
            132,
            None,
        ));
        assert!(client_commitment_due(true, (100, 8), (101, 8), 132, None));
        assert_eq!(fs::read(&publication)?, original_bytes);
        assert_eq!(restored["roots"], state["roots"]);
        // Source/proof/canonical failures are not reclassified as an ordinary queue hold.
        let before = state.clone();
        assert!(record_root_publication_error(
            directory.path(),
            &mut state,
            "original",
            anyhow::anyhow!("source authorities disagree on authenticated root"),
        )
        .is_err());
        assert_eq!(state, before);
        Ok(())
    }

    #[tokio::test]
    async fn provenance_accessor_preserves_original_and_rejects_mixed_external_identity(
    ) -> Result<()> {
        use alloy::{eips::Encodable2718, signers::local::PrivateKeySigner};
        let signer = PrivateKeySigner::from_bytes(&B256::from([7; 32]))?;
        let client = Address::from([3; 20]);
        let raw = crate::ethereum::signed_test_transaction(&signer, 36, client, 21_000, vec![])
            .await?
            .encoded_2718();
        let original = json!({"clientAddress":format!("{client:#x}"),"block":420,"blockHash":bytes([8;32]),"root":bytes([9;32]),"txHash":bytes(keccak256(&raw)),"nonce":36,"rawTransaction":bytes(&raw),"sender":format!("{:#x}",signer.address()),"chainId":560_048});
        let directory = tempfile::tempdir()?;
        let mut state = json!({"submission":original,"follower":{}});
        save(directory.path(), &mut state, None)?;
        let calls = Cell::new(0);
        let progressed = Cell::new(false);
        run_token_sessions(directory.path(), false, &progressed, || {
            calls.set(calls.get() + 1);
            let attempt = calls.get();
            let original = &original;
            let directory = directory.path();
            async move {
                let retained = read_state_file(&directory.join("state.json"))?;
                assert_eq!(&retained["submission"], original);
                crate::ethereum::signed_transaction(
                    &decode(&retained["submission"]["rawTransaction"])?,
                    B256::from(array::<32>(&retained["submission"]["txHash"])?),
                    number(&retained["submission"]["nonce"])?,
                    client,
                )?;
                match attempt {
                    1 => Err(disconnected_source()),
                    2 => Err(ethereum_client::Error::ErrorInHTTPTransport(
                        alloy::transports::TransportErrorKind::custom(std::io::Error::from(
                            std::io::ErrorKind::AddrNotAvailable,
                        )),
                    )
                    .into()),
                    _ => Ok(()),
                }
            }
        })
        .await?;
        assert_eq!(calls.get(), 3);
        let own = json!({"origin":"own","block":420,"blockHash":bytes([8;32]),"txHash":original["txHash"],"submission":original});
        assert_eq!(commitment_transaction(&own)?, &original);
        let external = json!({"origin":"external","block":420,"blockHash":bytes([8;32]),"txHash":original["txHash"],"acceptedTransaction":original});
        assert_eq!(commitment_transaction(&external)?, &original);
        for (field, value) in [
            ("sender", json!(format!("{:#x}", Address::ZERO))),
            ("chainId", json!(1)),
            ("nonce", json!(37)),
            ("txHash", json!(bytes([0; 32]))),
        ] {
            let mut changed = external.clone();
            changed["acceptedTransaction"][field] = value;
            assert!(commitment_transaction(&changed).is_err());
        }
        let mut mixed = external.clone();
        mixed["submission"] = original.clone();
        assert!(commitment_transaction(&mixed).is_err());
        mixed["submission"] = Value::Null;
        mixed["origin"] = Value::Null;
        assert!(commitment_transaction(&mixed).is_err());
        assert_eq!(own["submission"], original);
        let mut terminal_state = json!({"submission":original,"terminalSubmissions":[],"follower":{"lastSuccessfulUpdate":400,"lastObservedFinalizedUpdate":420}});
        archive_terminal_submission(
            &mut terminal_state,
            json!({"submission":original,"outcome":"held-consumed-nonce","nonceDisposition":{"block":10,"blockHash":bytes([1;32]),"accountNonce":37,"pinnedNonceConsumed":true}}),
        )?;
        assert!(terminal_state["submission"].is_null());
        assert_eq!(
            terminal_state["terminalSubmissions"][0]["submission"],
            original
        );
        assert_eq!(terminal_state["follower"]["heldOriginalSubmissions"], 1);
        assert_eq!(terminal_state["follower"]["failedOriginalSubmissions"], 0);
        assert_eq!(terminal_state["follower"]["lastSuccessfulUpdate"], 400);
        assert_eq!(
            terminal_state["follower"]["lastObservedFinalizedUpdate"],
            420
        );
        let before = terminal_state.clone();
        assert!(archive_terminal_submission(
            &mut terminal_state,
            json!({"submission":original,"outcome":"finalized-revert"})
        )
        .is_err());
        assert_eq!(terminal_state, before);
        Ok(())
    }

    #[test]
    fn root_inventory_accepts_owned_archive_but_rejects_unindexed_entries() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let publication = directory.path().join("original.json");
        fs::write(&publication, b"{}")?;
        let registered = std::collections::BTreeSet::from([publication.as_path()]);
        let archive = directory.path().join("finality-history");
        fs::create_dir(&archive)?;
        validate_root_publication_inventory(directory.path(), &registered)?;

        let unindexed = directory.path().join("unindexed.json");
        fs::write(&unindexed, b"{}")?;
        assert!(validate_root_publication_inventory(directory.path(), &registered).is_err());
        fs::remove_file(&unindexed)?;
        fs::create_dir(&unindexed)?;
        assert!(validate_root_publication_inventory(directory.path(), &registered).is_err());
        fs::remove_dir(&unindexed)?;

        fs::remove_dir(&archive)?;
        fs::write(&archive, b"{}")?;
        assert!(validate_root_publication_inventory(directory.path(), &registered).is_err());
        #[cfg(unix)]
        {
            fs::remove_file(&archive)?;
            std::os::unix::fs::symlink(directory.path(), &archive)?;
            assert!(validate_root_publication_inventory(directory.path(), &registered).is_err());
        }
        Ok(())
    }

    #[test]
    fn historical_nonzero_inventory_survives_observed_frontier_and_nonzero_intermediate(
    ) -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut state = json!({"rootMinimum":41,"roots":{},"submission":null});
        retain_token_root(directory.path(), &mut state, 40, [1; 32], 7, [2; 32])?;
        retain_token_root(directory.path(), &mut state, 41, [1; 32], 7, [2; 32])?;
        retain_token_root(directory.path(), &mut state, 43, [3; 32], 8, [4; 32])?;
        retain_token_root(directory.path(), &mut state, 99, [5; 32], 9, [0; 32])?;
        let frontier = format!("99-{}", hex::encode([0; 32]));
        state["roots"][&frontier]["status"] = json!("observed");
        assert_eq!(pending_token_root_block(&state)?, Some(43));
        let intermediate =
            next_queue_progress_block(1, 100_000, 100_001, Some(90_000)).expect("distance target");
        assert_eq!(intermediate, 57_601);
        retain_token_root(
            directory.path(),
            &mut state,
            intermediate,
            [6; 32],
            10,
            [7; 32],
        )?;
        let key = format!("{intermediate}-{}", hex::encode([7; 32]));
        assert_eq!(state["roots"][&key]["kind"], "merkleRoot");
        assert_eq!(state["roots"].as_object().expect("roots").len(), 4);
        assert_eq!(
            state["roots"][format!("41-{}", hex::encode([2; 32]))]["status"],
            "pending"
        );
        Ok(())
    }

    #[test]
    fn empty_progress_crosses_long_gaps_without_skipping_pending_roots() {
        let mut max = 1;
        let mut steps = 0;
        while let Some(next) = next_queue_progress_block(max, 200_000, 200_001, Some(200_000)) {
            assert!(u64::from(next) > max && u64::from(next) - max <= EMPTY_PROGRESS_MAX_DISTANCE);
            max = u64::from(next);
            steps += 1;
        }
        assert_eq!((steps, max), (3, 172_801));
        assert!(!root_requires_empty_progress(max, 200_000));
        assert!(!root_requires_empty_progress(0, 200_000));
        assert!(!root_requires_empty_progress(1, 57_601));
        assert!(root_requires_empty_progress(1, 57_602));
        assert_eq!(next_queue_progress_block(0, 200_000, 200_001, None), None);
        assert_eq!(
            next_queue_progress_block(1, 57_601, 57_601, None),
            Some(57_600)
        );
        assert_eq!(next_queue_progress_block(1, 57_600, 100_000, None), None);
        assert_eq!(next_queue_progress_block(1, 100_000, 0, None), None);
        assert_eq!(
            next_queue_progress_block(1, 100_000, 100_001, Some(57_601)),
            None
        );
        assert_eq!(
            next_queue_progress_block(u64::MAX, u32::MAX, u32::MAX, None),
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
    fn queue_epoch_rollover_and_historical_backfill_keep_each_authenticated_identity() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let mut state = json!({"roots":{}, "rootScan":{"block":90,"blockHash":bytes([1;32])}});
        retain_token_root(directory.path(), &mut state, 89, [2; 32], 8, [3; 32])?;
        retain_token_root(directory.path(), &mut state, 90, [4; 32], 9, [5; 32])?;
        // An older epoch arriving through archive backfill is not today's lane identity.
        retain_token_root(directory.path(), &mut state, 70, [6; 32], 7, [7; 32])?;
        let restored = read_state_file(&directory.path().join("state.json"))?;
        assert_eq!(
            restored["roots"][format!("89-{}", hex::encode([3; 32]))]["queueId"],
            8
        );
        assert_eq!(
            restored["roots"][format!("90-{}", hex::encode([5; 32]))]["queueId"],
            9
        );
        assert_eq!(
            restored["roots"][format!("70-{}", hex::encode([7; 32]))]["queueId"],
            7
        );
        let before = fs::read(directory.path().join("state.json"))?;
        assert!(retain_token_root(directory.path(), &mut state, 89, [2; 32], 9, [3; 32]).is_err());
        assert_eq!(fs::read(directory.path().join("state.json"))?, before);
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
        migrate_commitment_journal(&mut state)?;
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
        let mut ethereum = Ethereum::connect_readonly(
            &ethereum_rpc,
            &crate::ethereum::test_finality_args(std::env::var("BEEFY_TEST_HOODI_WITNESS_RPC")?)?,
            &state["activeEthereum"],
            None,
            crate::ethereum::PublicEthereumNetwork::Hoodi,
        )
        .await?;
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
    #[ignore = "requires an explicitly owned Hoodi-history Anvil fork and copied public evidence; no native source writes"]
    async fn authenticated_competing_root_unfences_only_independent_follower() -> Result<()> {
        use alloy::{
            consensus::{SignableTransaction, TxEip1559},
            eips::Encodable2718,
            network::TxSigner,
            primitives::TxKind,
            signers::local::PrivateKeySigner,
            sol_types::SolCall,
        };
        let rpc = std::env::var("BEEFY_TEST_ROOT_FORK_RPC")?;
        ensure!(
            crate::local_source_rpc(&rpc),
            "root regression requires an owned loopback fork"
        );
        let fixture = std::path::PathBuf::from(std::env::var("BEEFY_TEST_ROOT_FORK_DIR")?);
        let manifest: Value = serde_json::from_slice(&fs::read(fixture.join("deployment.json"))?)?;
        let journal: Value =
            serde_json::from_slice(&fs::read(fixture.join("follower-original.json"))?)?;
        let published: Value =
            serde_json::from_slice(&fs::read(fixture.join("published-root.json"))?)?;
        let ethereum = Ethereum::connect_readonly(
            &rpc,
            &crate::ethereum::test_finality_args(std::env::var("BEEFY_TEST_HOODI_WITNESS_RPC")?)?,
            &manifest["ethereum"],
            None,
            crate::ethereum::PublicEthereumNetwork::Hoodi,
        )
        .await?;
        let provider = ethereum.api.raw_provider();
        let original_head = provider.get_block_number().await?;
        ensure!(
            original_head == 3_810_598,
            "fork differs from authenticated regression pin"
        );
        let target: u32 = number(&published["sourceBlock"])?.try_into()?;
        let root: [u8; 32] = array(&published["root"])?;
        let signer = PrivateKeySigner::from_bytes(&B256::from([19; 32]))?;
        let input = ethereum_client::abi::IMessageQueue::submitMerkleRootCall {
            blockNumber: alloy::primitives::U256::from(target),
            merkleRoot: B256::from(root),
            proof: decode(&published["queueProof"])?.into(),
        }
        .abi_encode();
        let mut transaction = TxEip1559 {
            chain_id: 560_048,
            nonce: 0,
            gas_limit: 1_000_000,
            max_fee_per_gas: 100_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: TxKind::Call(ethereum.queue_address),
            input: input.into(),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut transaction).await?;
        let raw = TxEnvelope::Eip1559(transaction.into_signed(signature)).encoded_2718();
        let hash = B256::from(keccak256(&raw));
        let mut intent = published.clone();
        intent["sender"] = json!(signer.address());
        intent["nonce"] = json!("0");
        intent["rawTransaction"] = json!(bytes(&raw));
        intent["txHash"] = json!(hash);
        intent["status"] = json!("pending");
        for field in [
            "publicationReceipt",
            "observedProgress",
            "originalDisposition",
            "lastError",
        ] {
            intent[field] = Value::Null;
        }
        intent["terminalSubmissions"] = json!([]);
        let original = intent.clone();
        let evidence = fixture.join("pending-reconciliation");
        fs::create_dir(&evidence).context("fork regression output must be new")?;
        let path = evidence.join("root.json");
        fs::write(&path, serde_json::to_vec_pretty(&intent)?)?;
        let mut state = json!({"roots":{"original":{"status":"pending", "publication":path,
            "block":target,"kind":"merkleRoot","queueRoot":published["root"]}}, "follower":{}});
        record_pending_root_publication(&ethereum, &evidence, &mut state, "original").await?;
        ensure!(
            state["follower"]["status"] == "held-root",
            "unobserved original lost its barrier"
        );
        let anchor = (
            number(&intent["proof"]["anchorBlock"])?.try_into()?,
            array(&intent["proof"]["anchorRoot"])?,
            B256::from(array::<32>(&intent["acceptedAnchorTx"])?),
            (
                number(&intent["acceptedCheckpoint"]["block"])?,
                B256::from(array::<32>(&intent["acceptedCheckpoint"]["blockHash"])?),
            ),
        );
        let retained_anchor = journal["commitments"]
            .as_array()
            .context("commitments missing")?
            .iter()
            .find(|entry| {
                commitment_transaction(entry)
                    .is_ok_and(|tx| tx["txHash"] == published["acceptedAnchorTx"])
            })
            .context("original accepted anchor missing")?;
        let source =
            Source::connect(GearApi::new(&std::env::var("BEEFY_TEST_SOURCE_RPC")?, 3).await?)
                .await?;
        let captured_anchor = source
            .recapture(anchor.0, decode(&retained_anchor["rawScale"])?)
            .await?;
        let newest = source
            .proof(captured_anchor.block - 1, &captured_anchor)
            .await?;
        let publisher_state = fixture.join("follower-original.json");
        for _ in 0..2 {
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(25),
                crate::root_publication::wait_for_root_receipt(
                    &ethereum,
                    &path,
                    &mut intent,
                    target,
                    "merkleRoot",
                    root,
                    anchor,
                    (
                        &publisher_state,
                        retained_anchor,
                        &captured_anchor,
                        &newest.snapshot,
                    ),
                ),
            )
            .await
            .context("authenticated progress waited for the 30-second receipt timeout")??;
            ensure!(
                outcome == crate::root_publication::RootPublicationState::Pending,
                "competitor falsely completed the retained signed original"
            );
            record_pending_root_publication(&ethereum, &evidence, &mut state, "original").await?;
            ensure!(
                state["follower"]["status"] == "catching-up"
                    && state["follower"]["lastError"].is_null()
                    && state["roots"]["original"]["status"] == "pending",
                "actual Pending transition retained the global held-root barrier"
            );
        }
        ensure!(
            intent["observedProgress"]["actualTransactionHash"] == published["txHash"]
                && intent["publicationReceipt"].is_null(),
            "external receipt substituted for original success"
        );
        for field in [
            "rawTransaction",
            "txHash",
            "nonce",
            "acceptedAnchorTx",
            "acceptedCheckpoint",
            "proof",
            "queueProof",
        ] {
            ensure!(
                intent[field] == original[field],
                "observation rewrote original {field}"
            );
        }
        crate::root_publication::validate_registered_root_publications(&ethereum, &state["roots"])
            .await?;
        // Replay an authentic already-public next commitment only on this owned fork.
        // This is an EVM actor/barrier regression, not fresh native-source traffic.
        let next = journal["commitments"]
            .as_array()
            .context("missing original commitments")?
            .iter()
            .find(|entry| {
                entry["destinationBlock"]
                    .as_u64()
                    .is_some_and(|block| block > original_head)
            })
            .context("fork fixture lacks the next authentic original commitment")?;
        let next_original = commitment_transaction(next)?;
        let next_hash = B256::from(array::<32>(&next_original["txHash"])?);
        ensure!(
            next_original["sender"] != intent["sender"],
            "commitment signer is not independent"
        );
        let snapshot: Value = provider.client().request("evm_snapshot", ()).await?;
        provider
            .client()
            .request::<_, Value>("evm_setAutomine", (true,))
            .await?;
        let fork_timestamp = provider
            .get_block_by_number(alloy::rpc::types::BlockNumberOrTag::Latest)
            .await?
            .context("fork head missing")?
            .header
            .timestamp;
        provider
            .client()
            .request::<_, Value>(
                "evm_setNextBlockTimestamp",
                (
                    (number(&next["freshnessProof"]["sourceTimestampMs"])? / 1_000 + 12)
                        .max(fork_timestamp + 1),
                ),
            )
            .await?;
        let replayed = provider
            .send_raw_transaction(&decode(&next_original["rawTransaction"])?)
            .await?
            .get_receipt()
            .await?;
        ensure!(
            replayed.status()
                && replayed.transaction_hash == next_hash
                && ethereum.checkpoint().await?.block == number(&next["block"])?
                && provider.get_transaction_count(signer.address()).await? == 0
                && provider.get_transaction_by_hash(hash).await?.is_none(),
            "independent real BEEFY update failed or consumed the retained root original"
        );
        ensure!(
            serde_json::from_slice::<Value>(&fs::read(&path)?)? == intent,
            "independent update changed retained root bytes/nonce/anchor"
        );
        provider
            .client()
            .request::<_, Value>("anvil_setBalance", (signer.address(), "0xde0b6b3a7640000"))
            .await?;
        let reverted = provider
            .send_raw_transaction(&raw)
            .await?
            .get_receipt()
            .await?;
        ensure!(
            !reverted.status() && reverted.transaction_hash == hash,
            "stale original did not retain its distinct failure"
        );
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x100",))
            .await?;
        let outcome = crate::root_publication::wait_for_root_receipt(
            &ethereum,
            &path,
            &mut intent,
            target,
            "merkleRoot",
            root,
            anchor,
            (
                &publisher_state,
                retained_anchor,
                &captured_anchor,
                &newest.snapshot,
            ),
        )
        .await?;
        ensure!(
            outcome == crate::root_publication::RootPublicationState::Observed
                && intent["originalDisposition"] == "reverted"
                && intent["publicationReceipt"].is_null()
                && intent["terminalSubmissions"][0]["original"]["rawTransaction"]
                    == original["rawTransaction"],
            "late original failure was replaced by external target success"
        );
        let before_reorg = state.clone();
        let reverted: bool = provider.client().request("evm_revert", (snapshot,)).await?;
        ensure!(reverted, "owned fork snapshot did not revert");
        ensure!(
            record_pending_root_publication(&ethereum, &evidence, &mut state, "original")
                .await
                .is_err()
                && state == before_reorg,
            "lost original failure evidence silently released reconciliation ownership"
        );
        println!(
            "{}",
            json!({"scope":"source-independent authenticated Hoodi-history Anvil fork",
            "target":target,"observedTransaction":published["txHash"],"original":hash,
            "independentCommitment":next_hash,"independentSourceBlock":next["block"],
            "originalDisposition":intent["originalDisposition"],"publicWrites":false})
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires paired owned native archives, source/handover journals, Foundry and owned Anvil"]
    async fn interactive_external_progress_adopts_handover_and_restarts() -> Result<()> {
        use alloy::providers::WalletProvider;
        let rpc = std::env::var("BEEFY_TEST_SOURCE_RPC")?;
        let witness_rpc = std::env::var("BEEFY_TEST_WITNESS_RPC")?;
        ensure!(
            rpc != witness_rpc
                && crate::local_source_rpc(&rpc)
                && crate::local_source_rpc(&witness_rpc),
            "paired owned loopback archives required"
        );
        let journal = std::env::var("BEEFY_TEST_SOURCE_JOURNAL")?;
        let recorded: Value = serde_json::from_slice(&fs::read(&journal)?)?;
        let handovers: Value =
            serde_json::from_slice(&fs::read(std::env::var("BEEFY_TEST_HANDOVER_JOURNAL")?)?)?;
        let handovers = handovers["commitments"]
            .as_array()
            .context("missing genuine handover originals")?;
        ensure!(
            handovers.len() >= 2,
            "two archived authority handovers required; no synthetic native fallback"
        );
        let directory = Path::new(&journal).with_extension("interactive-recovery");
        fs::create_dir(&directory).context("interactive evidence directory must be new")?;
        let (mut source, witness) = Source::connect_pair(
            GearApi::new(&rpc, 3).await?,
            GearApi::new(&witness_rpc, 3).await?,
        )
        .await?;
        let initial = source
            .recapture(
                number(&recorded["commitments"][0]["block"])?.try_into()?,
                decode(&recorded["commitments"][0]["rawScale"])?,
            )
            .await?;
        let initial_proof = source.proof(initial.block - 1, &initial).await?;
        let mut ethereum = Ethereum::prepare(&directory, &initial, &initial_proof, &source).await?;
        let provider = ethereum.api.raw_provider().clone();
        provider
            .client()
            .request::<_, Value>("anvil_setIntervalMining", (0u64,))
            .await?;
        provider
            .client()
            .request::<_, Value>("evm_setAutomine", (true,))
            .await?;
        let deployment = json!({"anchor":{"sourceGenesis":bytes(source.source_genesis),"bridgeDomain":bytes(source.bridge_domain)},"ethereum":ethereum.manifest().await?});
        let head = source.api.latest_finalized_block().await?;
        let height = source.api.block_hash_to_number(head).await?;
        let mut state = json!({"schemaVersion":4,"terminalSubmissions":[],"mode":"hoodi-message-demo","sourceGenesis":bytes(source.source_genesis),"bridgeDomain":bytes(source.bridge_domain),"ethereum":deployment["ethereum"],"deployment":deployment,"bootstrap":bootstrap_record(&initial, &initial_proof)?,"commitments":[],"transactions":ethereum.transactions,"submission":null,"roots":{},"rootMinimum":1,"rootScan":{"block":height,"blockHash":bytes(head.0)}});
        source.seek_from(initial.block.checked_add(1).context("bootstrap overflow")?)?;
        let owned = source.next_commitment().await?;
        ensure!(
            u64::from(owned.block) < number(&handovers[0]["block"])?,
            "owned original must precede unrelated external handover"
        );
        let owned_proof = source.proof(owned.block - 1, &owned).await?;
        let stopped = ethereum
            .submit_commitment(
                &owned,
                &owned_proof.leaf,
                &owned_proof.snapshot,
                &owned_proof.simplified,
                None,
                |transaction, _| {
                    state["submission"] = transaction.clone();
                    state["submission"]["blockHash"] = json!(bytes(owned.block_hash));
                    state["submission"]["root"] = json!(bytes(owned.validated.mmr_root));
                    save(&directory, &mut state, None)?;
                    if !transaction["rawTransaction"].is_null() {
                        bail!("injected stop before original broadcast");
                    }
                    Ok(())
                },
            )
            .await
            .unwrap_err();
        ensure!(
            stopped
                .to_string()
                .contains("injected stop before original broadcast"),
            "original preparation failed: {stopped:#}"
        );
        let original = state["submission"].clone();
        let original_hash = B256::from(array::<32>(&original["txHash"])?);
        let nonce = number(&original["nonce"])?;
        let own_transactions = ethereum.transactions.clone();
        let sender = provider.default_signer_address();

        for (index, retained) in handovers.iter().enumerate() {
            let anchor = source
                .recapture(
                    number(&retained["block"])?.try_into()?,
                    decode(&retained["rawScale"])?,
                )
                .await?;
            ensure!(
                anchor.block_hash == array::<32>(&retained["blockHash"])?
                    && anchor.current.id == ethereum.checkpoint().await?.next_id,
                "fixture is not the next genuine authority handover"
            );
            let proof = source.proof(anchor.block - 1, &anchor).await?;
            provider
                .client()
                .request::<_, Value>(
                    "evm_setNextBlockTimestamp",
                    (proof.snapshot.source_timestamp_ms.div_ceil(1_000),),
                )
                .await?;
            let accepted = ethereum
                .submit_interactive_fixture(
                    &anchor,
                    &proof.leaf,
                    &proof.snapshot,
                    &proof.simplified,
                )
                .await?;
            ensure!(
                accepted["sender"] != format!("{sender:#x}"),
                "competitor used owned signer"
            );
            let (mut adopter, adopter_witness) = Source::connect_pair(
                GearApi::new(&rpc, 3).await?,
                GearApi::new(&witness_rpc, 3).await?,
            )
            .await?;
            adopter.seek_from(anchor.block)?;
            // Restart from the durable journal before adopting the next external handover.
            state = read_state_file(&directory.join("state.json"))?;
            migrate_commitment_journal(&mut state)?;
            revalidate_finalized_commitments(
                &adopter,
                &adopter_witness,
                &ethereum,
                &state,
                &directory.join("finality-history"),
            )
            .await?;
            follow_external_progress(
                &mut adopter,
                &adopter_witness,
                &mut ethereum,
                &directory,
                &mut state,
            )
            .await?;
            let entries = state["commitments"]
                .as_array()
                .context("adopted commitments")?;
            let entry = entries.last().context("missing interactive adoption")?;
            ensure!(
                entries.len() == index + 1
                    && entry["origin"] == "external"
                    && entry["finalized"] == true
                    && entry["block"] == anchor.block
                    && entry["txHash"] == accepted["txHash"]
                    && commitment_transaction(entry)?["rawTransaction"]
                        == accepted["rawTransaction"],
                "interactive handover was converted, lost or duplicated"
            );
            ensure!(
                state["submission"] == original
                    && state["terminalSubmissions"] == json!([])
                    && ethereum.transactions == own_transactions
                    && state["follower"]["lastSuccessfulUpdate"].is_null()
                    && provider.get_transaction_count(sender).await? == nonce
                    && provider
                        .get_transaction_by_hash(original_hash)
                        .await?
                        .is_none(),
                "external adoption released, broadcast or replaced the unrelated owned original"
            );
        }
        let mut restored = read_state_file(&directory.join("state.json"))?;
        let (mut restarted, restart_witness) = Source::connect_pair(
            GearApi::new(&rpc, 3).await?,
            GearApi::new(&witness_rpc, 3).await?,
        )
        .await?;
        restarted.seek_from(
            number(
                &restored["commitments"]
                    .as_array()
                    .context("commitments")?
                    .last()
                    .context("last commitment")?["block"],
            )?
            .try_into()?,
        )?;
        require_saved_submission(
            &restored["submission"],
            &owned,
            original_hash.into(),
            ethereum.client_address,
            deployment_client(&restored)?,
        )?;
        revalidate_finalized_commitments(
            &restarted,
            &restart_witness,
            &ethereum,
            &restored,
            &directory.join("finality-history"),
        )
        .await?;
        reconcile_original_submission(&mut ethereum, &directory, &mut restored).await?;
        follow_external_progress(
            &mut restarted,
            &restart_witness,
            &mut ethereum,
            &directory,
            &mut restored,
        )
        .await?;
        ensure!(
            restored == state
                && provider.get_transaction_count(sender).await? == nonce
                && provider
                    .get_transaction_by_hash(original_hash)
                    .await?
                    .is_none(),
            "restart altered external history or consumed owned original"
        );
        revalidate_finalized_commitments(
            &source,
            &witness,
            &ethereum,
            &restored,
            &directory.join("finality-history"),
        )
        .await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires archived Gear RPC, Foundry, and owned Anvil"]
    async fn interrupted_commitment_recovers_without_replacement() -> Result<()> {
        use alloy::{
            eips::Encodable2718, primitives::U256, providers::WalletProvider, sol_types::SolEvent,
        };
        use ethereum_client::abi::IMessageQueue;
        use futures::{SinkExt, StreamExt};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        // Forward the owned fixture's existing transports; these are not
        // operationally independent production providers.
        struct RecoveryRpc {
            url: String,
            blocked_receipts: Arc<AtomicUsize>,
            server: tokio::task::JoinHandle<()>,
        }

        impl RecoveryRpc {
            async fn new(
                upstream: alloy::providers::RootProvider,
                reject_receipts: bool,
            ) -> Result<Self> {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let url = format!("ws://{}", listener.local_addr()?);
                let blocked_receipts = Arc::new(AtomicUsize::new(0));
                let blocked = blocked_receipts.clone();
                let server = tokio::spawn(async move {
                    let mut clients = tokio::task::JoinSet::new();
                    loop {
                        tokio::select! {
                            accepted = listener.accept() => {
                                let Ok((stream, _)) = accepted else { break };
                                let upstream = upstream.clone();
                                let blocked = blocked.clone();
                                clients.spawn(async move {
                                    let mut stream = tokio_tungstenite::accept_async(stream).await?;
                                    while let Some(message) = stream.next().await {
                                        let message = match message {
                                            Ok(message) => message,
                                            Err(tokio_tungstenite::tungstenite::Error::Protocol(
                                                tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                                            )) => break,
                                            Err(error) => return Err(error.into()),
                                        };
                                        if message.is_close() { break; }
                                        if !message.is_text() { continue; }
                                        let request: Value = serde_json::from_str(message.to_text()?)?;
                                    let method = request["method"].as_str().context("RPC method missing")?;
                                    let unavailable = matches!(method, "eth_getTransactionReceipt" | "eth_getTransactionByHash" | "eth_getBlockReceipts")
                                        || (matches!(method, "eth_getBlockByHash" | "eth_getBlockByNumber") && request["params"][1] == true);
                                    let response = if reject_receipts && unavailable {
                                        blocked.fetch_add(1, Ordering::SeqCst);
                                        json!({"jsonrpc":"2.0", "id":request["id"],
                                            "error":{"code":-32000, "message":"receipt bodies unavailable"}})
                                    } else {
                                        let result: Value = upstream.client()
                                            .request(method.to_owned(), request["params"].clone()).await?;
                                        json!({"jsonrpc":"2.0", "id":request["id"], "result":result})
                                    };
                                        stream.send(tokio_tungstenite::tungstenite::Message::Text(
                                            serde_json::to_string(&response)?.into(),
                                        )).await?;
                                    }
                                    Ok::<_, anyhow::Error>(())
                                });
                            }
                            joined = clients.join_next(), if !clients.is_empty() => {
                                joined.expect("forwarder task missing")
                                    .expect("recovery RPC forwarding panicked")
                                    .expect("recovery RPC forwarding failed");
                            }
                        }
                    }
                });
                Ok(Self {
                    url,
                    blocked_receipts,
                    server,
                })
            }
        }

        impl Drop for RecoveryRpc {
            fn drop(&mut self) {
                self.server.abort();
            }
        }

        let rpc = std::env::var("BEEFY_TEST_SOURCE_RPC")?;
        let witness_rpc = std::env::var("BEEFY_TEST_WITNESS_RPC")?;
        let journal = std::env::var("BEEFY_TEST_SOURCE_JOURNAL")?;
        let token_stack_path = std::env::var("BEEFY_TEST_TOKEN_STACK")
            .context("full production-entrypoint native root smoke requires actual initialized native inventory")?;
        let relay_binary = std::env::var("BEEFY_TEST_RELAY_BIN").context(
            "full native root smoke requires the compiled owned-fixture beefy-relay binary",
        )?;
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
        let bootstrap = bootstrap_record(&initial, &initial_proof)?;
        let mut ethereum = Ethereum::prepare(directory, &initial, &initial_proof, &source).await?;
        source::local::initialize_bridge(&source.api).await?;
        let root_message = source::local::send_message(
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
        let mut state = json!({"schemaVersion":4,"terminalSubmissions":[],"mode":"hoodi-message-demo", "sourceGenesis":bytes(source.source_genesis), "bridgeDomain":bytes(source.bridge_domain),
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
            let destination_time = provider
                .get_block_by_number(alloy::eips::BlockNumberOrTag::Latest)
                .await?
                .context("fixture destination head missing")?
                .header
                .timestamp;
            provider
                .client()
                .request::<_, Value>(
                    "evm_setNextBlockTimestamp",
                    (proof
                        .snapshot
                        .source_timestamp_ms
                        .div_ceil(1_000)
                        .max(destination_time + 1),),
                )
                .await?;
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
            .await?
            .context("original commitment was not reconciled")?;
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
        let mut legacy = state.clone();
        legacy["schemaVersion"] = json!(3);
        for entry in legacy["commitments"]
            .as_array_mut()
            .context("legacy commitments")?
        {
            entry
                .as_object_mut()
                .context("legacy entry")?
                .remove("origin");
        }
        migrate_commitment_journal(&mut legacy)?;
        ensure!(
            legacy["schemaVersion"] == 4 && legacy["commitments"] == state["commitments"],
            "legacy migration rewrote original bytes or history"
        );
        let mut conflicting = legacy.clone();
        conflicting["schemaVersion"] = json!(3);
        ensure!(
            migrate_commitment_journal(&mut conflicting).is_err(),
            "legacy migration accepted conflicting provenance"
        );
        let mut attached = state.clone();
        attached["commitments"] = json!([]);
        attached["follower"] = json!({});
        attached["submission"] = Value::Null;
        let own_transactions = std::mem::take(&mut ethereum.transactions);
        attached["transactions"] = json!([]);
        for entry in state["commitments"].as_array().context("commitments")? {
            let candidate = source
                .recapture(
                    number(&entry["block"])?.try_into()?,
                    decode(&entry["rawScale"])?,
                )
                .await?;
            let proof = source.proof(candidate.block - 1, &candidate).await?;
            let accepted = accepted_tx(&attached, &ethereum, &candidate)
                .await?
                .context("accepted signed fixture")?;
            record_external_commitment(
                &mut ethereum,
                directory,
                &mut attached,
                &candidate,
                &proof,
                accepted,
            )
            .await?;
            ensure!(
                attached["follower"]["lastSuccessfulUpdate"].is_null(),
                "external attachment fabricated own success"
            );
        }
        ensure!(
            ethereum.transactions.is_empty(),
            "external progress entered own receipt accounting"
        );
        revalidate_finalized_commitments(
            &source,
            &witness,
            &ethereum,
            &attached,
            &directory.join("finality-history"),
        )
        .await?;
        let (constructor, pin) = ethereum
            .authenticate_bootstrap(&directory.join("deployment-bootstrap"), &Value::Null, None)
            .await?;
        ensure!(
            constructor.block == u64::from(initial.block)
                && constructor.root == [0; 32]
                && constructor.source_timestamp_ms == initial_proof.snapshot.source_timestamp_ms
                && constructor.current_root == initial.current.root
                && constructor.next_root == initial.next.root,
            "advanced initial attachment substituted latest checkpoint for immutable bootstrap"
        );
        let (restored, saved_pin) = ethereum
            .authenticate_bootstrap(&directory.join("deployment-bootstrap"), &pin, None)
            .await?;
        ensure!(
            restored.block == constructor.block
                && restored.current_root == constructor.current_root
                && saved_pin == pin,
            "immutable bootstrap restoration changed original constructor evidence"
        );
        let accepted_entry = attached["commitments"]
            .as_array()
            .context("commitments")?
            .last()
            .context("external checkpoint")?;
        ensure!(
            accepted_entry["origin"] == "external"
                && commitment_transaction(accepted_entry)?["txHash"]
                    == state["commitments"][2]["txHash"],
            "initial attachment lost actual accepted identity"
        );
        let signed_original = state["commitments"][0]["submission"].clone();
        let candidate = source
            .recapture(
                number(&state["commitments"][2]["block"])?.try_into()?,
                decode(&state["commitments"][2]["rawScale"])?,
            )
            .await?;
        let proof = source.proof(candidate.block - 1, &candidate).await?;
        let mut separate = state.clone();
        separate["commitments"] = json!([]);
        separate["submission"] = signed_original.clone();
        record_external_commitment(
            &mut ethereum,
            directory,
            &mut separate,
            &candidate,
            &proof,
            array(&state["commitments"][2]["txHash"])?,
        )
        .await?;
        ensure!(
            separate["submission"] == signed_original,
            "competitor observation rewrote or released original signature"
        );
        ethereum.transactions = own_transactions;
        save(directory, &mut state, Some(&ethereum))?;
        let root_anchor = source
            .recapture(
                number(&state["commitments"][2]["block"])?.try_into()?,
                decode(&state["commitments"][2]["rawScale"])?,
            )
            .await?;
        let root_block = root_message.block;
        let root_proof = source.proof(root_block, &root_anchor).await?;
        let expected_root = root_proof.snapshot.queue_root;
        let encoded = crate::evidence::queue_envelope(&root_proof, &root_anchor, &root_message)?;
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
                encoded.clone().into(),
            )
            .nonce(0)
            .into_transaction_request();
        let signed = root_provider.fill(request).await?;
        let raw_root = signed
            .as_envelope()
            .context("root transaction must be signed")?
            .encoded_2718();
        let root_hash = B256::from(crate::keccak256(&raw_root));
        let root_path = directory.join("root-publications").join("original.json");
        fs::create_dir_all(root_path.parent().context("root publication directory")?)?;
        let mut root_intent = json!({
            "schemaVersion":4, "sourceBlock":root_block, "root":bytes(expected_root), "kind":"merkleRoot",
            "localRehearsal":true,"sourceIdentity":{"sourceGenesis":bytes(source.source_genesis),"bridgeDomain":bytes(source.bridge_domain),"sourceDomain":state["deployment"]["ethereum"]["sourceDomain"],"destinationChainId":31337,"destinationQueue":format!("{:#x}",ethereum.queue_address)},
            "acceptedAnchorClient":format!("{:#x}",ethereum.client_address),
            "acceptedAnchorTx":state["commitments"][2]["txHash"],
            "acceptedCheckpoint":{"block":state["commitments"][2]["destinationBlock"], "blockHash":state["commitments"][2]["destinationHash"]},
            "proof":crate::evidence::proof_evidence(&root_proof,&root_anchor,&encoded),"queueProof":bytes(&encoded),
            "sender":format!("{root_sender:#x}"), "nonce":"0",
            "rawTransaction":format!("0x{}",hex::encode(&raw_root)), "txHash":format!("{root_hash:#x}"),
            "publicationReceipt":null
        });
        let roots = json!({"original":{
            "status":"pending", "publication":root_path, "block":root_block,
            "kind":"merkleRoot", "queueRoot":bytes(expected_root)
        }});
        fs::write(&root_path, serde_json::to_vec(&root_intent)?)?;
        crate::root_publication::validate_registered_root_publications(&ethereum, &roots).await?;
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
        crate::root_publication::validate_registered_root_publications(&ethereum, &roots).await?;
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
        crate::root_publication::validate_registered_root_publications(&ethereum, &roots).await?;
        state["roots"] = roots.clone();
        let advanced = loop {
            let candidate = source.next_commitment().await?;
            if candidate.block > root_anchor.block {
                break candidate;
            }
        };
        let advanced_proof = source.proof(advanced.block - 1, &advanced).await?;
        let destination_time = provider
            .get_block_by_number(alloy::eips::BlockNumberOrTag::Latest)
            .await?
            .context("fixture destination head missing")?
            .header
            .timestamp;
        provider
            .client()
            .request::<_, Value>(
                "evm_setNextBlockTimestamp",
                (advanced_proof
                    .snapshot
                    .source_timestamp_ms
                    .div_ceil(1_000)
                    .max(destination_time + 1),),
            )
            .await?;
        ethereum
            .submit_interactive_fixture(
                &advanced,
                &advanced_proof.leaf,
                &advanced_proof.snapshot,
                &advanced_proof.simplified,
            )
            .await?;
        ensure!(
            ethereum.checkpoint().await?.block > u64::from(root_anchor.block),
            "root recovery fixture did not advance past the original anchor"
        );

        {
            use std::os::unix::fs::PermissionsExt;
            alloy::sol! {
                #[sol(rpc)]
                interface RecoveryGovernance {
                    function erc20Manager() external view returns (address);
                }
                #[sol(rpc)]
                interface RecoveryManager {
                    function vftManagers() external view returns (bytes32[] memory);
                }
            }
            let restart_directory = tempfile::tempdir()?;
            let restart_path = restart_directory
                .path()
                .join("root-publications/original.json");
            fs::create_dir_all(restart_path.parent().context("root directory")?)?;
            let publisher_state = restart_directory.path().join("state.json");
            let manifest_path = restart_directory.path().join("deployment.json");
            let wallet_path = restart_directory.path().join("wallet.json");
            fs::write(
                &wallet_path,
                serde_json::to_vec(&json!({
                    "address":format!("{root_sender:#x}"),"private_key":format!("0x{}",hex::encode([19u8;32]))
                }))?,
            )?;
            fs::set_permissions(&wallet_path, fs::Permissions::from_mode(0o600))?;
            let (chain_id, genesis) = ethereum.api.finality_identity()?;
            let fixture = crate::ethereum::OwnedFixtureNetwork {
                schema_version: 1,
                ethereum_chain_id: chain_id,
                ethereum_genesis_hash: genesis,
                // Only the existing owned-Anvil approval path is under test here;
                // this marker is not Beacon-finality qualification.
                beacon_genesis_validators_root: B256::from([1; 32]),
                beacon_genesis_time: 1,
            };
            let governance = queue.governanceAdmin().call().await?;
            let manager = RecoveryGovernance::new(governance, provider.clone())
                .erc20Manager()
                .call()
                .await?;
            let gear_managers = RecoveryManager::new(manager, provider.clone())
                .vftManagers()
                .call()
                .await?;
            let mock_receiver = std::mem::replace(&mut ethereum.receiver_address, manager);
            let mut token_ethereum = ethereum.manifest().await?;
            ethereum.receiver_address = mock_receiver;
            token_ethereum["sourceGenesis"] = json!(bytes(source.source_genesis));
            let token_deployment = json!({
                "mode":"hoodi-token-stack","localRehearsal":false,"ownedFixtureNetwork":fixture,
                "gearManager":format!("{:#x}",gear_managers.first().context("fixture VFT manager missing")?),
                "anchor":{"sourceGenesis":bytes(source.source_genesis),"bridgeDomain":bytes(source.bridge_domain),"sourceIdentity":source.identity},
                "ethereum":token_ethereum
            });
            fs::write(&manifest_path, serde_json::to_vec(&token_deployment)?)?;
            let registration = json!({
                "status":"pending","publication":restart_path,"block":root_block,"blockHash":bytes(root_proof.source_hash),
                "queueId":root_proof.snapshot.queue_id,"kind":"merkleRoot","queueRoot":bytes(expected_root)
            });
            let mut publisher = state.clone();
            publisher["mode"] = json!("hoodi-token-follow");
            publisher["localRehearsal"] = json!(false);
            publisher["deployment"] = token_deployment.clone();
            publisher["activeEthereum"] = token_ethereum;
            publisher["followerSigner"] =
                json!(format!("{:#x}", provider.default_signer_address()));
            publisher["rootPublisherSigner"] = json!(format!("{root_sender:#x}"));
            publisher["roots"] = json!({"original":registration});
            fs::write(&publisher_state, serde_json::to_vec(&publisher)?)?;
            let anchor_entry = &publisher["commitments"][2];
            let anchor_tx = B256::from(array::<32>(&anchor_entry["txHash"])?);
            let cache_name = format!("{}.json", hex::encode(anchor_tx));
            let retained_dir = restart_directory.path().join("finality-history/receipts");
            fs::create_dir_all(&retained_dir)?;
            fs::copy(
                directory
                    .join("finality-history/receipts")
                    .join(&cache_name),
                retained_dir.join(&cache_name),
            )?;
            let anchor_newest = source.proof(root_anchor.block - 1, &root_anchor).await?;
            let anchor_pins = (
                root_anchor.block,
                root_anchor.validated.mmr_root,
                anchor_tx,
                (
                    number(&anchor_entry["destinationBlock"])?,
                    B256::from(array::<32>(&anchor_entry["destinationHash"])?),
                ),
            );
            let mut restart_intent = root_intent.clone();
            restart_intent["localRehearsal"] = json!(false);
            restart_intent["ownedFixtureNetwork"] = serde_json::to_value(&fixture)?;
            restart_intent["publicationReceipt"] = json!({
                "block": finalized_root.included_block_number,
                "blockHash": format!("{:#x}", finalized_root.included_block_hash),
                "kind": "merkleRoot",
                "finalized": true,
                "finalizedBlock": finalized_root.finalized_block_number,
                "finalizedBlockHash": format!("{:#x}", finalized_root.finalized_block_hash),
                "receiptProof": &finalized_root.inclusion,
                "checkpoint": &finalized_root.checkpoint,
            });
            let saved_intent = restart_intent.clone();
            let saved_bytes = serde_json::to_vec(&restart_intent)?;
            fs::write(&restart_path, &saved_bytes)?;
            let primary_rpc = RecoveryRpc::new(provider.root().clone(), true).await?;
            let witness_rpc =
                RecoveryRpc::new(ethereum.api.raw_witness_provider()?.clone(), false).await?;
            let source_witness_rpc = std::env::var("BEEFY_TEST_WITNESS_RPC")?;
            let beacon = ethereum
                ._beacon
                .as_ref()
                .context("signed Beacon fixture missing")?;
            let finality = cli_utils::EthereumFinalityArgs {
                ethereum_witness_rpc: Some(witness_rpc.url.clone()),
                ethereum_finality_beacon_rpc: beacon.beacon_rpc.clone(),
                ethereum_finality_bootstrap: beacon.approval_path.clone(),
                ethereum_finality_bootstrap_sha256: format!("{:#x}", beacon.approval_sha256),
                ethereum_finality_store: restart_directory.path().join("publisher-beacon"),
                ethereum_finality_proof_sources: vec![beacon.directory.clone()],
            };
            let restarted_api = ethereum_client::EthApi::new(
                &primary_rpc.url,
                &format!("{:#x}", ethereum.queue_address),
                None,
                None,
                None,
            )
            .await?
            .with_beacon_finality(
                beacon
                    .config
                    .clone()
                    .with_proof_sources(vec![beacon.directory.clone()])?,
                &finality.ethereum_finality_store,
            )
            .await?
            .with_data_witness(&witness_rpc.url, chain_id, genesis)
            .await?;
            let original_api = std::mem::replace(&mut ethereum.api, restarted_api);
            for _ in 0..2 {
                ensure!(
                    !crate::root_publication::reconcile_root_original(
                        &ethereum,
                        &restart_path,
                        &mut restart_intent,
                    )
                    .await?,
                    "finalized original was archived on restart"
                );
                ensure!(
                    crate::root_publication::wait_for_root_receipt(
                        &ethereum,
                        &restart_path,
                        &mut restart_intent,
                        root_block,
                        "merkleRoot",
                        expected_root,
                        anchor_pins,
                        (
                            &publisher_state,
                            anchor_entry,
                            &root_anchor,
                            &anchor_newest.snapshot
                        ),
                    )
                    .await?
                        == crate::root_publication::RootPublicationState::Accepted,
                    "waiting entrypoint did not accept the retained finalized original"
                );
                ensure!(
                    crate::root_publication::publish_root_locked(
                        &rpc,
                        &source_witness_rpc,
                        &primary_rpc.url,
                        &finality,
                        &wallet_path,
                        &manifest_path,
                        &publisher_state,
                        &registration,
                        &restart_path,
                        Some(&fixture),
                        crate::ethereum::PublicEthereumNetwork::Hoodi,
                    )
                    .await?
                        == crate::root_publication::RootPublicationState::Accepted,
                    "full publisher did not recover the retained original"
                );
                ensure!(
                    restart_intent == saved_intent && fs::read(&restart_path)? == saved_bytes,
                    "restart changed signed original, nonce, inclusion or ownership disposition"
                );
                ensure!(
                    restart_intent["terminalSubmissions"].is_null()
                        && restart_intent["originalDisposition"].is_null()
                        && primary_rpc.blocked_receipts.load(Ordering::SeqCst) == 0,
                    "finalized recovery fetched a receipt body or recorded terminal/nonceConsumed history"
                );
            }
            let corrupt_path = restart_directory.path().join("corrupt.json");
            let mut corrupt_intent = saved_intent.clone();
            let mut corrupt_proof = finalized_root.inclusion.clone();
            corrupt_proof.receipt_proof.clear();
            corrupt_intent["publicationReceipt"]["receiptProof"] =
                serde_json::to_value(corrupt_proof)?;
            let corrupt_original = corrupt_intent.clone();
            let corrupt_bytes = serde_json::to_vec(&corrupt_intent)?;
            fs::write(&corrupt_path, &corrupt_bytes)?;
            ensure!(
                crate::root_publication::reconcile_root_original(
                    &ethereum,
                    &corrupt_path,
                    &mut corrupt_intent,
                )
                .await
                .is_err(),
                "corrupt retained receipt proof was accepted"
            );
            ensure!(
                corrupt_intent == corrupt_original
                    && fs::read(&corrupt_path)? == corrupt_bytes
                    && primary_rpc.blocked_receipts.load(Ordering::SeqCst) == 0,
                "corrupt proof rewrote the original or fell back to receipt bodies"
            );
            let corrupt_anchor_path = retained_dir.join(&cache_name);
            let original_anchor_bytes = fs::read(&corrupt_anchor_path)?;
            let mut corrupt_anchor: Value = serde_json::from_slice(&original_anchor_bytes)?;
            corrupt_anchor["proof"] = json!({});
            fs::write(&corrupt_anchor_path, serde_json::to_vec(&corrupt_anchor)?)?;
            ensure!(
                crate::root_publication::publish_root_locked(
                    &rpc,
                    &source_witness_rpc,
                    &primary_rpc.url,
                    &finality,
                    &wallet_path,
                    &manifest_path,
                    &publisher_state,
                    &registration,
                    &restart_path,
                    Some(&fixture),
                    crate::ethereum::PublicEthereumNetwork::Hoodi,
                )
                .await
                .is_err()
                    && fs::read(&restart_path)? == saved_bytes
                    && primary_rpc.blocked_receipts.load(Ordering::SeqCst) == 0,
                "corrupt anchor proof triggered fallback or changed the signed original"
            );
            ethereum.api = original_api;
        }

        // New signing goes through the real CLI and production authorization gate.
        // The independently passed digest never mutates this process's environment.
        {
            use std::os::unix::fs::PermissionsExt;
            alloy::sol! {
                #[sol(rpc)]
                interface ApprovedFixtureGovernance {
                    function erc20Manager() external view returns (address);
                }
            }
            let approved_dir = directory.join("approved-production-root");
            fs::create_dir(&approved_dir)?;
            let native: crate::execution::fixture::NativeInventory =
                serde_json::from_slice(&fs::read(&token_stack_path)?)?;
            let manager = ApprovedFixtureGovernance::new(
                queue.governanceAdmin().call().await?,
                provider.clone(),
            )
            .erc20Manager()
            .call()
            .await?;
            let mock_receiver = std::mem::replace(&mut ethereum.receiver_address, manager);
            let previous_approval = ethereum.execution.clone();
            let previous_transactions = ethereum.transactions.clone();
            let (manifest, approved_digest) = ethereum
                .install_owned_native_approval(
                    &source,
                    &initial,
                    &initial_proof,
                    &approved_dir,
                    Some(native),
                )
                .await?;
            let owned_network = ethereum
                .owned_fixture_network
                .as_ref()
                .context("owned network missing")?
                .clone();
            let publisher_path = approved_dir.join("state.json");
            let mut publisher = state.clone();
            publisher["mode"] = json!("hoodi-token-follow");
            publisher["localRehearsal"] = json!(false);
            publisher["deployment"] = manifest.clone();
            publisher["activeEthereum"] = manifest["ethereum"].clone();
            publisher["followerSigner"] =
                json!(format!("{:#x}", provider.default_signer_address()));
            publisher["submission"] = Value::Null;
            publisher["roots"] = json!({});
            let accepted = accepted_tx(&publisher, &ethereum, &advanced)
                .await?
                .context("latest native interactive anchor missing")?;
            // The child must own its writable archive; seed through a separate parent journal.
            let seed_dir = approved_dir.join("parent-seed");
            fs::create_dir(&seed_dir)?;
            record_external_commitment(
                &mut ethereum,
                &seed_dir,
                &mut publisher,
                &advanced,
                &advanced_proof,
                accepted,
            )
            .await?;
            let publication_block = advanced_proof.source;
            ensure!(
                publication_block > root_block,
                "new root smoke must not reuse the accepted original's height"
            );
            let published_root = advanced_proof.snapshot.queue_root;
            let kind = if published_root == [0; 32] {
                "emptyProgress"
            } else {
                "merkleRoot"
            };
            let signer =
                alloy::signers::local::PrivateKeySigner::from_bytes(&B256::from([18; 32]))?;
            let sender = signer.address();
            provider
                .client()
                .request::<_, Value>("anvil_setBalance", (sender, "0xde0b6b3a7640000"))
                .await?;
            // Anvil's direct balance edit changes the current block's proof database.
            // Finalize a mined post-funding state before the child authenticates accounts.
            provider
                .client()
                .request::<_, Value>("anvil_mine", ("0x3",))
                .await?;
            let wallet = approved_dir.join("wallet.json");
            fs::write(
                &wallet,
                serde_json::to_vec(&json!({"address":format!("{sender:#x}"),
                "private_key":format!("0x{}",hex::encode([18u8;32]))}))?,
            )?;
            fs::set_permissions(&wallet, fs::Permissions::from_mode(0o600))?;
            let publication = approved_dir.join("root-publications/approved.json");
            fs::create_dir_all(publication.parent().context("publication parent")?)?;
            let registration = json!({"status":"pending","publication":publication,
                "block":publication_block,"blockHash":bytes(advanced_proof.source_hash),
                "queueId":advanced_proof.snapshot.queue_id,"kind":kind,"queueRoot":bytes(published_root)});
            publisher["rootPublisherSigner"] = json!(format!("{sender:#x}"));
            publisher["roots"] = json!({"approved":registration});
            fs::write(&publisher_path, serde_json::to_vec(&publisher)?)?;
            let registration_path = approved_dir.join("registration.json");
            fs::write(&registration_path, serde_json::to_vec(&registration)?)?;
            let network_path = approved_dir.join("owned-network.json");
            fs::write(&network_path, serde_json::to_vec(&owned_network)?)?;
            let upstream = RecoveryRpc::new(provider.root().clone(), false).await?;
            let upstream_witness =
                RecoveryRpc::new(ethereum.api.raw_witness_provider()?.clone(), false).await?;
            let beacon = ethereum
                ._beacon
                .as_ref()
                .context("signed Beacon fixture missing")?;
            provider
                .client()
                .request::<_, Value>("anvil_setIntervalMining", (1u64,))
                .await?;
            let result = tokio::time::timeout(
                Duration::from_secs(180),
                tokio::process::Command::new(&relay_binary)
                    .args([
                        "tokens-publish-root",
                        "--source-rpc",
                        &rpc,
                        "--witness-rpc",
                        &witness_rpc,
                        "--ethereum-rpc",
                        &upstream.url,
                        "--ethereum-witness-rpc",
                        &upstream_witness.url,
                    ])
                    .arg("--wallet")
                    .arg(&wallet)
                    .arg("--deployment-manifest")
                    .arg(approved_dir.join("local-fixture-manifest.json"))
                    .arg("--publisher-state")
                    .arg(&publisher_path)
                    .arg("--registration")
                    .arg(&registration_path)
                    .arg("--output")
                    .arg(&publication)
                    .arg("--owned-fixture-network")
                    .arg(&network_path)
                    .arg("--ethereum-finality-proof-source")
                    .arg(&beacon.directory)
                    .env(
                        "BEEFY_APPROVED_MANIFEST_SHA256",
                        format!("{approved_digest:#x}"),
                    )
                    .env("ETH_FINALITY_BEACON_RPC", &beacon.beacon_rpc)
                    .env("ETH_FINALITY_BOOTSTRAP", &beacon.approval_path)
                    .env(
                        "ETH_FINALITY_BOOTSTRAP_SHA256",
                        format!("{:#x}", beacon.approval_sha256),
                    )
                    .env("ETH_FINALITY_STORE", approved_dir.join("child-beacon"))
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .context("production root entrypoint timed out")??;
            provider
                .client()
                .request::<_, Value>("anvil_setIntervalMining", (0u64,))
                .await?;
            provider
                .client()
                .request::<_, Value>("evm_setAutomine", (true,))
                .await?;
            fs::write(approved_dir.join("publisher.stdout"), &result.stdout)?;
            fs::write(approved_dir.join("publisher.stderr"), &result.stderr)?;
            ensure!(
                result.status.success(),
                "production root entrypoint failed; inspect retained publisher.stderr"
            );
            let original: Value = serde_json::from_slice(&fs::read(&publication)?)?;
            ensure!(original["approvedManifestSha256"] == json!(approved_digest)
                && original["rawTransaction"].as_str().is_some()
                && original["publicationReceipt"]["finalized"] == true,
                "production publisher did not preserve approved signed original and finalized inclusion");
            let raw = decode(&original["rawTransaction"])?;
            let receipt = ethereum
                .api
                .get_finalized_receipt(B256::from(crate::keccak256(&raw)))
                .await?
                .context("production root original receipt missing")?;
            let mut view = ethereum.api.verified_finalized_view().await?;
            view.verify_blocks(
                provider.root(),
                &std::collections::BTreeMap::from([
                    (receipt.included_block_number, receipt.included_block_hash),
                    (receipt.finalized_block_number, receipt.finalized_block_hash),
                ]),
            )
            .await?;
            receipt.inclusion.verify_original_inclusion(
                &view,
                &raw,
                B256::from(crate::keccak256(&raw)),
            )?;
            ethereum.receiver_address = mock_receiver;
            ethereum.execution = previous_approval;
            ethereum.transactions = previous_transactions;
        }

        // Isolate consumer clock/claim changes from the original recovery and reorg fixture.
        {
            let upstream = RecoveryRpc::new(provider.root().clone(), false).await?;
            let fork_url = upstream.url.clone();
            let fork_block = provider.get_block_number().await?;
            let claim_node = tokio::task::spawn_blocking(move || {
                alloy::node_bindings::Anvil::new()
                    .fork(fork_url)
                    .fork_block_number(fork_block)
                    .chain_id(31_337)
                    .port(0_u16)
                    .try_spawn()
            })
            .await??;
            let provider =
                alloy::providers::ProviderBuilder::new().connect_http(claim_node.endpoint_url());
            let queue = IMessageQueue::new(ethereum.queue_address, provider.clone());
            // Historical consumers schedule from the publication block, not a root-keyed getter.
            let root_event = IMessageQueue::MerkleRoot {
                blockNumber: U256::from(root_block),
                merkleRoot: B256::from(expected_root),
            }
            .encode_log_data();
            ensure!(
                receipt.inner.logs().iter().any(|log| {
                    log.address() == ethereum.queue_address && log.inner.data == root_event
                }),
                "native publication lost the historical MerkleRoot event"
            );
            let claimant = *provider
                .get_accounts()
                .await?
                .get(2)
                .context("third owned Anvil account")?;
            let message_nonce = U256::from_be_bytes(root_message.message.nonce_be);
            let claim = queue
                .processMessage(
                    U256::from(root_block),
                    U256::from(root_message.inclusion.num_leaves),
                    U256::from(root_message.inclusion.leaf_index),
                    IMessageQueue::VaraMessage {
                        nonce: message_nonce,
                        source: B256::from(root_message.message.source),
                        destination: Address::from(root_message.message.destination),
                        payload: root_message.message.payload.into(),
                    },
                    root_message
                        .inclusion
                        .proof
                        .into_iter()
                        .map(B256::from)
                        .collect(),
                )
                .from(claimant);
            let early = claim
                .call()
                .await
                .err()
                .context("unmatured native claim succeeded")?;
            ensure!(
                matches!(
                    early.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>(),
                    Some(IMessageQueue::IMessageQueueErrors::MerkleRootDelayNotPassed(_))
                ),
                "early claim failed for a reason other than historical maturity"
            );
            let publication = provider
                .get_block_by_hash(receipt.block_hash.context("root inclusion hash")?)
                .await?
                .context("root publication block")?;
            let maturity = publication.header.timestamp
                + queue.PROCESS_USER_MESSAGE_DELAY().call().await?.to::<u64>();
            provider
                .client()
                .request::<_, Value>("evm_setNextBlockTimestamp", (maturity,))
                .await?;
            provider
                .client()
                .request::<_, Value>("anvil_mine", ("0x1",))
                .await?;
            claim.call().await?;
            let claim_hash: B256 = provider
                .client()
                .request(
                    "eth_sendTransaction",
                    (claim.clone().into_transaction_request(),),
                )
                .await?;
            let claimed = alloy::providers::PendingTransactionBuilder::new(
                provider.root().clone(),
                claim_hash,
            )
            .get_receipt()
            .await
            .context("native message claim receipt")?;
            let processed_event = IMessageQueue::MessageProcessed {
                blockNumber: U256::from(root_block),
                messageHash: B256::from(root_message.message_hash),
                messageNonce: message_nonce,
                messageDestination: ethereum.receiver_address,
            }
            .encode_log_data();
            ensure!(
                claimed.status()
                    && queue.isProcessed(message_nonce).call().await?
                    && claimed.inner.logs().iter().any(|log| {
                        log.address() == ethereum.queue_address && log.inner.data == processed_event
                    }),
                "mature native claim lost replay state or the historical MessageProcessed event"
            );
            let duplicate = claim
                .call()
                .await
                .err()
                .context("duplicate native claim succeeded")?;
            ensure!(
                matches!(
                    duplicate.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>(),
                    Some(IMessageQueue::IMessageQueueErrors::MessageAlreadyProcessed(error))
                        if error.messageNonce == message_nonce
                ),
                "duplicate native claim did not reject its original nonce"
            );
            println!(
                "historical_queue_interface={}",
                json!({"scope":"genuine native proof and isolated owned Anvil fork; historical interface, not deployed consumer artifact",
                    "sourceBlock":root_block,"rootTransaction":root_hash,"claimTransaction":claim_hash,
                    "messageNonce":message_nonce.to_string(),"publicationTimestamp":publication.header.timestamp,
                    "maturityTimestamp":maturity,"early":"MerkleRootDelayNotPassed","duplicate":"MessageAlreadyProcessed"})
            );
        }

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
        let pin_encoded = crate::encode_queue_proof(
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
        let pin_nonce = root_provider.get_transaction_count(root_sender).await?;
        let pin_request = if pin_kind == "emptyProgress" {
            queue
                .submitEmptyQueueProgress(
                    alloy::primitives::U256::from(pin_block),
                    pin_encoded.clone().into(),
                )
                .nonce(pin_nonce)
                .into_transaction_request()
        } else {
            queue
                .submitMerkleRoot(
                    alloy::primitives::U256::from(pin_block),
                    B256::from(pin_proof.snapshot.queue_root),
                    pin_encoded.clone().into(),
                )
                .nonce(pin_nonce)
                .into_transaction_request()
        };
        let pin_signed = root_provider.fill(pin_request).await?;
        let pin_raw = pin_signed
            .as_envelope()
            .context("root pin transaction must be signed")?
            .encoded_2718();
        let pin_intent = json!({"schemaVersion":4,"sourceBlock":pin_block,"root":bytes(pin_proof.snapshot.queue_root),"kind":pin_kind,
            "localRehearsal":true,"sourceIdentity":{"sourceGenesis":bytes(source.source_genesis),"bridgeDomain":bytes(source.bridge_domain),
                "sourceDomain":state["deployment"]["ethereum"]["sourceDomain"],"destinationChainId":31337,"destinationQueue":format!("{:#x}",ethereum.queue_address)},
            "acceptedAnchorTx":bytes(hash),"acceptedAnchorClient":format!("{:#x}",ethereum.client_address),
            "acceptedCheckpoint":{"block":original_commitment["destinationBlock"],"blockHash":original_commitment["destinationHash"]},
            "sender":format!("{root_sender:#x}"),"proof":crate::evidence::proof_evidence(&pin_proof,&anchor,&pin_encoded),"queueProof":bytes(&pin_encoded),
            "nonce":pin_nonce,"rawTransaction":bytes(&pin_raw),"txHash":bytes(keccak256(&pin_raw)),"publicationReceipt":null});
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
        let mut unsigned_intent = pin_intent.clone();
        unsigned_intent["rawTransaction"] = Value::Null;
        unsigned_intent["txHash"] = Value::Null;
        let unsigned_bytes = serde_json::to_vec(&unsigned_intent)?;
        fs::write(&pin_path, &unsigned_bytes)?;
        let mut unsigned_state = before_reorg.clone();
        unsigned_state["roots"]["pinned"] = json!({"block":pin_block,"kind":pin_kind,"queueRoot":bytes(pin_proof.snapshot.queue_root),"status":"pending","publication":pin_path});
        let reincluded = ethereum
            .api
            .get_finalized_receipt(hash.into())
            .await?
            .context("canonical original reinclusion")?;
        authenticate_reinclusion(
            &ethereum,
            directory,
            &unsigned_state,
            commitment_transaction(original_commitment)?,
            &anchor,
            &reincluded,
        )
        .await?;
        ensure!(
            fs::read(&pin_path)? == unsigned_bytes
                && unsigned_intent["nonce"] == pin_nonce
                && fs::read(directory.join("state.json"))? == original_disk_state,
            "unsigned refresh permission changed reserved nonce or original journal"
        );
        fs::write(&pin_path, &pinned_bytes)?;
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
        .await
        .context("revalidating finalized original after reinclusion")?;
        record_mined_commitment(
            &mut ethereum,
            directory,
            &mut interrupted,
            &anchor,
            &proof,
            hash,
        )
        .await
        .context("replaying receipt-only recovery after finality")?;
        promote_finality(&mut source, &mut ethereum, directory, &mut interrupted)
            .await
            .context("promoting receipt-only recovery after finality")?;
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
            .await
            .context("reverting original root snapshot")?;
        ensure!(
            reverted
                && crate::root_publication::validate_registered_root_publications(
                    &ethereum, &roots
                )
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
        // The explicit Anvil rewind invalidates Alloy's cached confirmation history.
        let reinclusion_provider: alloy::providers::RootProvider =
            alloy::providers::RootProvider::new(alloy::rpc::client::RpcClient::new(
                root_provider.client().transport().clone(),
                root_provider.client().is_local(),
            ));
        let reincluded_root = reinclusion_provider
            .send_raw_transaction(&raw_root)
            .await
            .context("rebroadcasting identical root after explicit test reorg")?
            .get_receipt()
            .await
            .context("waiting for identical root reinclusion")?;
        ensure!(
            reincluded_root.status()
                && reincluded_root.transaction_hash == root_hash
                && reincluded_root.block_hash != receipt.block_hash,
            "identical root did not succeed in a different inclusion"
        );
        ensure!(
            crate::root_publication::validate_registered_root_publications(&ethereum, &roots)
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
                .await
                .context("reverting finalized commitment snapshot")?,
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
    fn external_progress_requires_every_authority_handover_identity() -> Result<()> {
        let current = AuthoritySet {
            id: 7,
            keys: vec![vec![1; 33]],
            root: [2; 32],
        };
        let next = AuthoritySet {
            id: 8,
            keys: vec![vec![3; 33]],
            root: [4; 32],
        };
        let previous = json!({"current":current,"next":next});
        require_authority_successor(&previous, &current)?;
        require_authority_successor(&previous, &next)?;
        let mut changed = next.clone();
        changed.root = [5; 32];
        assert!(require_authority_successor(&previous, &changed).is_err());
        changed = next.clone();
        changed.keys[0][0] ^= 1;
        assert!(require_authority_successor(&previous, &changed).is_err());
        changed = next.clone();
        changed.id += 1;
        assert!(require_authority_successor(&previous, &changed).is_err());
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
}
