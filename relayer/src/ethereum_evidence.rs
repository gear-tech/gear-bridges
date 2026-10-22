//! Read-only, serial NDJSON evidence owner. No signer, Gear authority or listener.
//!
//! Child archive digests are keccak256 of compact JSON with recursively sorted object
//! keys (arrays retain order). This defines legacy JSONB semantic identity without
//! claiming to preserve unavailable original whitespace. Native Beacon proof digests
//! retain their own SHA-256/SCALE identity unchanged.
use alloy::{
    consensus::{proofs::calculate_receipt_root, Header, ReceiptEnvelope},
    eips::Encodable2718,
    primitives::{keccak256, B256},
    providers::Provider,
    rpc::types::BlockNumberOrTag,
};
use anyhow::{ensure, Context, Result};
use ethereum_client::{
    beacon_finality::BeaconFinalityConfig,
    inclusion::{authenticated_receipt_block, transaction_proof, ReceiptBlockErrors, RecordedReceiptProof},
    FinalityCheckpoint, FinalizedView, PollingEthApi,
};
use gear_common::durable::{committed_bytes, entry_exists, immutable_json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::{BTreeMap, HashSet}, fs::{self, File}, path::PathBuf, time::Duration};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, clap::Args)]
pub struct Args {
    #[arg(long)]
    pub ethereum_endpoint: String,
    #[arg(long)]
    pub ethereum_finality_beacon_rpc: String,
    #[arg(long)]
    pub ethereum_finality_bootstrap: PathBuf,
    #[arg(long)]
    pub ethereum_finality_bootstrap_sha256: B256,
    #[arg(long)]
    pub ethereum_finality_store: PathBuf,
    /// Read-only peer Beacon archive directories (for indexer owners: STORE/beacon).
    #[arg(long)]
    pub ethereum_finality_proof_source: Vec<PathBuf>,
    /// Read-only peer child roots containing immutable receipt evidence.
    #[arg(long)]
    pub ethereum_evidence_proof_source: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(into = "String")]
struct Decimal(u64);
impl From<Decimal> for String { fn from(value: Decimal) -> Self { value.0.to_string() } }
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        let parsed = value.parse::<u64>().map_err(serde::de::Error::custom)?;
        if value != parsed.to_string() { return Err(serde::de::Error::custom("noncanonical decimal")); }
        Ok(Self(parsed))
    }
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(into = "String")]
struct Hash(B256);
impl From<Hash> for String { fn from(value: Hash) -> Self { format!("{:#x}", value.0) } }
impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        let parsed = value.parse::<B256>().map_err(serde::de::Error::custom)?;
        if value != format!("{parsed:#x}") { return Err(serde::de::Error::custom("noncanonical hash")); }
        Ok(Self(parsed))
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target { number: Decimal, hash: Hash }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Original { hash: Hash, number: Decimal, block_hash: Option<Hash>, proof_digest: Option<Hash> }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyCheckpoint {
    schema_version: u32, policy: String, chain_id: Decimal, genesis_hash: Hash,
    number: Decimal, timestamp: Decimal, hash: Hash, parent_hash: Hash,
    state_root: Hash, transactions_root: Hash, receipts_root: Hash,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Previous { Native(FinalityCheckpoint), Legacy(LegacyCheckpoint) }
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetainedLink {
    original_digest: Hash,
    proof_digest: Hash,
    checkpoint: FinalityCheckpoint,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Request {
    Qualify {
        id: String,
        #[serde(rename = "previousCheckpoint")]
        previous: Option<Previous>,
        #[serde(default, rename = "checkpointFences")]
        checkpoint_fences: Vec<Previous>,
        #[serde(default, rename = "retainedRequalificationLinks")]
        retained_links: Vec<RetainedLink>,
        targets: Vec<Target>,
    },
    Receipts { id: String, checkpoint: FinalityCheckpoint, originals: Vec<Original> },
}
impl Request {
    fn id(&self) -> &str { match self { Self::Qualify { id, .. } | Self::Receipts { id, .. } => id } }
}

// serde's generic Value silently accepts duplicate keys. Reject them at every depth
// before decoding untagged checkpoint variants or saving semantic original identity.
struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("JSON without duplicate object keys") }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> { serde_json::Number::from_f64(v).map(|v| UniqueJson(Value::Number(v))).ok_or_else(|| E::custom("invalid number")) }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_string<E: serde::de::Error>(self, v: String) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(v.into())) }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> { Ok(UniqueJson(Value::Null)) }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(v)) = a.next_element()? { values.push(v); }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> std::result::Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, UniqueJson(value))) = a.next_entry::<String, UniqueJson>()? {
                    if values.insert(key, value).is_some() { return Err(serde::de::Error::custom("duplicate JSON key")); }
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        d.deserialize_any(Visitor)
    }
}
fn valid_id(id: &str) -> bool { !id.is_empty() && id.trim() == id && !id.chars().any(char::is_control) }
fn parse_request(bytes: &[u8]) -> Result<Request> {
    let value = serde_json::from_slice::<UniqueJson>(bytes)?.0;
    // Native checkpoint bytes are opaque to JS. Reject fields or spellings that
    // the native deserializer would silently discard/normalize before echoing.
    for checkpoint in value.get("checkpoint").into_iter()
        .chain(value.get("previousCheckpoint"))
        .chain(value.get("checkpointFences").and_then(Value::as_array).into_iter().flatten())
        .chain(value.get("retainedRequalificationLinks").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|link| link.get("checkpoint")))
        .filter(|checkpoint| checkpoint.get("schema_version").is_some())
    {
        let parsed: FinalityCheckpoint = serde_json::from_value(checkpoint.clone())?;
        ensure!(serde_json::to_value(parsed)? == *checkpoint, "noncanonical native checkpoint");
    }
    let request: Request = serde_json::from_value(value)?;
    ensure!(valid_id(request.id()), "invalid request id");
    Ok(request)
}

#[derive(Debug, thiserror::Error)]
#[error("requested execution evidence is unavailable")]
struct Unavailable;
fn error_kind(error: &anyhow::Error) -> &'static str {
    if error.chain().any(|e| e.is::<Unavailable>() || e.is::<tokio::time::error::Elapsed>()
        || matches!(e.downcast_ref::<ethereum_client::Error>(), Some(ethereum_client::Error::FinalizedAncestryPending | ethereum_client::Error::FinalizedEvidenceUnavailable(_))))
        || ethereum_client::error::is_transient_rpc_error(error) { "unavailable" } else { "conflict" }
}
fn failure(id: &str, kind: &str) -> Value {
    // Never echo RPC error messages: they may contain credential-bearing URLs.
    json!({"id":id,"ok":false,"error":{"kind":kind,"message":match kind {
        "unavailable" => "Requested evidence is unavailable; HOLD",
        "protocol" => "Invalid serial evidence protocol; closing owner",
        _ => "Evidence identity or proof conflict; HOLD",
    }}})
}
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(values.into_iter().map(|(k,v)| (k,canonical(v))).collect::<BTreeMap<_,_>>().into_iter().collect()),
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        value => value,
    }
}
fn digest<T: Serialize>(value: &T) -> Result<B256> { Ok(keccak256(serde_json::to_vec(&canonical(serde_json::to_value(value)?))?)) }
fn insert_fence(fences: &mut BTreeMap<u64, B256>, number: u64, hash: B256) -> Result<()> {
    ensure!(fences.insert(number, hash).is_none_or(|old| old == hash), "conflicting header fence");
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedReceipt {
    inclusion: RecordedReceiptProof,
    // Full envelopes authenticate the selected receipt's global log-index prefix.
    receipt_context: B256,
}

pub struct EvidenceOwner {
    api: PollingEthApi,
    directory: PathBuf,
    _owner: File,
    proof_sources: Vec<PathBuf>,
}
impl EvidenceOwner {
    pub async fn open(args: Args) -> Result<Self> {
        let config = BeaconFinalityConfig::load(&args.ethereum_finality_bootstrap,
            args.ethereum_finality_bootstrap_sha256, &args.ethereum_finality_beacon_rpc)?
            .with_proof_sources(args.ethereum_finality_proof_source)?;
        Self::connect(&args.ethereum_endpoint, config, args.ethereum_finality_store, args.ethereum_evidence_proof_source).await
    }
    async fn connect(endpoint: &str, config: BeaconFinalityConfig, directory: PathBuf, proof_sources: Vec<PathBuf>) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        let directory = fs::canonicalize(directory)?;
        let mut proof_sources = proof_sources.into_iter().map(|source| -> Result<PathBuf> {
            ensure!(source.is_absolute(), "evidence proof source must be an absolute directory");
            let source = fs::canonicalize(source)?;
            ensure!(source.is_dir(), "evidence proof source must be a directory");
            Ok(source)
        }).collect::<Result<Vec<_>>>()?;
        proof_sources.sort();
        proof_sources.dedup();
        proof_sources.retain(|source| source != &directory);
        let owner = File::options().read(true).write(true).create(true).truncate(false).open(directory.join("evidence-owner.lock"))?;
        owner.try_lock().context("evidence directory already owned")?;
        let api = PollingEthApi::new(endpoint).await?
            .with_beacon_finality(config, directory.join("beacon")).await?;
        Ok(Self { api, directory, _owner: owner, proof_sources })
    }
    fn record<T: Serialize>(&self, value: &T) -> Result<B256> {
        let value = canonical(serde_json::to_value(value)?);
        let hash = digest(&value)?;
        immutable_json(&self.directory.join(format!("evidence-{hash:x}.json")), &value)?;
        Ok(hash)
    }
    fn try_load<T: serde::de::DeserializeOwned>(&self, hash: B256) -> Result<Option<T>> {
        let mut found = None;
        for directory in std::iter::once(&self.directory).chain(&self.proof_sources) {
            let path = directory.join(format!("evidence-{hash:x}.json"));
            if !entry_exists(&path)? && !entry_exists(&path.with_extension("json.new"))? { continue; }
            let value = serde_json::from_slice::<UniqueJson>(&committed_bytes(&path)?)?.0;
            ensure!(digest(&value)? == hash, "retained evidence digest mismatch");
            found = Some(value);
        }
        found.map(serde_json::from_value).transpose().map_err(Into::into)
    }
    fn load<T: serde::de::DeserializeOwned>(&self, hash: B256) -> Result<T> {
        self.try_load(hash)?.ok_or_else(|| Unavailable.into())
    }
    fn original_digest(&self, original: B256) -> Result<Option<B256>> {
        let mut found = None;
        for directory in std::iter::once(&self.directory).chain(&self.proof_sources) {
            let path = directory.join(format!("original-{original:x}.json"));
            if !entry_exists(&path)? && !entry_exists(&path.with_extension("json.new"))? { continue; }
            let hash: B256 = serde_json::from_slice(&committed_bytes(&path)?)?;
            ensure!(found.is_none_or(|old| old == hash), "conflicting retained original proof identities");
            found = Some(hash);
        }
        Ok(found)
    }
    async fn header(&self, number: u64, hash: B256) -> Result<Header> {
        let path = self.directory.join(format!("header-{hash:x}.json"));
        let header: Header = if entry_exists(&path)? || entry_exists(&path.with_extension("json.new"))? {
            serde_json::from_slice(&committed_bytes(&path)?)?
        } else {
            tokio::time::timeout(REQUEST_TIMEOUT, self.api.get_block_by_hash(hash)).await??
                .ok_or(Unavailable)?.header.inner
        };
        ensure!(header.number == number && header.hash_slow() == hash, "header RLP identity mismatch");
        immutable_json(&path, &header)?;
        Ok(header)
    }
    async fn qualify(&self, previous: Option<Previous>, mut checkpoints: Vec<Previous>, retained_links: Vec<RetainedLink>, targets: Vec<Target>) -> Result<Value> {
        let mut view = self.api.verified_finalized_view().await?;
        let mut fences = BTreeMap::new();
        for target in targets { insert_fence(&mut fences, target.number.0, target.hash.0)?; }
        insert_fence(&mut fences, view.block_number(), view.block_hash())?;
        checkpoints.extend(previous);
        let mut native = Vec::new();
        let mut legacy = BTreeMap::new();
        for checkpoint in checkpoints {
            match checkpoint {
                Previous::Native(checkpoint) => {
                    ensure!(checkpoint.schema_version() == 2, "only Beacon native checkpoints accepted");
                    insert_fence(&mut fences, checkpoint.common_header().number, checkpoint.common_header().hash_slow())?;
                    native.push(checkpoint);
                }
                Previous::Legacy(old) => {
                    ensure!(old.schema_version == 1 && old.policy == "independent-two-el-v1"
                        && old.chain_id.0 == view.checkpoint().chain_id() && old.genesis_hash.0 == view.genesis_hash(), "legacy checkpoint identity mismatch");
                    insert_fence(&mut fences, old.number.0, old.hash.0)?;
                    legacy.insert(digest(&old)?, old);
                }
            }
        }
        for link in &retained_links {
            let old = legacy.get(&link.original_digest.0).context("retained link original was not supplied")?;
            ensure!(link.checkpoint.schema_version() == 2
                && old.number.0 <= link.checkpoint.common_header().number, "retained link checkpoint does not cover original");
            insert_fence(&mut fences, link.checkpoint.common_header().number, link.checkpoint.common_header().hash_slow())?;
            native.push(link.checkpoint.clone());
        }
        view.verify_checkpoints_with_blocks(&*self.api, &native, &fences).await?;
        let mut authenticated = BTreeMap::new();
        for (number, hash) in fences {
            let header = self.header(number, hash).await?;
            let proof_digest = self.record(&json!({"schemaVersion":1,"kind":"header","checkpoint":view.checkpoint(),"header":header}))?;
            authenticated.insert(number, (header, proof_digest));
        }
        let mut links = Vec::with_capacity(legacy.len());
        for (original_digest, old) in &legacy {
            let (header, proof_digest) = authenticated.get(&old.number.0).context("legacy header absent")?;
            ensure!(old.hash.0 == header.hash_slow() && old.timestamp.0 == header.timestamp && old.parent_hash.0 == header.parent_hash
                && old.state_root.0 == header.state_root && old.transactions_root.0 == header.transactions_root
                && old.receipts_root.0 == header.receipts_root, "legacy header fields mismatch");
            ensure!(self.record(old)? == *original_digest, "legacy original identity changed");
            let link = json!({"originalDigest":original_digest,"proofDigest":proof_digest});
            immutable_json(&self.directory.join(format!("requalification-{original_digest:x}-{proof_digest:x}.json")), &link)?;
            links.push(link);
        }
        // Validate only the explicitly requested historical pairs, not the
        // lifetime native-checkpoint × legacy-original cross product.
        for link in retained_links {
            let old = &legacy[&link.original_digest.0];
            let (header, _) = &authenticated[&old.number.0];
            let record = json!({"schemaVersion":1,"kind":"header","checkpoint":link.checkpoint,"header":header});
            ensure!(digest(&record)? == link.proof_digest.0, "retained requalification proof identity changed");
            self.record(&record)?;
            let retained = json!({"originalDigest":link.original_digest,"proofDigest":link.proof_digest});
            immutable_json(&self.directory.join(format!("requalification-{:x}-{:x}.json",link.original_digest.0,link.proof_digest.0)), &retained)?;
        }
        let headers = authenticated.values().map(|(header, proof)| header_evidence(header, *proof)).collect::<Vec<_>>();
        view.recheck(&*self.api).await?;
        Ok(json!({"ok":true,"op":"qualify","checkpoint":view.checkpoint(),"headers":headers,"requalificationLinks":links}))
    }
    async fn receipts(&self, checkpoint: FinalityCheckpoint, originals: Vec<Original>) -> Result<Value> {
        ensure!(checkpoint.schema_version() == 2, "receipt authority requires Beacon checkpoint");
        let mut view = self.api.verified_finalized_view().await?;
        view.pin_to_checkpoint(&*self.api, &checkpoint).await?;
        let mut fences = BTreeMap::new();
        let mut selected = Vec::with_capacity(originals.len());
        let mut seen = HashSet::new();
        for mut original in originals {
            ensure!(seen.insert(original.hash.0), "duplicate original");
            let expected = match original.proof_digest {
                Some(hash) => Some(hash.0),
                None => self.original_digest(original.hash.0)?,
            };
            original.proof_digest = expected.map(Hash);
            let saved = match expected {
                Some(hash) => self.try_load::<SavedReceipt>(hash)?.map(|saved| (hash, saved)),
                None => None,
            };
            let hash = match (&saved, original.block_hash) {
                (Some((_, saved)), _) => saved.inclusion.header.hash_slow(),
                (None, Some(hash)) => hash.0,
                (None, None) => {
                    let block = tokio::time::timeout(REQUEST_TIMEOUT, self.api.get_block_by_number(BlockNumberOrTag::Number(original.number.0))).await??.ok_or(Unavailable)?;
                    ensure!(block.header.number == original.number.0 && block.header.hash == block.header.inner.hash_slow(), "receipt block hint is malformed");
                    block.header.hash
                }
            };
            ensure!(original.block_hash.is_none_or(|expected| expected.0 == hash), "original block hash changed");
            insert_fence(&mut fences, original.number.0, hash)?;
            selected.push((original, hash, saved));
        }
        view.verify_blocks(&*self.api, &fences).await?;
        let mut receipts = Vec::with_capacity(selected.len());
        for (original, hash, saved) in selected {
            let (proof_digest, saved) = match saved {
                Some(saved) => saved,
                None => self.acquire_receipt(&view, &original, hash).await?,
            };
            ensure!(original.proof_digest.is_none_or(|expected| expected.0 == proof_digest), "original inclusion proof identity changed");
            let proof = &saved.inclusion;
            ensure!(proof.header.number == original.number.0 && proof.header.hash_slow() == hash, "retained original coordinates changed");
            proof.verify_inclusion(&view, original.hash.0)?;
            let context: Vec<ReceiptEnvelope> = self.load(saved.receipt_context)?;
            let receipt = receipt_json(proof, original.hash.0, &context)?;
            // Copy only fully authenticated immutable objects into our own store;
            // no peer cache, ownership lock, directory scan or state write is used.
            ensure!(self.record(&context)? == saved.receipt_context && self.record(&saved)? == proof_digest, "retained proof encoding changed");
            immutable_json(&self.directory.join(format!("original-{:x}.json",original.hash.0)), &proof_digest)?;
            receipts.push(json!({"hash":original.hash,"number":original.number,"blockHash":hash,
                "receipt":receipt,"blockTimestamp":proof.header.timestamp.to_string(),"proofDigest":proof_digest}));
        }
        view.recheck(&*self.api).await?;
        Ok(json!({"ok":true,"op":"receipts","checkpoint":checkpoint,"receipts":receipts}))
    }
    async fn acquire_receipt(&self, view: &FinalizedView, original: &Original, hash: B256) -> Result<(B256, SavedReceipt)> {
        let (header, transactions, envelopes) = tokio::time::timeout(REQUEST_TIMEOUT,
            authenticated_receipt_block(&*self.api, original.number.0, hash, ReceiptBlockErrors {
                block: "full block unavailable", receipts: "receipts unavailable", header: "header mismatch",
                transactions: "full transactions unavailable", transaction_root: Some("transaction root mismatch"),
                count: "incomplete receipt set", coordinates: "receipt coordinate mismatch", root: "receipt root mismatch",
            })).await??;
        let index = transactions.iter().position(|tx| keccak256(tx.encoded_2718()) == original.hash.0).ok_or(Unavailable)? as u64;
        let receipt = ethereum_common::utils::generate_merkle_proof(index, &envelopes)?;
        let proof = RecordedReceiptProof {
            genesis: view.genesis_hash(), transaction_proof: transaction_proof(&transactions, index, header.transactions_root)?,
            header, index, raw_transaction: transactions[index as usize].encoded_2718(),
            receipt: receipt.receipt, receipt_proof: receipt.proof,
        };
        proof.verify_inclusion(view, original.hash.0)?;
        let context = envelopes.into_iter().map(|(_, receipt)| receipt).collect::<Vec<_>>();
        let saved = SavedReceipt { inclusion: proof, receipt_context: self.record(&context)? };
        let proof_digest = digest(&saved)?;
        ensure!(original.proof_digest.is_none_or(|expected| expected.0 == proof_digest), "regenerated inclusion proof identity changed");
        self.record(&saved)?;
        immutable_json(&self.directory.join(format!("original-{:x}.json", original.hash.0)), &proof_digest)?;
        Ok((proof_digest, saved))
    }
    async fn execute(&self, request: Request) -> Value {
        let id = request.id().to_owned();
        let result = match request {
            Request::Qualify { previous, checkpoint_fences, retained_links, targets, .. } => self.qualify(previous, checkpoint_fences, retained_links, targets).await,
            Request::Receipts { checkpoint, originals, .. } => self.receipts(checkpoint, originals).await,
        };
        match result {
            Ok(mut response) => { response["id"] = id.into(); response }
            Err(error) => failure(&id, error_kind(&error)),
        }
    }
}
fn header_evidence(header: &Header, proof: B256) -> Value {
    json!({"number":header.number.to_string(),"timestamp":header.timestamp.to_string(),"hash":header.hash_slow(),
        "parentHash":header.parent_hash,"stateRoot":header.state_root,"transactionsRoot":header.transactions_root,
        "receiptsRoot":header.receipts_root,"proofDigest":proof})
}
fn receipt_json(proof: &RecordedReceiptProof, hash: B256, context: &[ReceiptEnvelope]) -> Result<Value> {
    ensure!(calculate_receipt_root(context) == proof.header.receipts_root, "receipt prefix root mismatch");
    let index = usize::try_from(proof.index)?;
    ensure!(context.get(index).is_some_and(|receipt| receipt.encoded_2718() == proof.receipt.encoded_2718()), "selected receipt differs from prefix context");
    let first = context[..index].iter().try_fold(0u64, |sum, receipt| sum.checked_add(receipt.logs().len() as u64).context("global log index overflow"))?;
    let block_hash = proof.header.hash_slow();
    // Consensus serialization supplies status/type/bloom/cumulativeGasUsed only;
    // all RPC coordinates below are derived from authenticated inclusion and prefix.
    let mut receipt = serde_json::to_value(&proof.receipt)?;
    ensure!(receipt.get("status").is_some(), "pre-Byzantium receipt cannot authorize bridge completion");
    receipt["transactionHash"] = json!(hash);
    receipt["blockHash"] = json!(block_hash);
    receipt["blockNumber"] = json!(format!("{:#x}", proof.header.number));
    receipt["transactionIndex"] = json!(format!("{:#x}", proof.index));
    let logs = proof.receipt.logs().iter().enumerate().map(|(offset, log)| -> Result<Value> {
        let log_index = first.checked_add(u64::try_from(offset)?).context("global log index overflow")?;
        Ok(json!({"address":log.address,"topics":log.topics(),"data":log.data.data,
            "logIndex":format!("{log_index:#x}"),"transactionHash":hash,"blockHash":block_hash,
            "blockNumber":format!("{:#x}",proof.header.number),"transactionIndex":format!("{:#x}",proof.index),"removed":false}))
    }).collect::<Result<Vec<_>>>()?;
    receipt["logs"] = json!(logs);
    Ok(receipt)
}

async fn frame<R: AsyncBufRead + Unpin>(input: &mut R) -> Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    loop {
        let buffer = input.fill_buf().await?;
        if buffer.is_empty() { ensure!(frame.is_empty(), "unterminated NDJSON frame"); return Ok(None); }
        let end = buffer.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(buffer.len(), |end| end + 1);
        ensure!(frame.len() + count <= MAX_FRAME, "NDJSON frame exceeds limit");
        frame.extend_from_slice(&buffer[..count]);
        input.consume(count);
        if end.is_some() { frame.pop(); return Ok(Some(frame)); }
    }
}
async fn send<W: AsyncWrite + Unpin>(output: &mut W, response: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(response)?;
    ensure!(bytes.len() < MAX_FRAME, "response exceeds NDJSON frame limit");
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await?;
    Ok(())
}
async fn serve<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(owner: &EvidenceOwner, mut input: R, mut output: W) -> Result<()> {
    let mut ids = HashSet::new();
    while let Some(bytes) = frame(&mut input).await? {
        let request = match parse_request(&bytes) {
            Ok(request) => request,
            Err(_) => {
                if let Ok(UniqueJson(value)) = serde_json::from_slice(&bytes) {
                    if let Some(id) = value.get("id").and_then(Value::as_str).filter(|id| valid_id(id)) {
                        send(&mut output, &failure(id, "protocol")).await?;
                    }
                }
                anyhow::bail!("invalid evidence protocol; closing owner");
            }
        };
        if !ids.insert(request.id().to_owned()) {
            send(&mut output, &failure(request.id(), "protocol")).await?;
            anyhow::bail!("duplicate evidence request id; closing owner");
        }
        send(&mut output, &owner.execute(request).await).await?;
    }
    Ok(())
}

pub async fn run(args: Args) -> Result<()> {
    // Startup errors are deliberately redacted too; logs must not reveal RPC secrets.
    let owner = EvidenceOwner::open(args).await.map_err(|_| anyhow::anyhow!("Ethereum evidence owner initialization failed; HOLD"))?;
    serve(&owner, tokio::io::BufReader::new(tokio::io::stdin()), tokio::io::stdout()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{proofs::calculate_transaction_root, Receipt, SignableTransaction, TxEip1559, TxEnvelope},
        network::TxSigner,
        primitives::{Address, TxKind},
        rpc::types::{Block, BlockTransactions, TransactionReceipt},
        signers::local::PrivateKeySigner,
    };
    use axum::{extract::State, routing::post, Json, Router};
    use ethereum_client::beacon_finality::test_fixtures::LocalBeaconFixture;
    use std::sync::{Arc, Mutex};

    #[test]
    fn legacy_jsonb_semantic_identity_matches_indexer_vector() -> Result<()> {
        let original = json!({
            "schemaVersion":1,"policy":"independent-two-el-v1","chainId":"31337",
            "genesisHash":format!("{:#x}",B256::repeat_byte(0x11)),
            "number":"42","timestamp":"1700000000",
            "hash":format!("{:#x}",B256::repeat_byte(0x22)),
            "parentHash":format!("{:#x}",B256::repeat_byte(0x33)),
            "stateRoot":format!("{:#x}",B256::repeat_byte(0x44)),
            "transactionsRoot":format!("{:#x}",B256::repeat_byte(0x55)),
            "receiptsRoot":format!("{:#x}",B256::repeat_byte(0x66))
        });
        let expected: B256 = "0xb21f8e37958dbd35993aca06421d55228ad6ebb68858172fbc782af43805097d".parse()?;
        assert_eq!(digest(&original)?,expected);
        let decoded: LegacyCheckpoint = serde_json::from_value(original.clone())?;
        assert_eq!(digest(&decoded)?,expected);
        let reversed: serde_json::Map<String,Value> = original.as_object().unwrap().iter().rev().map(|(key,value)| (key.clone(),value.clone())).collect();
        assert_eq!(digest(&reversed)?,expected);
        let mut changed = original;
        changed["stateRoot"] = json!(B256::repeat_byte(0x45));
        assert_ne!(digest(&changed)?,expected);
        Ok(())
    }

    #[test]
    fn strict_protocol_rejects_ambiguous_authority_and_coordinates() {
        let hash = format!("{:#x}", B256::repeat_byte(0xab));
        let good = json!({"op":"qualify","id":"1","targets":[{"number":"0","hash":hash}]});
        assert!(parse_request(&serde_json::to_vec(&good).unwrap()).is_ok());
        for bad in [
            r#"{"op":"qualify","id":"1","id":"2","targets":[]}"#.to_owned(),
            r#"{"op":"qualify","id":"","targets":[]}"#.to_owned(),
            r#"{"op":"qualify","id":"1","targets":[],"signer":"secret"}"#.to_owned(),
            r#"{"op":"send","id":"1","targets":[]}"#.to_owned(),
            good.to_string().replace("\"0\"", "\"00\""),
            good.to_string().replace(&hash, &hash.to_uppercase()),
            good.to_string().replace("\"number\":\"0\"", "\"number\":0"),
            r#"{"op":"qualify","id":"1","targets":[],"checkpointFences":[{"a":1,"a":2}]}"#.to_owned(),
        ] { assert!(parse_request(bad.as_bytes()).is_err(), "{bad}"); }
        assert_eq!(digest(&json!({"b":[{"z":2,"a":1}],"a":0})).unwrap(),
            digest(&json!({"a":0,"b":[{"a":1,"z":2}]})).unwrap());
        assert_eq!(error_kind(&Unavailable.into()), "unavailable");
        assert_eq!(error_kind(&anyhow::anyhow!("timeout in malicious content")), "conflict");
        assert!(!failure("1", "conflict").to_string().contains("secret"));
    }

    #[tokio::test]
    async fn bounded_frames_never_accept_partial_or_oversized_json() {
        assert!(frame(&mut &b"{\"op\":\"qualify\"}"[..]).await.is_err());
        let over = vec![b'x'; MAX_FRAME + 1];
        assert!(frame(&mut over.as_slice()).await.is_err());
        assert!(frame(&mut &b""[..]).await.unwrap().is_none());
        let mut two = &b"{}\n{}\n"[..];
        assert_eq!(frame(&mut two).await.unwrap().unwrap(), b"{}");
        assert_eq!(frame(&mut two).await.unwrap().unwrap(), b"{}");
    }

    struct Execution {
        blocks: Vec<Block>,
        receipts: Vec<TransactionReceipt>,
        tip: usize,
        bodies_available: bool,
    }
    struct Fixture {
        state: Arc<Mutex<Execution>>,
        url: String,
        server: tokio::task::JoinHandle<()>,
        beacon: LocalBeaconFixture,
    }
    impl Drop for Fixture { fn drop(&mut self) { self.server.abort(); } }
    impl Fixture {
        async fn new() -> Result<Self> {
            let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(62))?;
            let mut blocks: Vec<Block> = vec![Block::default(), Block::default(), Block::default()];
            blocks[0].header.inner.extra_data = vec![93].into();
            blocks[0].header.hash = blocks[0].header.inner.hash_slow();
            blocks[1].header.inner.parent_hash = blocks[0].header.hash;
            blocks[1].header.inner.number = 1;
            blocks[1].header.inner.timestamp = 1_700_000_012;
            let mut transactions = Vec::new();
            let mut envelopes = Vec::new();
            for nonce in 0..2 {
                let mut tx = TxEip1559 { chain_id:31337, nonce, gas_limit:100_000, max_fee_per_gas:10,
                    max_priority_fee_per_gas:1, to:TxKind::Call(Address::repeat_byte(9)), ..Default::default() };
                let signature = signer.sign_transaction(&mut tx).await?;
                let tx: TxEnvelope = tx.into_signed(signature).into();
                transactions.push(tx);
                let logs = (0..=nonce).map(|_| serde_json::from_value(json!({
                    "address":Address::repeat_byte(9),"topics":[B256::repeat_byte(42)],"data":"0x0102"
                }))).collect::<std::result::Result<Vec<_>, _>>()?;
                envelopes.push(ReceiptEnvelope::Eip1559(Receipt {
                    status:true.into(), cumulative_gas_used:21_000*(nonce+1), logs,
                }.with_bloom()));
            }
            blocks[1].header.inner.transactions_root = calculate_transaction_root(&transactions);
            blocks[1].header.inner.receipts_root = calculate_receipt_root(&envelopes);
            blocks[1].header.hash = blocks[1].header.inner.hash_slow();
            let mut rpc_transactions = Vec::new();
            let mut receipts = Vec::new();
            let mut global = 0u64;
            for (index, (tx, envelope)) in transactions.into_iter().zip(envelopes).enumerate() {
                let coordinates = json!({"transactionHash":keccak256(tx.encoded_2718()),"transactionIndex":format!("{index:#x}"),
                    "blockNumber":"0x1","blockHash":blocks[1].header.hash});
                let mut receipt = serde_json::to_value(envelope)?;
                receipt.as_object_mut().unwrap().extend(coordinates.as_object().unwrap().clone());
                receipt.as_object_mut().unwrap().extend(json!({"gasUsed":"0x5208","effectiveGasPrice":"0x1",
                    "from":signer.address(),"to":Address::repeat_byte(9),"contractAddress":null}).as_object().unwrap().clone());
                for log in receipt["logs"].as_array_mut().unwrap() {
                    log.as_object_mut().unwrap().extend(coordinates.as_object().unwrap().clone());
                    log["logIndex"] = json!(format!("{global:#x}"));
                    log["removed"] = json!(false);
                    global += 1;
                }
                receipts.push(serde_json::from_value(receipt)?);
                let mut transaction = serde_json::to_value(tx)?;
                transaction.as_object_mut().unwrap().extend(coordinates.as_object().unwrap().clone());
                transaction["hash"] = coordinates["transactionHash"].clone();
                transaction.as_object_mut().unwrap().remove("transactionHash");
                transaction["from"] = json!(signer.address());
                rpc_transactions.push(serde_json::from_value(transaction)?);
            }
            blocks[1].transactions = BlockTransactions::Full(rpc_transactions);
            blocks[2].header.inner.number = 2;
            blocks[2].header.inner.parent_hash = blocks[1].header.hash;
            blocks[2].header.inner.timestamp = 1_700_000_024;
            blocks[2].header.hash = blocks[2].header.inner.hash_slow();
            let state = Arc::new(Mutex::new(Execution { blocks, receipts, tip:1, bodies_available:true }));
            async fn rpc(State(state): State<Arc<Mutex<Execution>>>, Json(request): Json<Value>) -> Json<Value> {
                let state = state.lock().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x7a69"),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let number = if tag == "latest" || tag == "finalized" { state.tip } else { usize::from_str_radix(tag.trim_start_matches("0x"),16).unwrap() };
                        serde_json::to_value(state.blocks.get(number)).unwrap()
                    }
                    "eth_getBlockByHash" => {
                        if request["params"][1] == true && !state.bodies_available { Value::Null }
                        else { serde_json::to_value(state.blocks.iter().find(|b| json!(b.header.hash) == request["params"][0])).unwrap() }
                    }
                    "eth_getBlockReceipts" => if state.bodies_available { json!(state.receipts) } else { Value::Null },
                    _ => return Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"unsupported fixture method"}})),
                };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}",listener.local_addr()?);
            let app = Router::new().route("/",post(rpc)).with_state(state.clone());
            let server = tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); });
            let shared = state.clone();
            let genesis = state.lock().unwrap().blocks[0].header.hash;
            let beacon = LocalBeaconFixture::new(31337,genesis,Arc::new(move || {
                let state = shared.lock().unwrap();
                state.blocks[state.tip].header.clone()
            })).await?;
            Ok(Self { state,url,server,beacon })
        }
        async fn owner(&self) -> Result<EvidenceOwner> {
            EvidenceOwner::connect(&self.url,self.beacon.config.clone(),self.beacon.directory.join("child"),Vec::new()).await
        }
        fn request(&self, checkpoint: &Value, hash: B256, id: &str) -> Result<Request> {
            parse_request(&serde_json::to_vec(&json!({"op":"receipts","id":id,"checkpoint":checkpoint,
                "originals":[{"hash":hash,"number":"1","blockHash":self.state.lock().unwrap().blocks[1].header.hash}]}))?)
        }
    }

    // Large real Beacon SSZ values follow the native verifier's existing stack convention.
    #[test]
    fn signed_child_fixed_pins_restart_prefix_proofs_and_protocol_fail_closed() -> Result<()> {
        std::thread::Builder::new().stack_size(32*1024*1024).spawn(|| {
            tokio::runtime::Runtime::new().unwrap().block_on(signed_child_scenario())
        })?.join().unwrap()
    }
    async fn signed_child_scenario() -> Result<()> {
        let fixture = Fixture::new().await?;
        let owner = fixture.owner().await?;
        assert!(fixture.owner().await.is_err(), "same evidence directory must have one owner");
        let (hash, block) = {
            let state = fixture.state.lock().unwrap();
            (state.receipts[1].transaction_hash,state.blocks[1].header.clone())
        };
        let qualified = owner.execute(parse_request(br#"{"op":"qualify","id":"q","targets":[]}"#)?).await;
        assert_eq!(qualified["ok"],true,"{qualified}");
        let checkpoint = &qualified["checkpoint"];
        assert!(checkpoint["common_header"].get("hash").is_none(),
            "consensus checkpoint bytes must not acquire an RPC-only hash field");
        let head = qualified["headers"].as_array().unwrap().iter().find(|header| header["number"] == "1").expect("qualified headers include current head");
        assert_eq!(head["hash"],json!(block.inner.hash_slow()));
        assert_eq!(head["timestamp"],block.timestamp.to_string());
        assert_eq!(head["parentHash"],json!(block.parent_hash));
        assert_eq!(head["stateRoot"],json!(block.state_root));
        assert_eq!(head["transactionsRoot"],json!(block.transactions_root));
        assert_eq!(head["receiptsRoot"],json!(block.receipts_root));
        let head_proof: B256 = serde_json::from_value(head["proofDigest"].clone())?;
        let retained_head: Value = owner.load(head_proof)?;
        assert_eq!(retained_head["checkpoint"],*checkpoint);
        assert_eq!(retained_head["header"],checkpoint["common_header"]);
        let response = owner.execute(fixture.request(checkpoint,hash,"r")?).await;
        assert_eq!(response["ok"],true,"{response}");
        assert_eq!(response["checkpoint"],*checkpoint);
        assert_eq!(response["receipts"][0]["receipt"]["transactionIndex"],"0x1");
        assert_eq!(response["receipts"][0]["receipt"]["logs"][0]["logIndex"],"0x1");
        assert_eq!(response["receipts"][0]["receipt"]["logs"][1]["logIndex"],"0x2");
        // Separate writer stores share only explicit immutable read-only proof sources.
        let owner_fence_path = owner.directory.join("beacon/headers/fence.json");
        let owner_fence = fs::read(&owner_fence_path)?;
        let peer_config = fixture.beacon.config.clone().with_proof_sources(vec![owner.directory.join("beacon")])?;
        let peer = EvidenceOwner::connect(&fixture.url, peer_config, fixture.beacon.directory.join("peer"),Vec::new()).await?;
        let peer_receipt = peer.execute(fixture.request(checkpoint,hash,"peer")?).await;
        assert_eq!(peer_receipt["ok"],true,"{peer_receipt}");
        assert_eq!(peer_receipt["checkpoint"],*checkpoint);
        assert_eq!(peer_receipt["receipts"][0]["proofDigest"],response["receipts"][0]["proofDigest"]);
        drop(peer);
        assert_eq!(fs::read(&owner_fence_path)?, owner_fence,
            "a peer must not replace another owner's header fence with its own proof identity");
        let expected: B256 = serde_json::from_value(response["receipts"][0]["proofDigest"].clone())?;
        let mut exact = fixture.request(checkpoint,hash,"peer-retained")?;
        if let Request::Receipts { originals, .. } = &mut exact {
            originals[0].proof_digest = Some(Hash(expected));
        }
        fixture.state.lock().unwrap().bodies_available = false;
        let recovery_config = fixture.beacon.config.clone().with_proof_sources(vec![owner.directory.join("beacon")])?;
        let recovery = EvidenceOwner::connect(&fixture.url,recovery_config,fixture.beacon.directory.join("peer-recovery"),vec![owner.directory.clone()]).await?;
        let Request::Receipts { checkpoint: fixed, originals, .. } = exact else { unreachable!() };
        let recovered = recovery.receipts(fixed, originals).await.context("peer retained proof recovery")?;
        assert_eq!(recovered["ok"],true,"{recovered}");
        assert_eq!(recovered["receipts"],response["receipts"],"peer recovery must use exact retained MPT and prefix, with RPC bodies unavailable");
        drop(recovery);
        fixture.state.lock().unwrap().bodies_available = true;
        let mut wrong_identity = fixture.request(checkpoint,hash,"wrong-proof-digest")?;
        if let Request::Receipts { originals, .. } = &mut wrong_identity {
            originals[0].proof_digest = Some(Hash(B256::repeat_byte(85)));
        }
        let rejected = owner.execute(wrong_identity).await;
        assert_eq!(rejected["error"]["kind"],"conflict","valid regenerated proof cannot replace requested identity");
        let old = json!({"schemaVersion":1,"policy":"independent-two-el-v1","chainId":"31337",
            "genesisHash":fixture.beacon.config.identity().1,"number":"1","timestamp":block.timestamp.to_string(),
            "hash":block.hash,"parentHash":block.parent_hash,"stateRoot":block.state_root,
            "transactionsRoot":block.transactions_root,"receiptsRoot":block.receipts_root});
        let linked = owner.execute(parse_request(&serde_json::to_vec(&json!({
            "op":"qualify","id":"legacy","previousCheckpoint":checkpoint,"checkpointFences":[old],"targets":[]
        }))?)?).await;
        assert_eq!(linked["ok"],true,"{linked}");
        assert_eq!(linked["requalificationLinks"].as_array().unwrap().len(),1);
        let mut forged = old.clone();
        forged["stateRoot"] = json!(B256::repeat_byte(77));
        let rejected = owner.execute(parse_request(&serde_json::to_vec(&json!({
            "op":"qualify","id":"forged","checkpointFences":[forged],"targets":[]
        }))?)?).await;
        assert_eq!(rejected["error"]["kind"],"conflict");
        let absent = owner.execute(fixture.request(checkpoint,B256::repeat_byte(81),"absent")?).await;
        assert_eq!(absent["error"]["kind"],"unavailable");
        fixture.state.lock().unwrap().tip = 2;
        drop(owner);
        let owner = fixture.owner().await?;
        fixture.state.lock().unwrap().bodies_available = false;
        let reloaded = owner.execute(fixture.request(checkpoint,hash,"restart")?).await;
        assert_eq!(reloaded["ok"],true,"{reloaded}");
        assert_eq!(reloaded["checkpoint"],*checkpoint, "must not replace fixed original checkpoint with tip2");
        assert_eq!(reloaded["receipts"],response["receipts"]);
        let original_link = linked["requalificationLinks"][0].clone();
        let retained = json!({"originalDigest":original_link["originalDigest"],
            "proofDigest":original_link["proofDigest"],"checkpoint":linked["checkpoint"]});
        let request = json!({"op":"qualify","id":"legacy-restart","checkpointFences":[old],
            "retainedRequalificationLinks":[retained],"targets":[]});
        let linked_again = owner.execute(parse_request(&serde_json::to_vec(&request)?)?).await;
        assert_eq!(linked_again["ok"],true,"{linked_again}");
        assert_eq!(linked_again["requalificationLinks"].as_array().unwrap().len(),1);
        let mut forged_pair = request;
        forged_pair["retainedRequalificationLinks"][0]["proofDigest"] = json!(B256::repeat_byte(84));
        let rejected = owner.execute(parse_request(&serde_json::to_vec(&forged_pair)?)?).await;
        assert_eq!(rejected["error"]["kind"],"conflict");
        forged_pair["checkpointFences"] = json!([]);
        let rejected = owner.execute(parse_request(&serde_json::to_vec(&forged_pair)?)?).await;
        assert_eq!(rejected["error"]["kind"],"conflict","link without original must fail");
        let unavailable = owner.execute(fixture.request(checkpoint,B256::repeat_byte(82),"unavailable")?).await;
        assert_eq!(unavailable["error"]["kind"],"unavailable");
        let input = b"{\"op\":\"qualify\",\"id\":\"same\",\"targets\":[]}\n{\"op\":\"qualify\",\"id\":\"same\",\"targets\":[]}\n";
        let mut output = Vec::new();
        assert!(serve(&owner,&input[..],&mut output).await.is_err());
        let lines = output.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()).map(serde_json::from_slice::<Value>).collect::<std::result::Result<Vec<_>,_>>()?;
        assert_eq!(lines.len(),2);
        assert_eq!(lines[1]["error"]["kind"],"protocol");
        let proof_digest: B256 = serde_json::from_value(response["receipts"][0]["proofDigest"].clone())?;
        let mut corrupted: Value = owner.load(proof_digest)?;
        corrupted["inclusion"]["index"] = json!(0);
        fs::write(owner.directory.join(format!("evidence-{proof_digest:x}.json")),serde_json::to_vec(&corrupted)?)?;
        let corrupt = owner.execute(fixture.request(checkpoint,hash,"corrupt")?).await;
        assert_eq!(corrupt["error"]["kind"],"conflict");
        let mut forged_checkpoint = checkpoint.clone();
        forged_checkpoint["beacon"]["proofDigest"] = json!(B256::repeat_byte(83));
        let corrupt = owner.execute(fixture.request(&forged_checkpoint,hash,"corrupt-finality")?).await;
        assert_eq!(corrupt["ok"],false);
        Ok(())
    }
}
