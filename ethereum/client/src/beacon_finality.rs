//! Beacon consensus is the sole finality authority; execution RPC supplies proof data only.
use alloy::{primitives::{B256, Bytes}, providers::Provider, rpc::types::Header};
use anyhow::{bail, ensure, Context, Result};
use builtins_common::bls12_381::{ark_serialize::CanonicalDeserialize, Bls12_381Ops, Bls12_381OpsLowLevel};
use checkpoint_light_client_io::{verification::{self, BackendError, BlsBackend, Request, Response, VerificationState}, BeaconBlockHeader, Init, Network, SyncCommitteeKeys, Update, G1, G1TypeInfo, G2};
use ethereum_beacon_client::BeaconClient;
use ethereum_common::{beacon, tree_hash::TreeHash, utils};
use parity_scale_codec::{Decode, DecodeAll, Encode};
use serde::{Deserialize, Serialize};
use std::{fs::{self, File}, path::{Path, PathBuf}, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio::sync::Mutex;

pub const POLICY: &str = "beacon-sync-committee-v1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapApproval {
    pub schema_version: u32,
    pub chain_id: u64,
    pub genesis_hash: B256,
    #[serde(with = "network_json")]
    pub network: Network,
    pub genesis_validators_root: B256,
    pub genesis_time: u64,
    pub trusted_bootstrap_root: B256,
    pub trusted_bootstrap_slot: u64,
    pub approved_at_ms: u64,
    pub bootstrap_expires_at_ms: u64,
}

mod network_json {
    use super::*;
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    enum Wire {
        Mainnet, Sepolia, Holesky, Hoodi,
        LocalFixture { genesis_time: u64, genesis_validators_root: B256 },
    }
    pub fn serialize<S: serde::Serializer>(network: &Network, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match network {
            Network::Mainnet => Wire::Mainnet,
            Network::Sepolia => Wire::Sepolia,
            Network::Holesky => Wire::Holesky,
            Network::Hoodi => Wire::Hoodi,
            Network::LocalFixture { genesis_time, genesis_validators_root } => Wire::LocalFixture {
                genesis_time: *genesis_time, genesis_validators_root: B256::from(genesis_validators_root.0),
            },
        };
        wire.serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Network, D::Error> {
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Mainnet => Network::Mainnet, Wire::Sepolia => Network::Sepolia,
            Wire::Holesky => Network::Holesky, Wire::Hoodi => Network::Hoodi,
            Wire::LocalFixture { genesis_time, genesis_validators_root } => Network::LocalFixture {
                genesis_time, genesis_validators_root: genesis_validators_root.0.into(),
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct BeaconFinalityConfig {
    approval: BootstrapApproval,
    approval_bytes: Bytes,
    approval_hash: B256,
    beacon_rpc: String,
    proof_sources: Vec<PathBuf>,
}
impl BeaconFinalityConfig {
    /// The digest must come from the independent approval channel, not the serving endpoint.
    pub fn load(path: impl AsRef<Path>, expected_sha256: B256, beacon_rpc: &str) -> Result<Self> {
        Self::parse(fs::read(path)?, expected_sha256, beacon_rpc)
    }
    fn parse(bytes: Vec<u8>, expected: B256, beacon_rpc: &str) -> Result<Self> {
        ensure!(expected != B256::ZERO && digest(&bytes) == expected, "HOLD: bootstrap approval digest mismatch");
        let approval: BootstrapApproval = serde_json::from_slice(&bytes)?;
        ensure!(approval.schema_version == 1 && approval.chain_id != 0
            && approval.genesis_hash != B256::ZERO && approval.trusted_bootstrap_root != B256::ZERO
            && approval.trusted_bootstrap_slot != 0
            && approval.approved_at_ms < approval.bootstrap_expires_at_ms,
            "HOLD: invalid bootstrap approval");
        ensure!(approval.genesis_time == approval.network.genesis_time()
            && approval.genesis_validators_root.0 == approval.network.genesis_validators_root().0,
            "HOLD: approval network genesis mismatch");
        let expected_chain = match approval.network {
            Network::Mainnet => 1, Network::Sepolia => 11155111,
            Network::Holesky => 17000, Network::Hoodi => 560048,
            Network::LocalFixture { .. } => 31337,
        };
        ensure!(expected_chain == approval.chain_id, "HOLD: approval chain/network mismatch");
        let url = reqwest::Url::parse(beacon_rpc)?;
        let loopback = url.host_str().is_some_and(|host| host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
        ensure!(url.scheme() == "https" || (url.scheme() == "http" && loopback), "HOLD: Beacon RPC requires HTTPS except literal loopback");
        ensure!(!matches!(approval.network, Network::LocalFixture { .. }) || loopback, "HOLD: local fixture network requires literal loopback");
        Ok(Self { approval, approval_bytes: bytes.into(), approval_hash: expected, beacon_rpc: beacon_rpc.trim_end_matches('/').to_owned(), proof_sources: Vec::new() })
    }
    pub fn identity(&self) -> (u64, B256) { (self.approval.chain_id, self.approval.genesis_hash) }
    /// Read immutable, digest-addressed proofs from explicitly approved peer stores.
    /// Peer mutable caches, revision files and owner locks are never consulted.
    pub fn with_proof_sources(mut self, sources: Vec<PathBuf>) -> Result<Self> {
        let mut canonical = std::collections::BTreeSet::new();
        for source in sources {
            let source = fs::canonicalize(source).context("HOLD: proof source directory unavailable")?;
            ensure!(source.is_dir(), "HOLD: proof source is not a directory");
            canonical.insert(source);
        }
        self.proof_sources = canonical.into_iter().collect();
        Ok(self)
    }
    pub(crate) fn validate_execution_endpoint(&self, url: &reqwest::Url) -> Result<()> {
        if matches!(self.approval.network, Network::LocalFixture { .. }) {
            ensure!(url.host_str().is_some_and(|host| host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())),
                "HOLD: local fixture execution RPC requires literal loopback");
        }
        Ok(())
    }
}

/// Uses the very same installed POP implementation as the Gear builtin.
#[derive(Debug)]
pub struct NativeBls;
impl BlsBackend for NativeBls {
    async fn call(&self, request: Request) -> Result<Vec<u8>, BackendError> {
        let response = match request {
            Request::AggregateG1 { points } => Bls12_381OpsLowLevel::aggregate_g1(points).map(Response::AggregateG1),
            Request::MapToG2Affine { message } => Bls12_381OpsLowLevel::map_to_g2affine(message).map(Response::MapToG2Affine),
            Request::MultiMillerLoop { a, b } => Bls12_381OpsLowLevel::multi_miller_loop(a, b).map(Response::MultiMillerLoop),
            Request::FinalExponentiation { f } => Bls12_381OpsLowLevel::final_exponentiation(f).map(Response::FinalExponentiation),
            _ => return Err(BackendError::MalformedResponse),
        }.map_err(|_| BackendError::MalformedResponse)?;
        Ok(response.encode())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeaconCheckpointProof {
    pub genesis_validators_root: B256,
    pub bootstrap_approval_hash: B256,
    pub finalized_beacon_header_scale: Bytes,
    pub proof_digest: B256,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyFinalityCheckpoint {
    pub schema_version: u32,
    pub policy: String,
    pub chain_id: u64,
    pub genesis_hash: B256,
    pub common_header: alloy::consensus::Header,
    pub primary_finalized_header: alloy::consensus::Header,
    pub witness_finalized_header: alloy::consensus::Header,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeaconFinalityCheckpoint {
    pub schema_version: u32,
    pub policy: String,
    pub chain_id: u64,
    pub genesis_hash: B256,
    pub common_header: alloy::consensus::Header,
    pub beacon: BeaconCheckpointProof,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FinalityCheckpoint {
    Legacy(LegacyFinalityCheckpoint),
    Beacon(BeaconFinalityCheckpoint),
}
impl<'de> Deserialize<'de> for FinalityCheckpoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire { Legacy(LegacyFinalityCheckpoint), Beacon(BeaconFinalityCheckpoint) }
        let checkpoint = match Wire::deserialize(deserializer)? {
            Wire::Legacy(value) => Self::Legacy(value),
            Wire::Beacon(value) => Self::Beacon(value),
        };
        checkpoint.validate().map_err(serde::de::Error::custom)?;
        Ok(checkpoint)
    }
}
impl FinalityCheckpoint {
    pub fn schema_version(&self) -> u32 {
        match self { Self::Legacy(value) => value.schema_version, Self::Beacon(value) => value.schema_version }
    }
    pub fn policy(&self) -> &str {
        match self { Self::Legacy(value) => &value.policy, Self::Beacon(value) => &value.policy }
    }
    pub fn chain_id(&self) -> u64 {
        match self { Self::Legacy(value) => value.chain_id, Self::Beacon(value) => value.chain_id }
    }
    pub fn genesis_hash(&self) -> B256 {
        match self { Self::Legacy(value) => value.genesis_hash, Self::Beacon(value) => value.genesis_hash }
    }
    pub fn common_header(&self) -> &alloy::consensus::Header {
        match self { Self::Legacy(value) => &value.common_header, Self::Beacon(value) => &value.common_header }
    }
    pub fn legacy(&self) -> Option<&LegacyFinalityCheckpoint> {
        match self { Self::Legacy(value) => Some(value), _ => None }
    }
    pub fn beacon(&self) -> Option<&BeaconFinalityCheckpoint> {
        match self { Self::Beacon(value) => Some(value), _ => None }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.chain_id() != 0 && self.genesis_hash() != B256::ZERO, "HOLD: invalid checkpoint network");
        match self {
            Self::Legacy(value) => ensure!(value.schema_version == 1 && value.policy == "independent-two-el-v1"
                && value.common_header.number <= value.primary_finalized_header.number
                && value.common_header.number <= value.witness_finalized_header.number,
                "HOLD: invalid historical two-EL checkpoint"),
            Self::Beacon(value) => {
                ensure!(value.schema_version == 2 && value.policy == POLICY
                    && value.beacon.proof_digest != B256::ZERO
                    && value.beacon.bootstrap_approval_hash != B256::ZERO
                    && value.beacon.genesis_validators_root != B256::ZERO,
                    "HOLD: invalid Beacon checkpoint");
                decode_exact::<BeaconBlockHeader>(&value.beacon.finalized_beacon_header_scale)?;
            }
        }
        Ok(())
    }
    /// Includes every advertised historical fence, not just the old common header.
    pub(crate) fn fences(&self) -> Result<std::collections::BTreeMap<u64, B256>> {
        self.validate()?;
        let mut targets = std::collections::BTreeMap::new();
        let mut insert = |header: &alloy::consensus::Header| -> Result<()> {
            let hash = header.hash_slow();
            ensure!(targets.insert(header.number, hash).is_none_or(|old| old == hash), "HOLD: conflicting checkpoint fences");
            Ok(())
        };
        insert(self.common_header())?;
        if let Some(legacy) = self.legacy() {
            insert(&legacy.primary_finalized_header)?;
            insert(&legacy.witness_finalized_header)?;
        }
        Ok(targets)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacySubmissionSnapshot {
    pub block_number: u64,
    pub block_hash: B256,
    pub pinned_nonce_consumed: bool,
    pub message_processed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenSubmissionSnapshot {
    pub schema_version: u32,
    pub block_number: u64,
    pub block_hash: B256,
    pub account_nonce: u64,
    pub pinned_nonce_consumed: bool,
    pub account_proof: crate::inclusion::RecordedAccountProof,
    pub checkpoint: FinalityCheckpoint,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum FinalizedSubmissionSnapshot {
    Legacy(LegacySubmissionSnapshot),
    Beacon(ProvenSubmissionSnapshot),
}
impl<'de> Deserialize<'de> for FinalizedSubmissionSnapshot {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire { Legacy(LegacySubmissionSnapshot), Beacon(ProvenSubmissionSnapshot) }
        match Wire::deserialize(deserializer)? {
            Wire::Legacy(value) => Ok(Self::Legacy(value)),
            Wire::Beacon(value) => {
                if value.schema_version != 2 || value.checkpoint.beacon().is_none() {
                    return Err(serde::de::Error::custom("invalid proven nonce snapshot schema/policy"));
                }
                Ok(Self::Beacon(value))
            }
        }
    }
}
impl FinalizedSubmissionSnapshot {
    pub fn block_number(&self) -> u64 {
        match self { Self::Legacy(value) => value.block_number, Self::Beacon(value) => value.block_number }
    }
    pub fn block_hash(&self) -> B256 {
        match self { Self::Legacy(value) => value.block_hash, Self::Beacon(value) => value.block_hash }
    }
    pub fn pinned_nonce_consumed(&self) -> bool {
        match self { Self::Legacy(value) => value.pinned_nonce_consumed, Self::Beacon(value) => value.pinned_nonce_consumed }
    }
    pub fn legacy(&self) -> Option<&LegacySubmissionSnapshot> {
        match self { Self::Legacy(value) => Some(value), _ => None }
    }
    pub fn beacon(&self) -> Option<&ProvenSubmissionSnapshot> {
        match self { Self::Beacon(value) => Some(value), _ => None }
    }
}

#[derive(Clone, Encode, Decode)]
struct BeaconStore {
    revision: u64,
    transition: [u8; 32],
    finalized_header: BeaconBlockHeader,
    current_committee: Box<SyncCommitteeKeys>,
    next_committee: Box<SyncCommitteeKeys>,
}
impl BeaconStore {
    fn view<'a>(&'a self, network: &'a Network) -> VerificationState<'a> {
        VerificationState { network, finalized_slot: self.finalized_header.slot,
            current_committee: &self.current_committee, next_committee: &self.next_committee }
    }
}

#[derive(Encode, Decode)]
enum ProofRecord {
    Bootstrap { approval_hash: [u8; 32], verified_at_ms: u64, init: Init },
    Update { parent: [u8; 32], verified_at_ms: u64, update: Update, aggregate: Vec<u8> },
    FullBlock { transition: [u8; 32], block: FullBlock },
}

#[derive(Encode, Decode)]
enum FullBlock {
    Deneb(beacon::Block),
    Electra(beacon::electra::Block),
    Fulu(beacon::electra::Block),
}
impl FullBlock {
    fn binding(&self, network: &Network, header: &BeaconBlockHeader) -> Result<(u64, B256)> {
        let epoch = utils::calculate_epoch(header.slot);
        let (slot, root, payload) = match self {
            Self::Deneb(block) if epoch >= network.epoch_deneb() && epoch < network.epoch_electra() =>
                (block.slot, block.tree_hash_root(), &block.body.execution_payload),
            Self::Electra(block) if epoch >= network.epoch_electra() && epoch < network.epoch_fulu() =>
                (block.slot, block.tree_hash_root(), &block.body.execution_payload),
            Self::Fulu(block) if epoch >= network.epoch_fulu() =>
                (block.slot, block.tree_hash_root(), &block.body.execution_payload),
            _ => bail!("HOLD: unsupported or mismatched Beacon fork"),
        };
        ensure!(slot == header.slot && root == header.tree_hash_root(), "HOLD: full Beacon block/header root mismatch");
        Ok((payload.block_number, B256::from(payload.block_hash.0.0)))
    }
}

pub(crate) struct BeaconAuthority {
    config: BeaconFinalityConfig,
    client: BeaconClient,
    directory: PathBuf,
    state: Mutex<Arc<BeaconStore>>,
    archives: std::sync::Mutex<Vec<Arc<super::finality_archive::HeaderArchive>>>,
    _owner: File,
}
impl std::fmt::Debug for BeaconAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BeaconAuthority").field("approval_hash", &self.config.approval_hash).finish_non_exhaustive()
    }
}

fn digest(bytes: &[u8]) -> B256 { B256::from(sp_core::hashing::sha2_256(bytes)) }
fn now_ms() -> Result<u64> { Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?) }
fn decode_exact<T: Decode + Encode>(bytes: &[u8]) -> Result<T> {
    let value = T::decode_all(&mut &bytes[..]).map_err(|error| anyhow::anyhow!("HOLD: malformed SCALE proof: {error}"))?;
    ensure!(value.encode() == bytes, "HOLD: noncanonical SCALE proof");
    Ok(value)
}
fn read_scale<T: Decode + Encode>(path: &Path) -> Result<T> {
    let bytes: Bytes = serde_json::from_slice(&gear_common::durable::committed_bytes(path)?)?;
    decode_exact(&bytes)
}
fn keys(committee: &beacon::SyncCommittee) -> Result<Box<SyncCommitteeKeys>> {
    let points = committee.pubkeys.0.iter().map(|key| {
        G1::deserialize_compressed(&key.0.0[..]).map(|key| G1TypeInfo(key).into())
            .map_err(|error| anyhow::anyhow!("HOLD: malformed committee public key: {error}"))
    }).collect::<Result<Vec<_>>>()?;
    Ok(Box::new(ethereum_common::base_types::FixedArray(points.try_into().map_err(|_| anyhow::anyhow!("HOLD: wrong committee size"))?)))
}
fn update_wire(update: utils::Update) -> Result<(Update, Vec<u8>)> {
    let signature = G2::deserialize_compressed(&update.sync_aggregate.sync_committee_signature.0.0[..])
        .map_err(|error| anyhow::anyhow!("HOLD: malformed aggregate signature: {error}"))?;
    let aggregate = update.sync_aggregate.encode();
    let next = keys(&update.next_sync_committee)?;
    Ok((Update { signature_slot: update.signature_slot, attested_header: update.attested_header,
        finalized_header: update.finalized_header, sync_committee_signature: checkpoint_light_client_io::G2TypeInfo(signature).into(),
        sync_committee_next_aggregate_pubkey: Some(update.next_sync_committee.aggregate_pubkey),
        sync_committee_next_pub_keys: Some(next),
        sync_committee_next_branch: Some(update.next_sync_committee_branch.into_iter().map(|value| value.0.0).collect()),
        finality_branch: update.finality_branch.into_iter().map(|value| value.0.0).collect(),
    }, aggregate))
}

impl BeaconAuthority {
    pub(crate) fn identity(&self) -> (u64, B256) { self.config.identity() }
    pub(crate) fn matches_config(&self, config: &BeaconFinalityConfig) -> bool {
        self.config.approval_hash == config.approval_hash
            && self.config.approval_bytes == config.approval_bytes
            && self.config.beacon_rpc == config.beacon_rpc
            && self.config.proof_sources == config.proof_sources
    }
    pub(crate) fn attach_archive(self: &Arc<Self>, archive: Arc<super::finality_archive::HeaderArchive>) -> Result<()> {
        archive.claim(self)?;
        let mut archives = self.archives.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !archives.iter().any(|registered| Arc::ptr_eq(registered, &archive)) {
            archives.push(archive);
        }
        Ok(())
    }
    pub(crate) fn archives(&self) -> Vec<Arc<super::finality_archive::HeaderArchive>> {
        self.archives.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
    fn record_path(&self, hash: B256) -> PathBuf { self.directory.join(format!("proof-{hash:x}.json")) }
    fn record(&self, record: &ProofRecord) -> Result<B256> {
        self.record_bytes(Bytes::from(record.encode()))
    }
    fn record_bytes(&self, bytes: Bytes) -> Result<B256> {
        let hash = digest(&bytes);
        gear_common::durable::immutable_json(&self.record_path(hash), &bytes)?;
        Ok(hash)
    }
    fn read_record(&self, hash: B256) -> Result<ProofRecord> {
        for directory in std::iter::once(&self.directory).chain(&self.config.proof_sources) {
            let path = directory.join(format!("proof-{hash:x}.json"));
            if !gear_common::durable::entry_exists(&path)?
                && !gear_common::durable::entry_exists(&PathBuf::from(format!("{}.new", path.display())))?
                && !gear_common::durable::entry_exists(&path.with_extension("tmp"))?
                && !gear_common::durable::entry_exists(&PathBuf::from(format!("{}.tmp", path.display())))?
            {
                continue;
            }
            let bytes: Bytes = serde_json::from_slice(&gear_common::durable::committed_bytes(&path)?)?;
            ensure!(digest(&bytes) == hash, "HOLD: retained Beacon proof digest mismatch");
            return decode_exact(&bytes);
        }
        bail!("HOLD: retained Beacon proof is unavailable in approved sources")
    }
    fn revision_path(&self, revision: u64) -> PathBuf { self.directory.join(format!("revision-{revision}.json")) }
    fn retain_store(&self, store: &BeaconStore) -> Result<()> {
        gear_common::durable::immutable_json(&self.revision_path(store.revision), &B256::from(store.transition))?;
        super::finality_archive::atomic_json(&self.directory.join("store.json"), &Bytes::from(store.encode()))
    }
    async fn bootstrap_record(config: &BeaconFinalityConfig, init: Init, at: u64, hash: B256) -> Result<BeaconStore> {
        let approval = &config.approval;
        ensure!(at >= approval.approved_at_ms && at <= approval.bootstrap_expires_at_ms && at <= now_ms()?, "HOLD: bootstrap outside approved validity window");
        ensure!(init.network.encode() == approval.network.encode()
            && init.trusted_bootstrap_root.0 == approval.trusted_bootstrap_root.0
            && init.bootstrap_header.slot == approval.trusted_bootstrap_slot,
            "HOLD: bootstrap does not match independent approval");
        let verified = verification::verify_bootstrap(init, at, &NativeBls).await?;
        Ok(BeaconStore { revision: 0, transition: hash.0, finalized_header: verified.finalized_header,
            current_committee: verified.current_committee, next_committee: verified.next_committee })
    }
    async fn apply(&self, base: &BeaconStore, update: Update, aggregate: &[u8], at: u64, hash: B256) -> Result<BeaconStore> {
        ensure!(at <= now_ms()?, "HOLD: future transition timestamp");
        let verified = verification::verify_update(base.view(&self.config.approval.network), at, update, aggregate, &NativeBls).await?;
        let (current_committee, next_committee) = match verified.next_committee {
            Some(next) => (base.next_committee.clone(), next),
            None => (base.current_committee.clone(), base.next_committee.clone()),
        };
        Ok(BeaconStore {
            revision: base.revision.checked_add(1).context("HOLD: Beacon revision overflow")?,
            transition: hash.0,
            finalized_header: verified.finalized_header.unwrap_or_else(|| base.finalized_header.clone()),
            current_committee,
            next_committee,
        })
    }
    /// Authenticate the complete linked transcript, never cached committee keys.
    async fn replay(&self, tip: B256) -> Result<BeaconStore> {
        let mut path = Vec::new();
        let mut hash = tip;
        let mut state = loop {
            match self.read_record(hash)? {
                ProofRecord::Bootstrap { approval_hash, verified_at_ms, init } => {
                    ensure!(approval_hash == self.config.approval_hash.0, "HOLD: transcript approval mismatch");
                    break Self::bootstrap_record(&self.config, init, verified_at_ms, hash).await?;
                }
                ProofRecord::Update { parent, .. } => {
                    path.push(hash);
                    hash = B256::from(parent);
                }
                ProofRecord::FullBlock { .. } => bail!("HOLD: full block is not a committee transition"),
            }
        };
        for hash in path.into_iter().rev() {
            let ProofRecord::Update { parent, verified_at_ms, update, aggregate } = self.read_record(hash)? else {
                bail!("HOLD: invalid signed transition record")
            };
            ensure!(parent == state.transition, "HOLD: signed transition parent mismatch");
            state = self.apply(&state, update, &aggregate, verified_at_ms, hash).await?;
        }
        Ok(state)
    }
    pub(crate) async fn open(config: BeaconFinalityConfig, directory: &Path) -> Result<Arc<Self>> {
        fs::create_dir_all(directory)?;
        let owner = File::options().create(true).truncate(false).read(true).write(true).open(directory.join("owner.lock"))?;
        owner.try_lock().context("HOLD: another owner holds this Beacon store")?;
        let client = BeaconClient::with_client(config.beacon_rpc.clone(), reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT).redirect(reqwest::redirect::Policy::none()).build()?, Some(REQUEST_TIMEOUT));
        let approval_path = directory.join("approval.json");
        let initial = directory.join("revision-0.json");
        let (state, bootstrap) = if initial.try_exists()? {
            let retained: Bytes = serde_json::from_slice(&gear_common::durable::committed_bytes(&approval_path)?)?;
            ensure!(retained == config.approval_bytes, "HOLD: retained approval differs from independent approval");
            let hash: B256 = serde_json::from_slice(&gear_common::durable::committed_bytes(&initial)?)?;
            let bytes: Bytes = serde_json::from_slice(&gear_common::durable::committed_bytes(&directory.join(format!("proof-{hash:x}.json")))?)?;
            ensure!(digest(&bytes) == hash, "HOLD: bootstrap digest mismatch");
            let ProofRecord::Bootstrap { approval_hash, verified_at_ms, init } = decode_exact(&bytes)? else { bail!("HOLD: missing bootstrap transcript") };
            ensure!(approval_hash == config.approval_hash.0, "HOLD: bootstrap approval mismatch");
            (Self::bootstrap_record(&config, init, verified_at_ms, hash).await?, None)
        } else {
            ensure!(!directory.join("store.json").try_exists()?, "HOLD: missing bootstrap history");
            let at = now_ms()?;
            ensure!(at >= config.approval.approved_at_ms && at <= config.approval.bootstrap_expires_at_ms, "HOLD: fresh bootstrap approval expired or not yet valid");
            let bootstrap = client.get_bootstrap(&format!("{:#x}", config.approval.trusted_bootstrap_root)).await?;
            let period = utils::calculate_period(bootstrap.header.slot);
            let update = client.get_updates(period, 1).await?.into_iter().next()
                .ok_or(crate::Error::FinalizedEvidenceUnavailable("bootstrap period update"))?.data;
            let (update, aggregate) = update_wire(update)?;
            let init = Init { network: config.approval.network.clone(), bootstrap_header: bootstrap.header,
                sync_committee_current_pub_keys: keys(&bootstrap.current_sync_committee)?,
                sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
                sync_committee_current_branch: bootstrap.current_sync_committee_branch.into_iter().map(|value| value.0.0).collect(),
                update, sync_aggregate_encoded: aggregate, trusted_bootstrap_root: config.approval.trusted_bootstrap_root.0.into() };
            let record = ProofRecord::Bootstrap { approval_hash: config.approval_hash.0, verified_at_ms: at, init };
            let bytes = Bytes::from(record.encode());
            let ProofRecord::Bootstrap { init, .. } = record else { unreachable!() };
            let state = Self::bootstrap_record(&config, init, at, digest(&bytes)).await?;
            ensure!(now_ms()? <= config.approval.bootstrap_expires_at_ms, "HOLD: bootstrap approval expired during acquisition");
            (state, Some(bytes))
        };
        let authority = Arc::new(Self { config, client, directory: directory.to_owned(), state: Mutex::new(Arc::new(state)), archives: Default::default(), _owner: owner });
        if let Some(record) = bootstrap {
            gear_common::durable::immutable_json(&approval_path, &authority.config.approval_bytes)?;
            authority.record_bytes(record)?;
            let state = authority.state.lock().await;
            authority.retain_store(state.as_ref())?;
        } else {
            let mut state = authority.state.lock().await;
            // Every immutable revision is retained; a cache rollback cannot hide signed transitions.
            let mut revisions = std::collections::BTreeMap::new();
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let name = entry.file_name();
                if let Some(number) = name.to_str().and_then(|name| name.strip_prefix("revision-")).and_then(|name| name.strip_suffix(".json")) {
                    revisions.insert(number.parse::<u64>()?, entry.path());
                }
            }
            let saved = if directory.join("store.json").try_exists()? { Some(read_scale::<BeaconStore>(&directory.join("store.json"))?) } else { None };
            let mut saved_matches = saved.as_ref().is_none_or(|saved| saved.encode() == state.encode());
            for (revision, path) in revisions.into_iter().skip(1) {
                ensure!(revision == state.revision + 1, "HOLD: missing Beacon transition history");
                let hash: B256 = serde_json::from_slice(&gear_common::durable::committed_bytes(&path)?)?;
                let ProofRecord::Update { parent, verified_at_ms, update, aggregate } = authority.read_record(hash)? else { bail!("HOLD: invalid transition record") };
                ensure!(parent == state.transition, "HOLD: transition parent mismatch");
                *state = Arc::new(authority.apply(&state, update, &aggregate, verified_at_ms, hash).await?);
                saved_matches |= saved.as_ref().is_some_and(|saved| saved.encode() == state.encode());
            }
            ensure!(saved_matches, "HOLD: derived store differs from authenticated transcript");
            authority.retain_store(&state)?;
        }
        Ok(authority)
    }
    async fn commit_update(&self, base: Arc<BeaconStore>, update: Update, aggregate: Vec<u8>) -> Result<()> {
        let at = now_ms()?;
        let record = ProofRecord::Update { parent: base.transition, verified_at_ms: at, update, aggregate };
        let bytes = Bytes::from(record.encode());
        let hash = digest(&bytes);
        let ProofRecord::Update { update, aggregate, .. } = record else { unreachable!() };
        let next = self.apply(&base, update, &aggregate, at, hash).await?;
        let mut state = self.state.lock().await;
        ensure!(state.revision == base.revision && state.transition == base.transition, "HOLD: Beacon state changed during verification");
        self.record_bytes(bytes)?;
        self.retain_store(&next)?;
        *state = Arc::new(next);
        Ok(())
    }
    async fn refresh(&self) -> Result<Arc<BeaconStore>> {
        let finality = self.client.get_finality_update().await?;
        let target = finality.finalized_header.slot;
        loop {
            let base = self.state.lock().await.clone();
            if target <= base.finalized_header.slot { return Ok(base); }
            let period = utils::calculate_period(base.finalized_header.slot);
            let target_period = utils::calculate_period(target);
            if target_period > period {
                let update = self.client.get_updates(period + 1, 1).await?.into_iter().next()
                    .ok_or(crate::Error::FinalizedEvidenceUnavailable("committee transition"))?.data;
                let (update, aggregate) = update_wire(update)?;
                ensure!(update.finalized_header.slot > base.finalized_header.slot, "HOLD: nonprogressing committee transition");
                self.commit_update(base, update, aggregate).await?;
            } else {
                let signature = G2::deserialize_compressed(&finality.sync_aggregate.sync_committee_signature.0.0[..])
                    .map_err(|error| anyhow::anyhow!("HOLD: malformed finality signature: {error}"))?;
                let aggregate = finality.sync_aggregate.encode();
                let update = ethereum_beacon_client::utils::sync_update_from_finality(signature, finality);
                self.commit_update(base, update, aggregate).await?;
                return Ok(self.state.lock().await.clone());
            }
        }
    }
    pub(crate) async fn pin(&self, provider: &impl Provider) -> Result<(Header, BeaconCheckpointProof)> {
        let (chain, genesis) = self.identity();
        ensure!(tokio::time::timeout(REQUEST_TIMEOUT, provider.get_chain_id())
            .await.map_err(alloy::transports::TransportErrorKind::custom)?? == chain,
            "HOLD: execution chain mismatch");
        let genesis_header = super::qualified_header(provider, alloy::rpc::types::BlockNumberOrTag::Number(0)).await?;
        ensure!(genesis_header.hash == genesis, "HOLD: execution genesis mismatch");
        let state = self.refresh().await?;
        let root = state.finalized_header.tree_hash_root();
        let epoch = utils::calculate_epoch(state.finalized_header.slot);
        let network = &self.config.approval.network;
        let block = if epoch >= network.epoch_fulu() {
            FullBlock::Fulu(self.client.get_block_by_hash(&root.0).await?)
        } else if epoch >= network.epoch_electra() {
            FullBlock::Electra(self.client.get_block_by_hash(&root.0).await?)
        } else {
            ensure!(epoch >= network.epoch_deneb(), "HOLD: unsupported Beacon fork");
            FullBlock::Deneb(self.client.get_block_by_hash(&root.0).await?)
        };
        let (number, hash) = block.binding(network, &state.finalized_header)?;
        let execution = tokio::time::timeout(REQUEST_TIMEOUT, provider.get_block_by_hash(hash))
            .await.map_err(alloy::transports::TransportErrorKind::custom)??
            .ok_or(crate::Error::FinalizedEvidenceUnavailable("authenticated execution header"))?.header;
        ensure!(execution.number == number && execution.hash == hash && execution.inner.hash_slow() == hash, "HOLD: execution RLP/payload binding mismatch");
        let proof_digest = self.record(&ProofRecord::FullBlock { transition: state.transition, block })?;
        Ok((execution, BeaconCheckpointProof { genesis_validators_root: self.config.approval.genesis_validators_root,
            bootstrap_approval_hash: self.config.approval_hash,
            finalized_beacon_header_scale: state.finalized_header.encode().into(), proof_digest }))
    }
    pub(crate) async fn verify_proof(&self, proof: &BeaconCheckpointProof, execution: &alloy::consensus::Header) -> Result<()> {
        ensure!(proof.bootstrap_approval_hash == self.config.approval_hash
            && proof.genesis_validators_root == self.config.approval.genesis_validators_root,
            "HOLD: checkpoint approval identity mismatch");
        let ProofRecord::FullBlock { transition, block } = self.read_record(proof.proof_digest)? else { bail!("HOLD: checkpoint full block proof missing") };
        let state = self.replay(B256::from(transition)).await?;
        ensure!(state.finalized_header.encode() == proof.finalized_beacon_header_scale.as_ref(), "HOLD: checkpoint Beacon header differs from verified transcript");
        let (number, hash) = block.binding(&self.config.approval.network, &state.finalized_header)?;
        ensure!(execution.number == number && execution.hash_slow() == hash, "HOLD: checkpoint execution RLP binding mismatch");
        Ok(())
    }
    pub(crate) fn retain_requalification(&self, original: &impl Serialize, proof: &BeaconCheckpointProof) -> Result<()> {
        let original_bytes = serde_json::to_vec(original)?;
        let original_digest = digest(&original_bytes);
        gear_common::durable::immutable_json(&self.directory.join(format!("legacy-{original_digest:x}.json")), &Bytes::from(original_bytes))?;
        gear_common::durable::immutable_json(&self.directory.join(format!("requalified-{original_digest:x}-{:x}.json", proof.proof_digest)), &(original_digest, proof))
    }
}

#[cfg(test)]
#[path = "beacon_finality_tests.rs"]
mod tests;

#[cfg(any(test, feature = "test-fixtures"))]
#[path = "beacon_test_fixture.rs"]
pub mod test_fixtures;
