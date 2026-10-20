//! Immutable cutover seals. Reading an original lane never restores its mutable runtime.
use super::{
    message_sender::MessageSender, storage::InboundRuntimeIdentity, tx_manager::Transaction,
};
use crate::message_relayer::common::ethereum::block_storage::DiscoveryState;
use anyhow::{Context, Result};
use gclient::GearApi;
use gear_common::{
    api_provider::ApiProviderConnection,
    durable::{committed_bytes, entry_exists, immutable_json},
};
use parity_scale_codec::{Decode, Encode};
use primitive_types::H256;
use sails_rs::{
    calls::{ActionIo, Query},
    gclient::calls::{GClientRemoting, QueryExtGClient},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use vft_manager_client::{traits::VftManager as _, CutoverPhase, CutoverStatus, VftManager};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalExternalOriginal {
    pub provenance: String,
    pub original: super::message_sender::SignedSubmission,
    pub discovered_at: super::message_sender::FinalizedReceiptObservation,
}

// Only the fields needed to bind immutable external coverage to never-dispatched old rows.
// The tool separately authenticates the complete transaction/receipt-trie coverage report.
#[derive(Deserialize)]
struct ExternalCoverageReport {
    schema_version: u32,
    admissions: Vec<ExternalCoverageAdmission>,
}
#[derive(Deserialize)]
struct ExternalCoverageAdmission {
    receipt_key: (u64, u64),
    original_tx_hash: alloy_primitives::B256,
    original_block_hash: H256,
    admission: String,
    proof: Vec<u8>,
    settlement: Option<super::message_sender::FinalizedReceiptObservation>,
    #[serde(default)]
    signed: Option<super::message_sender::SignedSubmission>,
    #[serde(default)]
    attempts: Vec<super::message_sender::SignedSubmission>,
    #[serde(default)]
    original_lane_uuid: Option<Uuid>,
    #[serde(default)]
    external_original: Option<CanonicalExternalOriginal>,
    #[serde(default)]
    undispatched_old_admission: Option<Uuid>,
}
/// A foreign original never replaces a retained signature, native attempt, known reply or handoff.
pub fn is_undispatched_old_admission(transaction: &Transaction) -> bool {
    use super::tx_manager::TxStatus;
    matches!(
        transaction.status,
        TxStatus::ComposeProof
            | TxStatus::SubmitMessage
            | TxStatus::PreparingSubmission
            | TxStatus::Failed { .. }
            | TxStatus::NeedsReconciliation { .. }
    ) && transaction.saved_settlement_observation.is_none()
        && transaction.receipt.as_ref().is_none_or(|receipt| {
            receipt.signed_submission.is_none()
                && receipt.submission_attempts.is_empty()
                && receipt.initial_response.is_none()
                && receipt.handed_off_at_ms.is_none()
        })
}

async fn verify_external_coverage(
    seal: &CutoverSeal,
    journal: &OriginalJournal,
    coverage: &ExternalCoverageReport,
    sender: &MessageSender,
    api: &GearApi,
) -> Result<(
    std::collections::BTreeSet<Uuid>,
    BTreeMap<(u64, ethereum_client::TxHash), (u64, H256)>,
)> {
    let mut unsigned = std::collections::BTreeSet::new();
    let mut fulfilled = BTreeMap::new();
    let mut receipt_keys = std::collections::BTreeSet::new();
    let head = api.last_block_hash().await?;
    let finalized = api.block_number_at(head).await?;
    for admission in &coverage.admissions {
        let Some(external) = &admission.external_original else {
            anyhow::ensure!(
                admission.undispatched_old_admission.is_none()
                    && admission.admission != "externally-delivered",
                "HOLD: external admission lacks its canonical original evidence"
            );
            continue;
        };
        anyhow::ensure!(
            external.provenance == "canonical-external"
                && admission.admission == "externally-delivered"
                && admission.signed.is_none()
                && admission.attempts.is_empty()
                && admission.original_lane_uuid.is_none()
                && receipt_keys.insert(admission.receipt_key),
            "HOLD: external provenance is mixed with own originals or duplicates another receipt"
        );
        let mut input = admission.proof.as_slice();
        let event = eth_events_electra_client::EthToVaraEvent::decode(&mut input)?;
        let execution = &event.proof_block.block.body.execution_payload;
        let block_hash = H256::from(execution.block_hash.0 .0);
        anyhow::ensure!(
            input.is_empty()
                && admission.receipt_key == (event.proof_block.block.slot, event.transaction_index)
                && admission.receipt_key.0 >= seal.source_receipt_floor
                && admission.receipt_key.0 <= seal.boundary.slot
                && block_hash == admission.original_block_hash
                && execution.block_number <= seal.boundary.execution_number
                && admission.original_tx_hash != alloy_primitives::B256::ZERO,
            "HOLD: external proof/key/block is outside original deployment coverage"
        );
        let settlement = admission
            .settlement
            .as_ref()
            .context("HOLD: external original has no saved full settlement pin")?;
        let discovered_number = external
            .discovered_at
            .finalized_block_number
            .context("HOLD: external discovery has no finalized height")?;
        let discovered_hash = external
            .discovered_at
            .finalized_block_hash
            .parse::<H256>()?;
        anyhow::ensure!(discovered_hash != H256::zero() && discovered_number <= finalized
            && api.get_block_hash(discovered_number).await? == discovered_hash
            && settlement.finalized_block_number.is_some_and(|number| number <= discovered_number && number <= seal.retirement.block_number),
            "HOLD: external discovery/settlement is not canonical finalized pre-retirement evidence");
        // Historical mode authenticates the actual foreign sender/raw/proof/reply and every economic effect.
        // It does not sign, rebroadcast, replace a known original, or mutate the old row.
        sender
            .verify_saved_completion(api, &event, &external.original, settlement)
            .await?;
        if let Some(uuid) = admission.undispatched_old_admission {
            let old = journal
                .transactions
                .get(&uuid)
                .context("HOLD: external link is not an unresolved original-lane admission")?;
            anyhow::ensure!(is_undispatched_old_admission(old) && unsigned.insert(uuid)
                && old.tx.tx_hash == admission.original_tx_hash && old.tx.slot_number.0 == admission.receipt_key.0,
                "HOLD: external receipt cannot replace a retained signature, handoff or another admission");
            if old.receipt.is_some() {
                let original = old.receipt_event()?;
                anyhow::ensure!(
                    original.transaction_index == event.transaction_index
                        && original.receipt_rlp == event.receipt_rlp
                        && H256::from(
                            original
                                .proof_block
                                .block
                                .body
                                .execution_payload
                                .block_hash
                                .0
                                 .0
                        ) == block_hash,
                    "HOLD: unsigned original row and foreign original prove different receipts"
                );
            }
        }
        anyhow::ensure!(
            fulfilled
                .insert(
                    (admission.receipt_key.0, admission.original_tx_hash),
                    (execution.block_number, block_hash)
                )
                .is_none(),
            "HOLD: external receipt duplicates original handoff ownership"
        );
        anyhow::ensure!(
            api.get_block_hash(discovered_number).await? == discovered_hash,
            "HOLD: external discovery finalized pin changed during qualification"
        );
    }
    let final_head = api.last_block_hash().await?;
    anyhow::ensure!(
        api.block_number_at(final_head).await? >= finalized
            && api.get_block_hash(finalized).await? == head,
        "HOLD: Gear finality changed during external original qualification"
    );
    Ok((unsigned, fulfilled))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredecessorCutover {
    pub manifest_digest: H256,
    pub predecessor_identity: InboundRuntimeIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoverBoundary {
    pub slot: u64,
    pub beacon_root: H256,
    pub execution_number: u64,
    pub execution_hash: H256,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedFile {
    pub path: PathBuf,
    pub digest: H256,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizedCutoverStatus {
    pub block_number: u32,
    pub block_hash: H256,
    pub status: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoverSeal {
    pub schema_version: u32,
    pub old_identity: InboundRuntimeIdentity,
    pub new_identity: InboundRuntimeIdentity,
    pub new_manager_code_id: H256,
    pub source_receipt_floor: u64,
    pub boundary: CutoverBoundary,
    pub config_fingerprint: H256,
    pub old_state: SealedFile,
    pub old_blocks: SealedFile,
    pub old_discovery: SealedFile,
    pub coverage: SealedFile,
    pub lifecycle_evidence: SealedFile,
    pub retirement: FinalizedCutoverStatus,
    pub readiness: FinalizedCutoverStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoverMarker {
    pub seal_path: PathBuf,
    pub manifest_digest: H256,
}

pub fn digest(bytes: &[u8]) -> H256 {
    H256(alloy_primitives::keccak256(bytes).0)
}
pub fn marker_path(journal: &Path) -> PathBuf {
    if journal.is_dir() {
        journal.join(".cutover-seal")
    } else {
        PathBuf::from(format!("{}.cutover-seal", journal.display()))
    }
}
pub fn reject_sealed(journal: &Path) -> Result<()> {
    anyhow::ensure!(
        !entry_exists(&marker_path(journal))?,
        "HOLD: sealed original journal {}; use verify-cutover (read-only)",
        journal.display()
    );
    Ok(())
}
pub fn sealed_file(path: &Path) -> Result<SealedFile> {
    let path = fs::canonicalize(path)?;
    Ok(SealedFile {
        digest: digest(&committed_bytes(&path)?),
        path,
    })
}
fn decode_status(pin: &FinalizedCutoverStatus) -> Result<CutoverStatus> {
    anyhow::ensure!(
        pin.block_hash != H256::zero(),
        "HOLD: missing finalized cutover pin"
    );
    let mut bytes = pin.status.as_slice();
    let status = CutoverStatus::decode(&mut bytes)?;
    anyhow::ensure!(bytes.is_empty(), "HOLD: trailing cutover status bytes");
    Ok(status)
}
impl CutoverSeal {
    pub fn predecessor(&self, manifest_digest: H256) -> PredecessorCutover {
        PredecessorCutover {
            manifest_digest,
            predecessor_identity: self.old_identity.clone(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        let old = &self.old_identity;
        let new = &self.new_identity;
        anyhow::ensure!(
            self.schema_version == 1
                && self.new_manager_code_id != H256::zero()
                && self.config_fingerprint != H256::zero()
                && self.boundary.beacon_root != H256::zero()
                && self.boundary.execution_hash != H256::zero()
                && self.source_receipt_floor <= self.boundary.slot
                && old.ethereum_chain_id != 0
                && old.ethereum_genesis_hash != H256::zero()
                && old.gear_genesis_hash != H256::zero()
                && old.vft_manager_address != H256::zero()
                && old.historical_proxy_address != H256::zero()
                && old.checkpoint_light_client_address != H256::zero()
                && old.gear_sender != [0; 32]
                && new.gear_sender != [0; 32]
                && old.ethereum_start_block <= self.boundary.execution_number
                && (old.erc20_manager_address.is_some() ^ old.bridging_payment_address.is_some())
                && old.vft_manager_address != new.vft_manager_address
                && new.vft_manager_address != H256::zero()
                && old.ethereum_chain_id == new.ethereum_chain_id
                && old.ethereum_genesis_hash == new.ethereum_genesis_hash
                && old.gear_genesis_hash == new.gear_genesis_hash
                && old.erc20_manager_address == new.erc20_manager_address
                && old.bridging_payment_address == new.bridging_payment_address
                && old.historical_proxy_address == new.historical_proxy_address
                && old.checkpoint_light_client_address == new.checkpoint_light_client_address
                && new.ethereum_start_block
                    == self
                        .boundary
                        .execution_number
                        .checked_add(1)
                        .context("HOLD: execution boundary overflow")?,
            "HOLD: invalid cutover identity/domain"
        );
        let retirement = decode_status(&self.retirement)?;
        let readiness = decode_status(&self.readiness)?;
        anyhow::ensure!(
            retirement.phase == CutoverPhase::Retired
                && retirement.successor == Some(new.vft_manager_address.0.into())
                && retirement.minimum_receipt_slot == self.source_receipt_floor
                && retirement.last_receipt_slot == Some(self.boundary.slot)
                && retirement.config_fingerprint == Some(self.config_fingerprint.0.into())
                && retirement.transfer_count == retirement.completed_transfer_count
                && retirement.in_flight_transfer_count == 0
                && self.readiness.block_number >= self.retirement.block_number
                && readiness.phase == CutoverPhase::Ready
                && readiness.predecessor == Some(old.vft_manager_address.0.into())
                && readiness.minimum_receipt_slot
                    == self
                        .boundary
                        .slot
                        .checked_add(1)
                        .context("HOLD: receipt boundary overflow")?
                && readiness.successor.is_none()
                && readiness.last_receipt_slot.is_none()
                && readiness.config_fingerprint == retirement.config_fingerprint,
            "HOLD: cutover retirement/readiness status does not match seal"
        );
        let files = [
            &self.old_state,
            &self.old_blocks,
            &self.old_discovery,
            &self.coverage,
            &self.lifecycle_evidence,
        ];
        for (index, file) in files.iter().enumerate() {
            anyhow::ensure!(
                file.path.is_absolute()
                    && file.digest != H256::zero()
                    && fs::canonicalize(&file.path)? == file.path
                    && files[..index]
                        .iter()
                        .all(|previous| previous.path != file.path),
                "HOLD: invalid/duplicate sealed evidence path"
            );
        }
        anyhow::ensure!(
            self.old_state
                .path
                .file_name()
                .is_some_and(|name| name == "state.json")
                && self
                    .old_blocks
                    .path
                    .file_name()
                    .is_some_and(|name| name == "blocks.json")
                && self
                    .coverage
                    .path
                    .file_name()
                    .is_some_and(|name| name == "cutover-coverage.json")
                && self
                    .lifecycle_evidence
                    .path
                    .file_name()
                    .is_some_and(|name| name == "cutover-calls.json")
                && self.old_state.path.parent() == self.old_blocks.path.parent(),
            "HOLD: original state/block paths disagree"
        );
        Ok(())
    }
}
pub fn load_seal(path: &Path) -> Result<(CutoverSeal, H256)> {
    let bytes = committed_bytes(path)?;
    let seal: CutoverSeal = serde_json::from_slice(&bytes)?;
    seal.validate()?;
    Ok((seal, digest(&bytes)))
}
pub fn verify_seal_files(path: &Path) -> Result<(CutoverSeal, H256)> {
    let (seal, hash) = load_seal(path)?;
    for file in [
        &seal.old_state,
        &seal.old_blocks,
        &seal.old_discovery,
        &seal.coverage,
        &seal.lifecycle_evidence,
    ] {
        anyhow::ensure!(
            digest(&committed_bytes(&file.path)?) == file.digest,
            "HOLD: sealed evidence changed: {}",
            file.path.display()
        );
    }
    Ok((seal, hash))
}
pub fn install_seal_markers(path: &Path) -> Result<H256> {
    let (seal, hash) = verify_seal_files(path)?;
    let marker = CutoverMarker {
        seal_path: fs::canonicalize(path)?,
        manifest_digest: hash,
    };
    immutable_json(
        &marker_path(
            seal.old_state
                .path
                .parent()
                .context("HOLD: missing original directory")?,
        ),
        &marker,
    )?;
    immutable_json(&marker_path(&seal.old_discovery.path), &marker)?;
    Ok(hash)
}
/// Caller must hold original writer ownership until both markers have been committed.
pub fn seal_cutover(path: &Path, seal: &CutoverSeal) -> Result<H256> {
    seal.validate()?;
    for file in [
        &seal.old_state,
        &seal.old_blocks,
        &seal.old_discovery,
        &seal.coverage,
        &seal.lifecycle_evidence,
    ] {
        anyhow::ensure!(
            sealed_file(&file.path)? == *file,
            "HOLD: old evidence changed before sealing"
        );
    }
    immutable_json(path, seal)?;
    install_seal_markers(path)
}
pub fn verify_markers(path: &Path) -> Result<(CutoverSeal, H256)> {
    let (seal, hash) = verify_seal_files(path)?;
    let expected = CutoverMarker {
        seal_path: fs::canonicalize(path)?,
        manifest_digest: hash,
    };
    for journal in [
        seal.old_state
            .path
            .parent()
            .context("HOLD: missing original directory")?,
        seal.old_discovery.path.as_path(),
    ] {
        let marker: CutoverMarker =
            serde_json::from_slice(&committed_bytes(&marker_path(journal))?)?;
        anyhow::ensure!(
            marker == expected,
            "HOLD: old journal markers and cutover seal disagree"
        );
    }
    Ok((seal, hash))
}
pub struct JournalOwnership {
    _transactions: File,
    _discovery: File,
}
fn lock_file(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "HOLD: inbound ownership lock must be a regular file"
    );
    file.try_lock()
        .context("HOLD: another inbound journal writer owns this lane")?;
    Ok(file)
}
pub fn lock_original_journals(transactions: &Path, discovery: &Path) -> Result<JournalOwnership> {
    fs::create_dir_all(transactions)?;
    let parent = discovery
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    if discovery.try_exists()? {
        anyhow::ensure!(
            fs::symlink_metadata(discovery)?.is_file(),
            "HOLD: discovery journal cannot be a symlink"
        );
    }
    let discovery = fs::canonicalize(parent)?.join(
        discovery
            .file_name()
            .context("HOLD: invalid discovery path")?,
    );
    Ok(JournalOwnership {
        _transactions: lock_file(&transactions.join("inbound.lock"))?,
        _discovery: lock_file(&PathBuf::from(format!("{}.lock", discovery.display())))?,
    })
}

pub async fn authenticate_status_pins(seal: &CutoverSeal, api: &GearApi) -> Result<()> {
    anyhow::ensure!(
        api.get_block_hash(0).await? == seal.old_identity.gear_genesis_hash,
        "HOLD: Gear genesis changed"
    );
    let latest = api.last_block_hash().await?;
    let finalized_number = api.block_number_at(latest).await?;
    let manager = VftManager::new(GClientRemoting::new(api.clone()));
    for (identity, pin) in [
        (&seal.old_identity, &seal.retirement),
        (&seal.new_identity, &seal.readiness),
    ] {
        anyhow::ensure!(
            pin.block_number <= finalized_number
                && api.get_block_hash(pin.block_number).await? == pin.block_hash,
            "HOLD: cutover finalized history changed"
        );
        let status = manager
            .cutover_status()
            .at_block(pin.block_hash.0.into())
            .recv(identity.vft_manager_address.0.into())
            .await?;
        anyhow::ensure!(
            status.encode() == pin.status,
            "HOLD: saved cutover status changed"
        );
    }
    let replacement = api
        .program_at(
            seal.new_identity.vft_manager_address.0.into(),
            Some(seal.readiness.block_hash),
        )
        .await?;
    anyhow::ensure!(
        replacement.code_id == seal.new_manager_code_id.0.into()
            && replacement.state == gear_core::program::ProgramState::Initialized,
        "HOLD: pinned replacement CodeId changed"
    );
    for pin in [&seal.retirement, &seal.readiness] {
        anyhow::ensure!(
            api.get_block_hash(pin.block_number).await? == pin.block_hash,
            "HOLD: cutover finalized pin changed during readback"
        );
    }
    let final_head = api.last_block_hash().await?;
    anyhow::ensure!(
        api.block_number_at(final_head).await? >= finalized_number
            && api.get_block_hash(finalized_number).await? == latest,
        "HOLD: Gear finality regressed or changed during cutover readback"
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct OriginalJournal {
    pub schema_version: u32,
    pub runtime_identity: Option<InboundRuntimeIdentity>,
    #[serde(default)]
    pub predecessor_cutover: Option<PredecessorCutover>,
    pub transactions: BTreeMap<Uuid, Transaction>,
    pub completed: BTreeMap<Uuid, Transaction>,
    pub failed: BTreeMap<Uuid, String>,
}
pub fn read_original_journal(path: &Path) -> Result<OriginalJournal> {
    let journal: OriginalJournal =
        serde_json::from_slice(&committed_bytes(&path.join("state.json"))?)?;
    anyhow::ensure!(
        matches!(journal.schema_version, 5..=7)
            && journal.runtime_identity.is_some()
            && (journal.schema_version == 7 || journal.predecessor_cutover.is_none()),
        "HOLD: unsupported original journal identity/schema"
    );
    for (uuid, tx) in journal.transactions.iter().chain(journal.completed.iter()) {
        anyhow::ensure!(
            *uuid == tx.uuid
                && !(journal.transactions.contains_key(uuid)
                    && journal.completed.contains_key(uuid)),
            "HOLD: original receipt UUID ownership changed"
        );
    }
    Ok(journal)
}
async fn verify_original_lane(
    seal: &CutoverSeal,
    provider: ApiProviderConnection,
    api: &GearApi,
) -> Result<()> {
    authenticate_status_pins(seal, api).await?;
    let journal = read_original_journal(
        seal.old_state
            .path
            .parent()
            .context("HOLD: missing original directory")?,
    )?;
    anyhow::ensure!(
        journal.runtime_identity.as_ref() == Some(&seal.old_identity),
        "HOLD: sealed source has the wrong original runtime identity"
    );
    let discovery: DiscoveryState =
        serde_json::from_slice(&committed_bytes(&seal.old_discovery.path)?)?;
    discovery.validate()?;
    anyhow::ensure!(
        discovery.identity == seal.old_identity && discovery.pending.is_empty(),
        "HOLD: original discovery has undrained coverage"
    );
    anyhow::ensure!(
        discovery
            .extracted
            .is_none_or(|cursor| cursor.number <= seal.boundary.execution_number),
        "HOLD: original discovery crossed its authenticated cutover domain"
    );
    let blocks: BTreeMap<
        crate::message_relayer::common::EthereumSlotNumber,
        super::storage::Block,
    > = serde_json::from_slice(&committed_bytes(&seal.old_blocks.path)?)?;
    let sender = MessageSender::new(
        seal.old_identity.vft_manager_address,
        vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec(),
        seal.old_identity.historical_proxy_address,
        provider.clone(),
        String::new(),
        seal.old_identity.erc20_manager_address,
    );
    let coverage: ExternalCoverageReport =
        serde_json::from_slice(&committed_bytes(&seal.coverage.path)?)?;
    anyhow::ensure!(
        coverage.schema_version == 1,
        "HOLD: unsupported external coverage schema"
    );
    let (unsigned, fulfilled_handoffs) =
        verify_external_coverage(seal, &journal, &coverage, &sender, api).await?;
    anyhow::ensure!(
        journal
            .transactions
            .keys()
            .all(|uuid| unsigned.contains(uuid))
            && journal.failed.keys().all(|uuid| unsigned.contains(uuid)),
        "HOLD: sealed source retains an owned/unknown original or unqualified unsigned admission"
    );
    for (slot, block) in &blocks {
        anyhow::ensure!(
            slot.0 >= seal.source_receipt_floor
                && slot.0 <= seal.boundary.slot
                && block.hash.is_some_and(|hash| hash != H256::zero()),
            "HOLD: original block handoff domain changed"
        );
        for tx_hash in &block.transactions {
            anyhow::ensure!(
                fulfilled_handoffs.get(&(slot.0, *tx_hash))
                    == Some(&(block.number.0, block.hash.expect("validated hash"))),
                "HOLD: original block handoff has no authenticated external fulfillment"
            );
        }
    }
    for transaction in journal.completed.values() {
        let payload = transaction.receipt_event()?;
        let receipt = transaction
            .receipt
            .as_ref()
            .context("HOLD: original receipt absent")?;
        anyhow::ensure!(
            matches!(transaction.status, super::tx_manager::TxStatus::Completed)
                && receipt.receipt_key.0 >= seal.source_receipt_floor
                && receipt.receipt_key.0 <= seal.boundary.slot,
            "HOLD: completed receipt lies outside its original deployment domain"
        );
        for attempt in &receipt.submission_attempts {
            verify_archived_proof(
                &sender,
                &provider,
                &seal.old_identity,
                &payload,
                &attempt.signed_submission,
            )
            .await?;
        }
        let signed = receipt
            .signed_submission
            .as_ref()
            .context("HOLD: original signature absent")?;
        let pin = transaction
            .saved_settlement_observation
            .as_ref()
            .or(transaction.receipt_observation.as_ref())
            .context("HOLD: original settlement pin absent")?;
        anyhow::ensure!(
            pin.finalized_block_number
                .is_some_and(|number| number <= seal.retirement.block_number),
            "HOLD: original settlement pin follows source retirement"
        );
        let account = subxt::utils::AccountId32::from(seal.old_identity.gear_sender);
        anyhow::ensure!(
            signed.sender == account.to_string(),
            "HOLD: original signed sender changed"
        );
        sender
            .verify_saved_completion(api, &payload, signed, pin)
            .await?;
    }
    // No load_state/save, signing, rebroadcast or reconciliation continuation occurs here.
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuccessorInitialization {
    predecessor_cutover: PredecessorCutover,
    identity: InboundRuntimeIdentity,
    discovery_path: PathBuf,
    seal_path: PathBuf,
}

/// A later cutover keeps the original predecessor identity and exact previous seal, not replay keys.
pub fn predecessor_seal_path(transactions: &Path) -> Result<Option<PathBuf>> {
    let journal = read_original_journal(transactions)?;
    let Some(predecessor) = journal.predecessor_cutover else {
        return Ok(None);
    };
    let intent: SuccessorInitialization = serde_json::from_slice(&committed_bytes(
        &transactions.join(".cutover-initialization"),
    )?)?;
    anyhow::ensure!(
        intent.predecessor_cutover == predecessor
            && Some(&intent.identity) == journal.runtime_identity.as_ref(),
        "HOLD: original successor initialization identity changed"
    );
    let (seal, hash) = verify_markers(&intent.seal_path)?;
    anyhow::ensure!(
        hash == predecessor.manifest_digest
            && seal.old_identity == predecessor.predecessor_identity
            && Some(&seal.new_identity) == journal.runtime_identity.as_ref(),
        "HOLD: predecessor seal identity changed"
    );
    Ok(Some(intent.seal_path))
}

pub async fn verify_originals_read_only(
    path: &Path,
    provider: ApiProviderConnection,
    api: &GearApi,
) -> Result<()> {
    let mut path = fs::canonicalize(path)?;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let (seal, hash) = verify_markers(&path)?;
        anyhow::ensure!(
            seen.insert(hash),
            "HOLD: cyclic predecessor cutover manifests"
        );
        verify_original_lane(&seal, provider.clone(), api).await?;
        let predecessor = predecessor_seal_path(
            seal.old_state
                .path
                .parent()
                .context("HOLD: missing original directory")?,
        )?;
        verify_markers(&path)?;
        let Some(previous) = predecessor else {
            return Ok(());
        };
        let (prior, _) = verify_markers(&previous)?;
        anyhow::ensure!(
            seal.source_receipt_floor
                == prior
                    .boundary
                    .slot
                    .checked_add(1)
                    .context("HOLD: predecessor boundary overflow")?
                && decode_status(&seal.retirement)?.predecessor
                    == Some(prior.old_identity.vft_manager_address.0.into()),
            "HOLD: later cutover changed its original predecessor/floor"
        );
        path = previous;
    }
}
/// Initialize only an unused successor pair; every empty snapshot commit is resumable.
pub async fn initialize_successor(
    path: &Path,
    transactions: &Path,
    discovery: &Path,
) -> Result<PredecessorCutover> {
    let (seal, hash) = verify_markers(path)?;
    let _owner = lock_original_journals(transactions, discovery)?;
    reject_sealed(transactions)?;
    reject_sealed(discovery)?;
    let transactions = fs::canonicalize(transactions)?;
    let parent = discovery
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let discovery = fs::canonicalize(parent)?.join(
        discovery
            .file_name()
            .context("HOLD: invalid successor discovery path")?,
    );
    anyhow::ensure!(
        transactions
            != seal
                .old_state
                .path
                .parent()
                .context("HOLD: missing old directory")?
            && discovery != seal.old_discovery.path,
        "HOLD: successor cannot reuse original journals"
    );
    let predecessor = seal.predecessor(hash);
    let intent_path = transactions.join(".cutover-initialization");
    anyhow::ensure!(
        ![
            "state.json",
            "blocks.json",
            "inbound.lock",
            ".cutover-initialization"
        ]
        .iter()
        .any(|name| discovery == transactions.join(name)),
        "HOLD: discovery path conflicts with successor transaction evidence"
    );
    let discovery_lock = PathBuf::from(format!("{}.lock", discovery.display()));
    if !intent_path.try_exists()? {
        for entry in fs::read_dir(&transactions)? {
            let entry = entry?;
            let name = entry.file_name();
            anyhow::ensure!(
                name == "inbound.lock"
                    || name == ".cutover-initialization.new"
                    || entry.path() == discovery_lock,
                "HOLD: successor transaction path is not new and unused"
            );
        }
        anyhow::ensure!(
            !discovery.try_exists()?
                && !discovery.with_extension("tmp").try_exists()?
                && !PathBuf::from(format!("{}.new", discovery.display())).try_exists()?,
            "HOLD: successor discovery path is not new and unused"
        );
    }
    immutable_json(
        &intent_path,
        &SuccessorInitialization {
            predecessor_cutover: predecessor.clone(),
            identity: seal.new_identity.clone(),
            discovery_path: discovery.clone(),
            seal_path: fs::canonicalize(path)?,
        },
    )?;
    let empty =
        BTreeMap::<crate::message_relayer::common::EthereumSlotNumber, super::storage::Block>::new(
        );
    immutable_json(&transactions.join("blocks.json"), &empty)?;
    immutable_json(
        &transactions.join("state.json"),
        &OriginalJournal {
            schema_version: 7,
            runtime_identity: Some(seal.new_identity.clone()),
            predecessor_cutover: Some(predecessor.clone()),
            transactions: BTreeMap::new(),
            completed: BTreeMap::new(),
            failed: BTreeMap::new(),
        },
    )?;
    let initial = DiscoveryState {
        schema_version: 2,
        identity: seal.new_identity.clone(),
        predecessor_cutover: Some(predecessor.clone()),
        start_parent_hash: Some(seal.boundary.execution_hash),
        discovered: None,
        extracted: None,
        pending: BTreeMap::new(),
    };
    immutable_json(&discovery, &initial)?;
    super::storage::JSONStorage::new(&transactions)
        .bind_successor_identity(seal.new_identity, predecessor.clone())
        .await?;
    Ok(predecessor)
}

/// Transport endpoints may change; the approved replacement, seal and both journal bindings may not.
pub async fn authenticate_startup(
    identity: &InboundRuntimeIdentity,
    seal_path: Option<&Path>,
    approved_code: Option<H256>,
    api: &GearApi,
    eth: &ethereum_client::PollingEthApi,
) -> Result<Option<PredecessorCutover>> {
    let at = api.last_block_hash().await?;
    let at_number = api.block_number_at(at).await?;
    let manager = VftManager::new(GClientRemoting::new(api.clone()));
    let status = manager
        .cutover_status()
        .at_block(at.0.into())
        .recv(identity.vft_manager_address.0.into())
        .await?;
    let predecessor = if let Some(path) = seal_path {
        let (seal, hash) = verify_markers(path)?;
        anyhow::ensure!(
            &seal.new_identity == identity && approved_code == Some(seal.new_manager_code_id),
            "HOLD: successor runtime/code does not match independently approved cutover"
        );
        authenticate_status_pins(&seal, api).await?;
        anyhow::ensure!(
            eth.chain_id().await? == identity.ethereum_chain_id
                && H256(eth.get_block(0).await?.header.hash.0) == identity.ethereum_genesis_hash
                && eth
                    .is_finalized_block(
                        seal.boundary.execution_number,
                        seal.boundary.execution_hash
                    )
                    .await?,
            "HOLD: cutover boundary is not canonical finalized Ethereum history"
        );
        let boundary = eth.get_block(seal.boundary.execution_number).await?;
        anyhow::ensure!(
            H256(boundary.header.hash.0) == seal.boundary.execution_hash
                && boundary.header.inner.hash_slow() == boundary.header.hash,
            "HOLD: authenticated cutover execution header changed"
        );
        let program = api
            .program_at(identity.vft_manager_address.0.into(), Some(at))
            .await?;
        anyhow::ensure!(
            program.code_id == seal.new_manager_code_id.0.into()
                && program.state == gear_core::program::ProgramState::Initialized,
            "HOLD: replacement program/code is not the independently approved immutable program"
        );
        let old = manager
            .cutover_status()
            .at_block(at.0.into())
            .recv(seal.old_identity.vft_manager_address.0.into())
            .await?;
        anyhow::ensure!(
            old.encode() == seal.retirement.status,
            "HOLD: retired predecessor status changed"
        );
        anyhow::ensure!(
            matches!(
                status.phase,
                CutoverPhase::Ready | CutoverPhase::Active | CutoverPhase::Draining
            ) && status.predecessor == Some(seal.old_identity.vft_manager_address.0.into())
                && status.minimum_receipt_slot
                    == seal
                        .boundary
                        .slot
                        .checked_add(1)
                        .context("HOLD: cutover floor overflow")?
                && (status.phase != CutoverPhase::Ready
                    || status.config_fingerprint == Some(seal.config_fingerprint.0.into())),
            "HOLD: successor deployment domain or activation phase changed"
        );
        Some(seal.predecessor(hash))
    } else {
        anyhow::ensure!(approved_code.is_none() && status.predecessor.is_none()
            && status.minimum_receipt_slot == 0 && matches!(status.phase,CutoverPhase::Active|CutoverPhase::Draining),
            "HOLD: replacement lane requires its immutable cutover seal and independent code approval");
        None
    };
    let finalized = api.last_block_hash().await?;
    anyhow::ensure!(
        api.block_number_at(finalized).await? >= at_number
            && api.get_block_hash(at_number).await? == at,
        "HOLD: successor startup finalized pin changed during authentication"
    );
    Ok(predecessor)
}

pub async fn ensure_receipt_domain(api: &GearApi, receiver: H256, slot: u64) -> Result<()> {
    let at = api.last_block_hash().await?;
    let status = VftManager::new(GClientRemoting::new(api.clone()))
        .cutover_status()
        .at_block(at.0.into())
        .recv(receiver.0.into())
        .await?;
    anyhow::ensure!(
        matches!(status.phase, CutoverPhase::Active | CutoverPhase::Draining)
            && slot >= status.minimum_receipt_slot
            && status.last_receipt_slot.is_none_or(|last| slot <= last),
        "HOLD: receipt is outside this manager deployment domain or receiver is not active"
    );
    Ok(())
}

async fn verify_archived_proof(
    sender: &MessageSender,
    provider: &ApiProviderConnection,
    identity: &InboundRuntimeIdentity,
    payload: &eth_events_electra_client::EthToVaraEvent,
    original: &super::message_sender::SignedSubmission,
) -> Result<()> {
    use historical_proxy_client::historical_proxy::io::Redirect;
    use std::str::FromStr;
    let chain = provider.client();
    let account = subxt::utils::AccountId32::from(identity.gear_sender);
    anyhow::ensure!(
        original.sender == account.to_string()
            && original.chain_genesis_hash == format!("{:#x}", identity.gear_genesis_hash)
            && original.manager == format!("{:#x}", identity.vft_manager_address)
            && original.historical_proxy == format!("{:#x}", identity.historical_proxy_address)
            && original.receipt_key == (payload.proof_block.block.slot, payload.transaction_index)
            && original.payload_hash
                == format!("{:#x}", alloy_primitives::keccak256(payload.encode())),
        "HOLD: archived original proof identity changed"
    );
    original.validate_native_reconciliation()?;
    for continuation in original
        .native_reconciliation_attempts
        .iter()
        .chain(original.native_reconciliation.as_deref())
    {
        sender
            .authenticate_archived_continuation(continuation)
            .await?;
    }
    let call = Redirect::encode_call(
        payload.proof_block.block.slot,
        payload.encode(),
        identity.vft_manager_address.0.into(),
        vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec(),
    );
    let mut observed = original.clone();
    if original.inclusion_block_number.is_none() {
        observed.scanned_finalized_number = original.prepared_finalized_number;
        observed.scanned_finalized_hash = original.prepared_finalized_hash.clone();
    }
    let included = sender
        .find_signed_extrinsic(&mut observed, identity.historical_proxy_address, &call)
        .await?;
    if !included {
        let hash = H256::from_str(&original.scanned_finalized_hash)?;
        let latest = chain
            .block_hash_to_number(chain.latest_finalized_block().await?)
            .await?;
        anyhow::ensure!(
            original.scanned_finalized_number <= latest
                && chain
                    .block_number_to_hash(original.scanned_finalized_number)
                    .await?
                    == hash
                && u64::from(chain.api.info_at(&original.sender, Some(hash)).await?.nonce)
                    > original.nonce,
            "HOLD: archived absent original has no canonical consumed-nonce disposition"
        );
    } else {
        anyhow::ensure!(
            observed.inclusion_block_number == original.inclusion_block_number
                && observed.inclusion_block_hash == original.inclusion_block_hash
                && observed.message_id == original.message_id
                && observed.finalized_dispatch_error == original.finalized_dispatch_error,
            "HOLD: archived original dispatch changed"
        );
        if let Some(id) = original.message_id.as_deref() {
            sender
                .find_finalized_reply(
                    &mut observed,
                    H256::from_str(id)?,
                    identity.historical_proxy_address,
                )
                .await?
                .context("HOLD: archived original reply unavailable")?;
            anyhow::ensure!(
                observed.finalized_reply == original.finalized_reply
                    && observed.finalized_reply_error == original.finalized_reply_error,
                "HOLD: archived original reply changed"
            );
        } else {
            anyhow::ensure!(
                original.finalized_dispatch_error.is_some(),
                "HOLD: archived original has an unknown outcome"
            );
        }
    }
    Ok(())
}

pub async fn receipt_domain(api: &GearApi, receiver: H256) -> Result<(u64, Option<u64>)> {
    let at = api.last_block_hash().await?;
    let height = api.block_number_at(at).await?;
    let status = VftManager::new(GClientRemoting::new(api.clone()))
        .cutover_status()
        .at_block(at.0.into())
        .recv(receiver.0.into())
        .await?;
    let current = api.last_block_hash().await?;
    anyhow::ensure!(
        api.block_number_at(current).await? >= height
            && api.get_block_hash(height).await? == at
            && matches!(status.phase, CutoverPhase::Active | CutoverPhase::Draining),
        "HOLD: receipt domain is not canonical finalized active/draining state"
    );
    Ok((status.minimum_receipt_slot, status.last_receipt_slot))
}

#[cfg(test)]
mod tests {
    use super::{
        super::{
            storage::{JSONStorage, Storage},
            tx_manager::TransactionManager,
        },
        *,
    };
    use crate::message_relayer::common::ethereum::block_storage::{
        tests::identity, BlockCursor, JSONBlockStorage,
    };
    use gear_common::signer_ownership::{lock_cutover_signer, lock_signer, release_cutover_signer};
    use std::sync::Arc;

    #[test]
    fn external_original_link_never_replaces_signature_handoff_or_claimed_completion() {
        use super::super::{
            message_sender::{SignedSubmission, SubmissionEvidence},
            tx_manager::{ReceiptEvidence, TxStatus},
        };
        use crate::message_relayer::common::{EthereumSlotNumber, TxHashWithSlot};
        let mut transaction = Transaction::new(
            TxHashWithSlot {
                slot_number: EthereumSlotNumber(32),
                tx_hash: alloy_primitives::B256::repeat_byte(1),
            },
            TxStatus::ComposeProof,
        );
        assert!(is_undispatched_old_admission(&transaction));
        transaction.status = TxStatus::PreparedSubmission;
        assert!(!is_undispatched_old_admission(&transaction));
        transaction.status = TxStatus::Completed;
        assert!(!is_undispatched_old_admission(&transaction));
        transaction.status = TxStatus::SubmitMessage;
        transaction.receipt = Some(ReceiptEvidence {
            payload: Vec::new(),
            receipt_key: (32, 0),
            composed_at_ms: 0,
            handed_off_at_ms: None,
            initial_response: None,
            signed_submission: None,
            submission_attempts: Vec::new(),
        });
        assert!(is_undispatched_old_admission(&transaction));
        transaction.receipt.as_mut().unwrap().handed_off_at_ms = Some(0);
        assert!(!is_undispatched_old_admission(&transaction));
        transaction.receipt.as_mut().unwrap().handed_off_at_ms = None;
        transaction.receipt.as_mut().unwrap().initial_response = Some(SubmissionEvidence {
            observed_at_ms: 0,
            manager: H256::zero(),
            historical_proxy: H256::zero(),
            sender: String::new(),
            manager_reply: None,
            error: Some("known original hash without raw bytes".into()),
        });
        assert!(!is_undispatched_old_admission(&transaction));
        transaction.receipt.as_mut().unwrap().initial_response = None;
        let signed: SignedSubmission = serde_json::from_value(serde_json::json!({
            "chain_genesis_hash":"original","manager":"original","historical_proxy":"original","sender":"original",
            "receipt_key":[32,0],"payload_hash":"original","nonce":1,"extrinsic_hash":"original","raw_extrinsic":[],
            "prepared_finalized_number":1,"prepared_finalized_hash":"original","scanned_finalized_number":1,"scanned_finalized_hash":"original",
            "reply_scanned_finalized_number":1,"reply_scanned_finalized_hash":"original","inclusion_block_number":null,"inclusion_block_hash":null,"message_id":null
        })).unwrap();
        transaction.receipt.as_mut().unwrap().signed_submission = Some(signed.clone());
        assert!(!is_undispatched_old_admission(&transaction));
        transaction.receipt.as_mut().unwrap().signed_submission = None;
        transaction
            .receipt
            .as_mut()
            .unwrap()
            .submission_attempts
            .push(super::super::tx_manager::SubmissionAttempt {
                signed_submission: signed,
                response: None,
            });
        assert!(!is_undispatched_old_admission(&transaction));
    }

    fn fixture_seal(base: &Path, old: InboundRuntimeIdentity) -> CutoverSeal {
        let mut new = old.clone();
        new.vft_manager_address = H256::repeat_byte(8);
        new.ethereum_start_block = 101;
        let fingerprint = H256::repeat_byte(9);
        let retirement = CutoverStatus {
            phase: CutoverPhase::Retired,
            predecessor: None,
            successor: Some(new.vft_manager_address.0.into()),
            minimum_receipt_slot: 0,
            last_receipt_slot: Some(32),
            config_fingerprint: Some(fingerprint.0.into()),
            transfer_count: 1,
            completed_transfer_count: 1,
            in_flight_transfer_count: 0,
        };
        let readiness = CutoverStatus {
            phase: CutoverPhase::Ready,
            predecessor: Some(old.vft_manager_address.0.into()),
            successor: None,
            minimum_receipt_slot: 33,
            last_receipt_slot: None,
            config_fingerprint: Some(fingerprint.0.into()),
            transfer_count: 1,
            completed_transfer_count: 0,
            in_flight_transfer_count: 0,
        };
        CutoverSeal {
            schema_version: 1,
            old_identity: old,
            new_identity: new,
            new_manager_code_id: H256::repeat_byte(10),
            source_receipt_floor: 0,
            boundary: CutoverBoundary {
                slot: 32,
                beacon_root: H256::repeat_byte(11),
                execution_number: 100,
                execution_hash: H256::repeat_byte(12),
            },
            config_fingerprint: fingerprint,
            old_state: sealed_file(&base.join("old/state.json")).unwrap(),
            old_blocks: sealed_file(&base.join("old/blocks.json")).unwrap(),
            old_discovery: sealed_file(&base.join("old-discovery.json")).unwrap(),
            coverage: sealed_file(&base.join("cutover-coverage.json")).unwrap(),
            lifecycle_evidence: sealed_file(&base.join("cutover-calls.json")).unwrap(),
            retirement: FinalizedCutoverStatus {
                block_number: 20,
                block_hash: H256::repeat_byte(13),
                status: retirement.encode(),
            },
            readiness: FinalizedCutoverStatus {
                block_number: 21,
                block_hash: H256::repeat_byte(14),
                status: readiness.encode(),
            },
        }
    }

    #[tokio::test]
    async fn detached_writer_handles_keep_ownership_after_parent_drops() {
        let base = std::env::temp_dir().join(format!("cutover-writers-{}", Uuid::now_v7()));
        let transactions = base.join("transactions");
        let discovery_path = base.join("discovery.json");
        let parent = Arc::new(lock_original_journals(&transactions, &discovery_path).unwrap());
        let storage = JSONStorage::new(&transactions).with_journal_ownership(parent.clone());
        let discovery = JSONBlockStorage::new(discovery_path.clone(), identity())
            .await
            .unwrap()
            .with_journal_ownership(parent.clone());
        drop(parent);
        assert!(lock_original_journals(&transactions, &discovery_path).is_err());
        drop(storage);
        assert!(lock_original_journals(&transactions, &discovery_path).is_err());
        drop(discovery);
        let next = lock_original_journals(&transactions, &discovery_path).unwrap();
        drop(next);
        fs::remove_dir_all(base).unwrap();
    }
    #[tokio::test]
    async fn original_seal_is_read_only_and_successor_starts_at_exact_boundary() {
        let base = std::env::temp_dir().join(format!("cutover-seal-{}", Uuid::now_v7()));
        fs::create_dir_all(&base).unwrap();
        let old = base.join("old");
        let discovery_path = base.join("old-discovery.json");
        let owner = lock_original_journals(&old, &discovery_path).unwrap();
        let storage = Arc::new(JSONStorage::new(&old));
        storage.bind_runtime_identity(identity()).await.unwrap();
        JSONBlockStorage::new(discovery_path.clone(), identity())
            .await
            .unwrap();
        let mut original_discovery: serde_json::Value =
            serde_json::from_slice(&fs::read(&discovery_path).unwrap()).unwrap();
        original_discovery["schema_version"] = 1.into();
        let discovery_bytes = serde_json::to_vec(&original_discovery).unwrap();
        fs::write(&discovery_path, &discovery_bytes).unwrap();
        JSONBlockStorage::new(discovery_path.clone(), identity())
            .await
            .unwrap();
        assert_eq!(fs::read(&discovery_path).unwrap(), discovery_bytes);
        // Schema5 is an original lane, not silently relabeled as a successor or rewritten by verification.
        let mut state: serde_json::Value =
            serde_json::from_slice(&fs::read(old.join("state.json")).unwrap()).unwrap();
        state["schema_version"] = 5.into();
        fs::write(old.join("state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
        let original = fs::read(old.join("state.json")).unwrap();
        storage.bind_runtime_identity(identity()).await.unwrap();
        assert_eq!(fs::read(old.join("state.json")).unwrap(), original);
        for name in ["cutover-coverage.json", "cutover-calls.json"] {
            fs::write(base.join(name), b"{}").unwrap();
        }
        let seal = fixture_seal(&base, identity());
        let path = base.join("cutover.json");
        let hash = seal_cutover(&path, &seal).unwrap();
        assert_eq!(verify_markers(&path).unwrap().1, hash);
        assert!(reject_sealed(&old).is_err());
        assert!(reject_sealed(&discovery_path).is_err());
        let manager = TransactionManager::new(storage.clone());
        assert!(storage.load(&manager).await.is_err());
        assert!(storage.save(&manager).await.is_err());
        let readonly = read_original_journal(&old).unwrap();
        assert_eq!(readonly.schema_version, 5);
        assert!(readonly.predecessor_cutover.is_none());
        assert_eq!(fs::read(old.join("state.json")).unwrap(), original);
        let next = base.join("new");
        let next_discovery = base.join("new-discovery.json");
        let predecessor = initialize_successor(&path, &next, &next_discovery)
            .await
            .unwrap();
        assert_eq!(predecessor.manifest_digest, hash);
        assert_eq!(
            initialize_successor(&path, &next, &next_discovery)
                .await
                .unwrap(),
            predecessor
        );
        // Crash after owned intent and an empty block snapshot, before state/discovery commits.
        let interrupted = base.join("interrupted");
        let interrupted_discovery = interrupted.join("discovery.json");
        let interrupted_owner =
            lock_original_journals(&interrupted, &interrupted_discovery).unwrap();
        immutable_json(
            &interrupted.join(".cutover-initialization"),
            &SuccessorInitialization {
                predecessor_cutover: predecessor.clone(),
                identity: seal.new_identity.clone(),
                discovery_path: fs::canonicalize(&interrupted)
                    .unwrap()
                    .join("discovery.json"),
                seal_path: fs::canonicalize(&path).unwrap(),
            },
        )
        .unwrap();
        fs::write(interrupted.join("blocks.json.new"), b"{}").unwrap();
        drop(interrupted_owner);
        assert_eq!(
            initialize_successor(&path, &interrupted, &interrupted_discovery)
                .await
                .unwrap(),
            predecessor
        );
        assert_eq!(
            read_original_journal(&interrupted)
                .unwrap()
                .runtime_identity,
            Some(seal.new_identity.clone())
        );
        assert_eq!(
            predecessor_seal_path(&interrupted).unwrap(),
            Some(fs::canonicalize(&path).unwrap())
        );
        let successor = read_original_journal(&next).unwrap();
        assert_eq!(successor.schema_version, 7);
        assert_eq!(successor.predecessor_cutover, Some(predecessor.clone()));
        assert!(JSONStorage::new(&next)
            .bind_runtime_identity(seal.new_identity.clone())
            .await
            .is_err());
        let discovery = JSONBlockStorage::new_bound(
            next_discovery.clone(),
            seal.new_identity.clone(),
            Some(predecessor.clone()),
        )
        .await
        .unwrap();
        assert_eq!(discovery.snapshot().await.schema_version, 2);
        assert_eq!(discovery.snapshot().await.next_block().unwrap(), 101);
        let first = BlockCursor {
            number: 101,
            hash: H256::repeat_byte(15),
        };
        assert!(discovery
            .add_block(first, H256::repeat_byte(16))
            .await
            .is_err());
        discovery
            .add_block(first, seal.boundary.execution_hash)
            .await
            .unwrap();
        assert!(initialize_successor(&path, &next, &next_discovery)
            .await
            .is_err());
        let mut changed = seal.clone();
        changed.boundary.slot += 1;
        assert!(seal_cutover(&path, &changed).is_err());
        fs::write(old.join("state.json"), b"{}").unwrap();
        assert!(verify_seal_files(&path).is_err());
        fs::write(old.join("state.json"), &original).unwrap();
        let marker_path = marker_path(&discovery_path);
        let saved_marker = fs::read(&marker_path).unwrap();
        let mut wrong: CutoverMarker = serde_json::from_slice(&saved_marker).unwrap();
        wrong.manifest_digest = H256::repeat_byte(17);
        fs::write(&marker_path, serde_json::to_vec(&wrong).unwrap()).unwrap();
        assert!(verify_markers(&path).is_err());
        fs::write(&marker_path, saved_marker).unwrap();
        assert_eq!(verify_markers(&path).unwrap().1, hash);
        drop((manager, storage, owner, discovery));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn immutable_seal_recovers_exact_interrupted_commit_but_rejects_changed_intent() {
        let base = std::env::temp_dir().join(format!("cutover-commit-{}", Uuid::now_v7()));
        fs::create_dir_all(&base).unwrap();
        let path = base.join("cutover.json");
        let temp = base.join("cutover.json.new");
        let value = serde_json::json!({"identity":"original"});
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&temp, &bytes).unwrap();
        fs::hard_link(&temp, &path).unwrap();
        immutable_json(&path, &value).unwrap();
        assert!(!temp.exists());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(immutable_json(&path, &serde_json::json!({"identity":"other"})).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn same_account_cannot_sign_from_different_journal_owners() {
        let base = std::env::temp_dir().join(format!("cutover-signers-{}", Uuid::now_v7()));
        fs::create_dir_all(&base).unwrap();
        let first_lane =
            lock_original_journals(&base.join("first"), &base.join("first.json")).unwrap();
        let second_lane =
            lock_original_journals(&base.join("second"), &base.join("second.json")).unwrap();
        let mut sender = [0; 32];
        sender[..16].copy_from_slice(Uuid::now_v7().as_bytes());
        let genesis = H256::repeat_byte(21);
        let first = lock_signer(genesis, sender).unwrap();
        assert!(lock_signer(genesis, sender).is_err());
        let other = lock_signer(H256::repeat_byte(22), sender).unwrap();
        drop(first);
        let second = lock_signer(genesis, sender).unwrap();
        drop((second, other, first_lane, second_lane));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn durable_cutover_owner_blocks_relayer_after_process_restart() {
        let base = std::env::temp_dir().join(format!("cutover-durable-owner-{}", Uuid::now_v7()));
        let first = base.join("first");
        let second = base.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let work = first.join("cutover-work.json");
        let rival = second.join("cutover-work.json");
        fs::write(&work, b"{}").unwrap();
        fs::write(&rival, b"{}").unwrap();
        let mut sender = [0; 32];
        sender[..16].copy_from_slice(Uuid::now_v7().as_bytes());
        let genesis = H256::repeat_byte(23);
        let original = lock_cutover_signer(genesis, sender, &work).unwrap();
        assert!(lock_signer(genesis, sender).is_err());
        drop(original);
        assert!(lock_signer(genesis, sender).is_err());
        assert!(lock_cutover_signer(genesis, sender, &rival).is_err());
        let resumed = lock_cutover_signer(genesis, sender, &work).unwrap();
        assert!(release_cutover_signer(genesis, sender, &rival, &resumed).is_err());
        fs::remove_file(&work).unwrap();
        assert!(release_cutover_signer(genesis, sender, &work, &resumed).is_err());
        fs::write(&work, b"{}").unwrap();
        // No signatures were prepared in this test; the empty original set is terminal.
        release_cutover_signer(genesis, sender, &work, &resumed).unwrap();
        assert!(lock_signer(genesis, sender).is_err());
        drop(resumed);
        let next = lock_signer(genesis, sender).unwrap();
        drop(next);
        fs::remove_dir_all(base).unwrap();
    }
}
