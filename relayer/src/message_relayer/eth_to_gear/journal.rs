//! Schema-8 storage. Hot pointers are bounded; immutable originals and cold liabilities are not.
use super::{
    super::tx_manager::{Transaction, TxStatus},
    JSONStorage,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub(super) const HOT_LIMIT: usize = 1_000;
pub(super) const PASS_LIMIT: usize = 128;

fn digest(bytes: &[u8]) -> String {
    hex::encode(alloy_primitives::keccak256(bytes))
}
fn checked_name(value: &str) -> Result<&str> {
    ensure!(
        value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit()),
        "HOLD: invalid journal digest"
    );
    Ok(value)
}
fn read_hashed(path: &Path, hash: &str) -> Result<Vec<u8>> {
    checked_name(hash)?;
    let bytes = fs::read(path)
        .with_context(|| format!("HOLD: missing journal evidence {}", path.display()))?;
    ensure!(
        digest(&bytes) == hash,
        "HOLD: journal digest mismatch {}",
        path.display()
    );
    Ok(bytes)
}
// A hard link publishes a fully fsynced immutable file without replacing a prior original.
fn immutable(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        ensure!(
            fs::read(path)? == bytes,
            "HOLD: immutable journal evidence changed"
        );
        return Ok(());
    }
    let parent = path.parent().context("journal parent missing")?;
    fs::create_dir_all(parent)?;
    for directory in parent
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .take(4)
    {
        fs::File::open(directory)?.sync_all()?;
    }
    let temporary = parent.join(format!(".{}.pending", digest(bytes)));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
    {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let partial = fs::read(&temporary)?;
            ensure!(
                bytes.starts_with(&partial),
                "HOLD: contradictory interrupted immutable write"
            );
            if partial.len() < bytes.len() {
                let retained = parent.join(format!(".{}.interrupted", digest(&partial)));
                if retained.exists() {
                    ensure!(
                        fs::read(&retained)? == partial,
                        "HOLD: interrupted evidence collision"
                    );
                } else {
                    fs::hard_link(&temporary, &retained)?;
                }
                fs::File::open(parent)?.sync_all()?;
                fs::remove_file(&temporary)?;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            } else {
                fs::File::open(&temporary)?.sync_all()?;
            }
        }
        Err(error) => return Err(error.into()),
    }
    match fs::hard_link(&temporary, path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => ensure!(
            fs::read(path)? == bytes,
            "HOLD: immutable publication collision"
        ),
        Err(error) => return Err(error.into()),
    }
    fs::File::open(parent)?.sync_all()?;
    // Retain the owned pending hardlink: it is non-authoritative crash evidence, not a new intent.
    Ok(())
}

pub(super) fn reject_interrupted(root: &Path) -> Result<()> {
    ensure!(!root.join("state.write-intent").exists(),
        "HOLD: interrupted inbound record publication; retain originals and qualify the write intent before recovery");
    Ok(())
}
pub(super) fn begin_write(root: &Path, previous: Option<&Value>, incoming: &Value) -> Result<()> {
    reject_interrupted(root)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("state.write-intent"))?;
    file.write_all(&serde_json::to_vec(
        &json!({"schema_version":8,"previous_state":previous,"incoming":incoming}),
    )?)?;
    file.sync_all()?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}
pub(super) fn finish_write(root: &Path) -> Result<()> {
    fs::remove_file(root.join("state.write-intent"))?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

/// Replay only a complete durable intent. Its originals are reused byte-for-byte;
/// a truncated/contradictory intent stays fenced, not inferred to have had no effect.
pub(super) async fn recover_write(root: &Path) -> Result<()> {
    let marker = root.join("state.write-intent");
    if !marker.exists() {
        return Ok(());
    }
    super::super::cutover::reject_sealed(root)?;
    let intent: Value = serde_json::from_slice(&fs::read(&marker)?)
        .context("HOLD: incomplete inbound write intent; preserve original bytes")?;
    ensure!(
        intent["schema_version"] == 8
            && intent["incoming"].is_object()
            && (intent["previous_state"].is_null() || intent["previous_state"].is_object()),
        "HOLD: malformed inbound write intent"
    );
    let previous = (!intent["previous_state"].is_null()).then_some(&intent["previous_state"]);
    let current: Option<Value> = if root.join("state.json").exists() {
        Some(serde_json::from_slice(&fs::read(root.join("state.json"))?)?)
    } else {
        None
    };
    let next = encode_state(root, intent["incoming"].clone(), previous).await?;
    ensure!(
        current.as_ref() == previous || current.as_ref() == Some(&next),
        "HOLD: interrupted inbound intent conflicts with committed state"
    );
    decode_hot(root, next.clone())?;
    let temporary = root.join("state.json.new");
    if temporary.exists() {
        let interrupted = fs::read(&temporary)?;
        immutable(
            &root.join("interrupted").join(digest(&interrupted)),
            &interrupted,
        )?;
        fs::remove_file(&temporary)?;
        fs::File::open(root)?.sync_all()?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(&next)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, root.join("state.json"))?;
    fs::File::open(root)?.sync_all()?;
    finish_write(root)
}
fn blob_path(root: &Path, hash: &str) -> Result<PathBuf> {
    Ok(root.join("evidence").join(checked_name(hash)?))
}
fn compact(root: &Path, value: &mut Value) -> Result<()> {
    match value {
        Value::Array(items)
            if items.len() >= 64 && items.iter().all(|v| v.as_u64().is_some_and(|n| n <= 255)) =>
        {
            let bytes: Vec<u8> = items.iter().map(|v| v.as_u64().unwrap() as u8).collect();
            let hash = digest(&bytes);
            immutable(&blob_path(root, &hash)?, &bytes)?;
            *value = json!({"$journal_bytes": hash, "length": bytes.len()});
        }
        Value::Array(items) => {
            for item in items {
                compact(root, item)?;
            }
        }
        Value::Object(items) => {
            ensure!(
                !items.contains_key("$journal_bytes"),
                "HOLD: reserved journal evidence key in input"
            );
            for item in items.values_mut() {
                compact(root, item)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn expand(root: &Path, value: &mut Value) -> Result<()> {
    if let Some(hash) = value.get("$journal_bytes").and_then(Value::as_str) {
        ensure!(
            value.as_object().is_some_and(|v| v.len() == 2),
            "HOLD: malformed evidence reference"
        );
        let bytes = read_hashed(&blob_path(root, hash)?, hash)?;
        ensure!(
            value["length"].as_u64() == Some(bytes.len() as u64),
            "HOLD: evidence length changed"
        );
        *value = serde_json::to_value(bytes)?;
    } else {
        match value {
            Value::Array(items) => {
                for item in items {
                    expand(root, item)?;
                }
            }
            Value::Object(items) => {
                for item in items.values_mut() {
                    expand(root, item)?;
                }
            }
            _ => (),
        }
    }
    Ok(())
}
fn record_path(root: &Path, uuid: Uuid, hash: &str) -> Result<PathBuf> {
    Ok(root
        .join("records")
        .join(uuid.to_string())
        .join(format!("{}.json", checked_name(hash)?)))
}
pub(super) fn read_record(root: &Path, uuid: Uuid, hash: &str) -> Result<Value> {
    let mut record: Value =
        serde_json::from_slice(&read_hashed(&record_path(root, uuid, hash)?, hash)?)?;
    ensure!(
        record["uuid"].as_str() == Some(uuid.to_string().as_str()),
        "HOLD: record UUID changed"
    );
    if let Some(previous) = record["previous"].as_str() {
        read_hashed(&record_path(root, uuid, previous)?, previous)?;
    }
    expand(root, &mut record["transaction"])?;
    let tx: Transaction = serde_json::from_value(record["transaction"].clone())?;
    ensure!(tx.uuid == uuid, "HOLD: transaction UUID changed");
    validate_attempts(&tx)?;
    Ok(record["transaction"].take())
}
fn validate_versions(root: &Path, uuid: Uuid, current: &str) -> Result<()> {
    let mut next = Some(current.to_owned());
    while let Some(hash) = next {
        let mut record: Value =
            serde_json::from_slice(&read_hashed(&record_path(root, uuid, &hash)?, &hash)?)?;
        ensure!(
            record["uuid"] == json!(uuid),
            "HOLD: historical version changed UUID"
        );
        expand(root, &mut record["transaction"])?;
        let tx: Transaction = serde_json::from_value(record["transaction"].take())?;
        ensure!(tx.uuid == uuid, "HOLD: historical transaction UUID changed");
        validate_attempts(&tx)?;
        next = record["previous"].as_str().map(str::to_owned);
    }
    Ok(())
}
pub(super) fn validate_attempts(tx: &Transaction) -> Result<()> {
    ensure!(
        !is_cold(&tx.status) || tx.receipt.is_some(),
        "HOLD: cold liability lost original receipt evidence"
    );
    if matches!(tx.status, TxStatus::Unprocessable { .. }) {
        ensure!(
            tx.receipt
                .as_ref()
                .is_some_and(|receipt| receipt.signed_submission.is_none()
                    && receipt.submission_attempts.is_empty()
                    && receipt.handed_off_at_ms.is_none()),
            "HOLD: unprocessable original still owns a signed attempt"
        );
    }
    if let Some(receipt) = &tx.receipt {
        ensure!(
            receipt.submission_attempts.len() <= PASS_LIMIT,
            "HOLD: proof attempt bound reached; retain originals"
        );
        for signed in receipt.signed_submission.iter().chain(
            receipt
                .submission_attempts
                .iter()
                .map(|a| &a.signed_submission),
        ) {
            ensure!(
                signed.native_reconciliation_attempts.len() <= PASS_LIMIT,
                "HOLD: native attempt bound exceeded; retain originals"
            );
            signed.validate_native_reconciliation()?;
        }
    }
    Ok(())
}
fn write_record(
    root: &Path,
    uuid: Uuid,
    mut transaction: Value,
    previous: Option<&str>,
) -> Result<String> {
    let tx: Transaction = serde_json::from_value(transaction.clone())?;
    ensure!(uuid == tx.uuid, "HOLD: record key changed");
    validate_attempts(&tx)?;
    if let Some(previous) = previous {
        let old = read_record(root, uuid, previous)?;
        let known = serde_json::to_value(serde_json::from_value::<Transaction>(old.clone())?)?;
        JSONStorage::preserve_unknown_json(&mut transaction, &known, &old);
        if transaction == old {
            return Ok(previous.to_owned());
        }
    }
    compact(root, &mut transaction)?;
    let bytes =
        serde_json::to_vec(&json!({"uuid":uuid,"previous":previous,"transaction":transaction}))?;
    let hash = digest(&bytes);
    immutable(&record_path(root, uuid, &hash)?, &bytes)?;
    Ok(hash)
}
fn key(tx: &crate::message_relayer::common::TxHashWithSlot) -> String {
    digest(format!("{}-{}", tx.slot_number.0, hex::encode(tx.tx_hash)).as_bytes())
}
fn index_node(root: &Path, hash: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&read_hashed(
        &root.join("ownership").join(checked_name(hash)?),
        hash,
    )?)?)
}
fn save_node(root: &Path, node: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(node)?;
    let hash = digest(&bytes);
    immutable(&root.join("ownership").join(&hash), &bytes)?;
    Ok(hash)
}
// Persistent sparse radix index: authenticated absence costs at most 65 small reads,
// independent of lifetime history. A missing referenced node is HOLD, never absence.
fn owner_at(root: &Path, mut hash: Option<String>, key: &str) -> Result<Option<Value>> {
    checked_name(key)?;
    for depth in 0..=64 {
        let Some(current) = hash else {
            return Ok(None);
        };
        let node = index_node(root, &current)?;
        if let Some(stored) = node["key"].as_str() {
            checked_name(stored)?;
            return Ok((stored == key).then(|| node["owner"].clone()));
        }
        ensure!(
            depth < 64 && node["children"].is_object(),
            "HOLD: invalid ownership index"
        );
        hash = node["children"]
            .get(&key[depth..depth + 1])
            .map(|v| {
                v.as_str()
                    .context("HOLD: malformed ownership child")
                    .map(str::to_owned)
            })
            .transpose()?;
    }
    anyhow::bail!("HOLD: ownership index depth exceeded")
}
fn insert_owner(
    root: &Path,
    hash: Option<&str>,
    key: &str,
    owner: &Value,
    depth: usize,
) -> Result<String> {
    let Some(hash) = hash else {
        return save_node(root, &json!({"key":key,"owner":owner}));
    };
    let mut node = index_node(root, hash)?;
    if let Some(stored) = node["key"].as_str() {
        checked_name(stored)?;
        if stored == key {
            ensure!(
                node["owner"] == *owner,
                "HOLD: immutable cold ownership changed"
            );
            return Ok(hash.to_owned());
        }
        ensure!(depth < 64, "HOLD: ownership key collision");
        let old_child = stored[depth..depth + 1].to_owned();
        node = json!({"children":{old_child:hash}});
    }
    ensure!(
        depth < 64 && node["children"].is_object(),
        "HOLD: invalid ownership branch"
    );
    let digit = &key[depth..depth + 1];
    let next = node["children"]
        .get(digit)
        .map(|v| v.as_str().context("HOLD: malformed ownership child"))
        .transpose()?;
    let child = insert_owner(root, next, key, owner, depth + 1)?;
    node["children"][digit] = json!(child);
    save_node(root, &node)
}
fn archive_path(root: &Path, hash: &str) -> Result<PathBuf> {
    Ok(root
        .join("archive")
        .join(format!("{}.json", checked_name(hash)?)))
}
fn read_entry(root: &Path, hash: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&read_hashed(
        &archive_path(root, hash)?,
        hash,
    )?)?)
}
fn archive(root: &Path, state: &mut Value, uuid: Uuid, hash: &str, tx: &Transaction) -> Result<()> {
    let tx_key = key(&tx.tx);
    if let Some(owner) = owner_at(
        root,
        state["ownership_root"].as_str().map(str::to_owned),
        &tx_key,
    )? {
        ensure!(
            owner["uuid"] == json!(uuid) && owner["record"] == hash,
            "HOLD: cold ownership changed"
        );
        return Ok(());
    }
    let sequence = state["archive_count"]
        .as_u64()
        .unwrap_or(0)
        .checked_add(1)
        .context("archive sequence overflow")?;
    let entry = json!({"sequence":sequence,"previous":state["archive_head"],"uuid":uuid,"record":hash,"tx":tx.tx});
    let bytes = serde_json::to_vec(&entry)?;
    let entry_hash = digest(&bytes);
    immutable(&archive_path(root, &entry_hash)?, &bytes)?;
    let owner = json!({"uuid":uuid,"record":hash,"entry":entry_hash});
    let ownership_root = insert_owner(root, state["ownership_root"].as_str(), &tx_key, &owner, 0)?;
    state["ownership_root"] = json!(ownership_root);
    state["archive_head"] = json!(entry_hash);
    state["archive_count"] = json!(sequence);
    Ok(())
}
/// Validate an indexed cold identity before deduplication. It never grants retry authority.
pub(super) fn cold_contains(
    root: &Path,
    tx: &crate::message_relayer::common::TxHashWithSlot,
) -> Result<bool> {
    let state: Value = serde_json::from_slice(&fs::read(root.join("state.json"))?)?;
    let Some(owner) = owner_at(
        root,
        state["ownership_root"].as_str().map(str::to_owned),
        &key(tx),
    )?
    else {
        return Ok(false);
    };
    let entry = read_entry(
        root,
        owner["entry"]
            .as_str()
            .context("HOLD: missing ownership entry")?,
    )?;
    ensure!(
        entry["sequence"]
            .as_u64()
            .is_some_and(|n| n > 0 && n <= state["archive_count"].as_u64().unwrap_or(0)),
        "HOLD: cold ownership not committed"
    );
    ensure!(
        entry["uuid"] == owner["uuid"]
            && entry["record"] == owner["record"]
            && entry["tx"] == serde_json::to_value(tx)?,
        "HOLD: cold ownership identity mismatch"
    );
    let uuid: Uuid = serde_json::from_value(owner["uuid"].clone())?;
    let record = read_record(
        root,
        uuid,
        owner["record"]
            .as_str()
            .context("HOLD: missing owner record")?,
    )?;
    let record: Transaction = serde_json::from_value(record)?;
    ensure!(
        record.tx.tx_hash == tx.tx_hash
            && record.tx.slot_number == tx.slot_number
            && is_cold(&record.status),
        "HOLD: cold transaction identity/status changed"
    );
    Ok(true)
}
pub(super) fn is_cold(status: &TxStatus) -> bool {
    matches!(status, TxStatus::Completed | TxStatus::Unprocessable { .. })
}
/// Publish records and ownership first; caller atomically publishes returned state before acknowledging ingress.
pub(super) async fn encode_state(
    root: &Path,
    mut incoming: Value,
    previous: Option<&Value>,
) -> Result<Value> {
    let old_schema = previous.and_then(|p| p["schema_version"].as_u64());
    if old_schema.is_some_and(|n| n < 8) {
        let bytes = fs::read(root.join("state.json"))?;
        if serde_json::from_slice::<Value>(&bytes)?["schema_version"]
            .as_u64()
            .is_some_and(|n| n < 8)
        {
            immutable(
                &root.join("legacy").join(format!("{}.json", digest(&bytes))),
                &bytes,
            )?;
        }
    }
    let mut state = previous.cloned().unwrap_or_else(|| json!({}));
    if old_schema != Some(8) {
        state["archive_head"] = Value::Null;
        state["archive_count"] = json!(0);
        state["ownership_root"] = Value::Null;
    }
    for field in [
        "runtime_identity",
        "manual_identity",
        "predecessor_cutover",
        "failed",
    ] {
        if let Some(value) = incoming.get_mut(field) {
            state[field] = value.take();
        }
    }
    state["schema_version"] = json!(8);
    // A manual journal owns exactly one operation; keep its completion available for
    // mandatory rerun reconciliation instead of putting lifetime history on the hot path.
    let manual = !state["manual_identity"].is_null();
    let mut hot = serde_json::Map::new();
    let mut processed = 0usize;
    for section in ["transactions", "completed"] {
        let rows = incoming[section]
            .as_object_mut()
            .context("HOLD: missing transaction inventory")?;
        for (id, row) in rows {
            let uuid = Uuid::parse_str(id)?;
            let tx: Transaction = serde_json::from_value(row.clone())?;
            let old = previous
                .and_then(|p| p["transactions"].get(id))
                .and_then(Value::as_str);
            let hash = write_record(root, uuid, row.take(), old)?;
            if is_cold(&tx.status) && !manual {
                archive(root, &mut state, uuid, &hash, &tx)?;
                if let Some(failed) = state["failed"].as_object_mut() {
                    failed.remove(id);
                }
            } else {
                ensure!(
                    hot.insert(id.clone(), json!(hash)).is_none(),
                    "HOLD: duplicate active UUID"
                );
            }
            processed += 1;
            if processed.is_multiple_of(PASS_LIMIT) {
                tokio::task::yield_now().await;
            }
        }
    }
    ensure!(
        hot.len() <= HOT_LIMIT,
        "HOLD: inbound active capacity exceeded; no evidence truncated"
    );
    ensure!(
        !manual || hot.len() <= 1,
        "HOLD: manual journal owns more than one operation"
    );
    state["transactions"] = Value::Object(hot);
    state["completed"] = json!({});
    Ok(state)
}
pub(super) fn decode_hot(root: &Path, mut state: Value) -> Result<Value> {
    if state["schema_version"].as_u64() != Some(8) {
        return Ok(state);
    }
    let rows = state["transactions"]
        .as_object()
        .context("HOLD: missing hot references")?;
    ensure!(
        rows.len() <= HOT_LIMIT,
        "HOLD: active reference bound exceeded"
    );
    let mut hot = serde_json::Map::new();
    let mut completed = serde_json::Map::new();
    for (id, hash) in rows {
        let row = read_record(
            root,
            Uuid::parse_str(id)?,
            hash.as_str().context("HOLD: malformed hot reference")?,
        )?;
        if row["status"].as_str() == Some("Completed") {
            completed.insert(id.clone(), row);
        } else {
            hot.insert(id.clone(), row);
        }
    }
    if let Some(head) = state["archive_head"].as_str() {
        let entry = read_entry(root, head)?;
        ensure!(
            entry["sequence"] == state["archive_count"],
            "HOLD: archive head count mismatch"
        );
    } else {
        ensure!(
            state["archive_count"].as_u64().unwrap_or(0) == 0,
            "HOLD: archive head missing"
        );
    }
    if let Some(hash) = state["ownership_root"].as_str() {
        index_node(root, hash)?;
    } else {
        ensure!(
            state["archive_count"].as_u64().unwrap_or(0) == 0,
            "HOLD: ownership root missing"
        );
    }
    state["transactions"] = Value::Object(hot);
    state["completed"] = Value::Object(completed);
    Ok(state)
}
/// Explicit offline inventory/requalification path; never invoked by the new-deposit scheduler.
pub(super) fn inventory(root: &Path) -> Result<Value> {
    reject_interrupted(root)?;
    let raw: Value = serde_json::from_slice(&fs::read(root.join("state.json"))?)?;
    if raw["schema_version"].as_u64() == Some(8) {
        for (id, hash) in raw["transactions"]
            .as_object()
            .context("HOLD: hot inventory missing")?
        {
            validate_versions(
                root,
                Uuid::parse_str(id)?,
                hash.as_str().context("HOLD: hot digest missing")?,
            )?;
        }
    }
    let mut state = decode_hot(root, raw)?;
    if state["schema_version"].as_u64() != Some(8) {
        return Ok(state);
    }
    let mut head = state["archive_head"].as_str().map(str::to_owned);
    let mut remaining = state["archive_count"].as_u64().unwrap_or(0);
    while let Some(hash) = head {
        let entry = read_entry(root, &hash)?;
        ensure!(
            remaining > 0 && entry["sequence"].as_u64() == Some(remaining),
            "HOLD: archive chain discontinuity"
        );
        let uuid: Uuid = serde_json::from_value(entry["uuid"].clone())?;
        validate_versions(
            root,
            uuid,
            entry["record"]
                .as_str()
                .context("HOLD: archive record missing")?,
        )?;
        let row = read_record(
            root,
            uuid,
            entry["record"]
                .as_str()
                .context("HOLD: archive record missing")?,
        )?;
        let tx: Transaction = serde_json::from_value(row.clone())?;
        ensure!(
            is_cold(&tx.status) && entry["tx"] == serde_json::to_value(&tx.tx)?,
            "HOLD: archive identity changed"
        );
        ensure!(
            cold_contains(root, &tx.tx)?,
            "HOLD: archive ownership missing"
        );
        let section = if matches!(tx.status, TxStatus::Completed) {
            "completed"
        } else {
            "transactions"
        };
        ensure!(
            state[section]
                .as_object_mut()
                .unwrap()
                .insert(uuid.to_string(), row)
                .is_none(),
            "HOLD: duplicate archive UUID"
        );
        head = entry["previous"].as_str().map(str::to_owned);
        remaining -= 1;
    }
    ensure!(remaining == 0, "HOLD: incomplete archive inventory");
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::{
        common::{EthereumSlotNumber, TxHashWithSlot},
        eth_to_gear::{
            storage::{tests::runtime_identity, Storage},
            tx_manager::TransactionManager,
        },
    };
    use std::sync::Arc;

    fn transaction(index: u64, status: TxStatus) -> Transaction {
        let mut hash = [0u8; 32];
        hash[..8].copy_from_slice(&index.to_le_bytes());
        let mut tx = Transaction::new(
            TxHashWithSlot {
                tx_hash: hash.into(),
                slot_number: EthereumSlotNumber(index + 1),
            },
            status,
        );
        if is_cold(&tx.status) {
            tx.receipt = Some(
                crate::message_relayer::eth_to_gear::tx_manager::ReceiptEvidence {
                    payload: vec![42; 128],
                    receipt_key: (index + 1, 0),
                    composed_at_ms: 1,
                    handed_off_at_ms: None,
                    initial_response: None,
                    signed_submission: None,
                    submission_attempts: Vec::new(),
                },
            );
        }
        tx
    }

    // Reconstructed from supplied HTML VB-10. This measures journal bounds, not network finality.
    #[tokio::test]
    async fn reconstructed_html_ten_thousand_cold_records_keep_hot_state_bounded() {
        let root = std::env::temp_dir().join(format!("inbound-schema8-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&root));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let mut identities = Vec::new();
        for index in 0..10_000 {
            let tx = transaction(
                index,
                TxStatus::Unprocessable {
                    encoded_envelope_bytes: 3_000_000,
                    required_gas: None,
                    approved_gas: Some(750_000_000_000),
                    diagnostic:
                        "full authenticated envelope exceeds runtime limit; liability retained"
                            .into(),
                },
            );
            if index == 0 || index == 9_999 {
                identities.push(tx.tx.clone());
            }
            manager.transactions.write().await.insert(tx.uuid, tx);
            if index % 128 == 127 || index == 9_999 {
                storage.save(&manager).await.unwrap();
                assert!(manager.transactions.read().await.is_empty());
                assert!(manager.completed.read().await.is_empty());
            }
        }
        let state = fs::read(root.join("state.json")).unwrap();
        assert!(
            state.len() < 4_096,
            "cold history must not amplify hot JSON"
        );
        let state: Value = serde_json::from_slice(&state).unwrap();
        assert_eq!(state["archive_count"], 10_000);
        for identity in identities {
            assert!(storage.cold_contains(&identity).await.unwrap());
        }
        let restarted = TransactionManager::new(storage.clone());
        storage.load(&restarted).await.unwrap();
        assert!(restarted.transactions.read().await.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    // Reconstructed from HTML VB-10: each crash boundary keeps its signer fence and original bytes.
    #[tokio::test]
    async fn reconstructed_html_interrupted_publication_never_authorizes_replacement() {
        for phase in 0..3 {
            let root = std::env::temp_dir().join(format!("inbound-crash-{}", Uuid::now_v7()));
            let storage = JSONStorage::new(&root);
            storage
                .bind_runtime_identity(runtime_identity())
                .await
                .unwrap();
            let original = fs::read(root.join("state.json")).unwrap();
            let before: Value = serde_json::from_slice(&original).unwrap();
            let mut incoming = decode_hot(&root, before.clone()).unwrap();
            let tx = transaction(
                7,
                TxStatus::Unprocessable {
                    encoded_envelope_bytes: 3_000_000,
                    required_gas: None,
                    approved_gas: None,
                    diagnostic: "retained liability".into(),
                },
            );
            incoming["transactions"][tx.uuid.to_string()] = serde_json::to_value(&tx).unwrap();
            begin_write(&root, Some(&before), &incoming).unwrap();
            if phase > 0 {
                let next = encode_state(&root, incoming, Some(&before)).await.unwrap();
                if phase == 2 {
                    fs::write(root.join("state.json"), serde_json::to_vec(&next).unwrap()).unwrap();
                }
            }
            let restarted = JSONStorage::new(&root);
            restarted
                .bind_runtime_identity(runtime_identity())
                .await
                .unwrap();
            assert!(!root.join("state.write-intent").exists());
            let retained = inventory(&root).unwrap();
            assert_eq!(retained["transactions"].as_object().unwrap().len(), 1);
            assert_eq!(retained["archive_count"], 1);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn reconstructed_html_missing_index_or_blob_is_hold_not_unseen() {
        let root = std::env::temp_dir().join(format!("inbound-missing-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&root));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let tx = transaction(
            9,
            TxStatus::Unprocessable {
                encoded_envelope_bytes: 9,
                required_gas: Some(11),
                approved_gas: Some(10),
                diagnostic: "over budget".into(),
            },
        );
        let identity = tx.tx.clone();
        manager.transactions.write().await.insert(tx.uuid, tx);
        storage.save(&manager).await.unwrap();
        let state: Value =
            serde_json::from_slice(&fs::read(root.join("state.json")).unwrap()).unwrap();
        let index = root
            .join("ownership")
            .join(state["ownership_root"].as_str().unwrap());
        fs::remove_file(index).unwrap();
        assert!(storage.cold_contains(&identity).await.is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn reconstructed_html_active_capacity_does_not_truncate_evidence() {
        let root = std::env::temp_dir().join(format!("inbound-capacity-{}", Uuid::now_v7()));
        let storage = Arc::new(JSONStorage::new(&root));
        storage
            .bind_runtime_identity(runtime_identity())
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone());
        let original = fs::read(root.join("state.json")).unwrap();
        for index in 0..=HOT_LIMIT {
            let tx = transaction(index as u64, TxStatus::ComposeProof);
            manager.transactions.write().await.insert(tx.uuid, tx);
        }
        assert!(storage.save(&manager).await.is_err());
        assert_eq!(manager.transactions.read().await.len(), HOT_LIMIT + 1);
        assert_eq!(fs::read(root.join("state.json")).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn reconstructed_html_truncated_intent_holds_and_partial_record_recovers_exact_bytes() {
        let root = std::env::temp_dir().join(format!("inbound-partial-{}", Uuid::now_v7()));
        fs::create_dir_all(&root).unwrap();
        let bytes = b"immutable original signed bytes";
        let pending = root.join(format!(".{}.pending", digest(bytes)));
        fs::write(&pending, &bytes[..8]).unwrap();
        immutable(&root.join("record"), bytes).unwrap();
        assert_eq!(fs::read(root.join("record")).unwrap(), bytes);
        assert_eq!(
            fs::read(root.join(format!(".{}.interrupted", digest(&bytes[..8])))).unwrap(),
            &bytes[..8]
        );
        fs::write(root.join("state.write-intent"), b"{truncated").unwrap();
        assert!(recover_write(&root)
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert_eq!(
            fs::read(root.join("state.write-intent")).unwrap(),
            b"{truncated"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
