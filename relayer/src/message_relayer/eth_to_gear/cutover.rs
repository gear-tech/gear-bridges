//! Immutable cutover seals. Reading an original lane never restores its mutable runtime.
use super::{
    message_sender::MessageSender, storage::InboundRuntimeIdentity, tx_manager::Transaction,
};
use crate::message_relayer::common::ethereum::block_storage::DiscoveryState;
use alloy::{
    consensus::TxEnvelope,
    eips::eip2718::{Decodable2718, Encodable2718},
    rpc::types::TransactionReceipt,
};
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoveredBlock {
    pub number: u64,
    pub hash: H256,
    pub parent_hash: H256,
    pub receipts_root: H256,
    pub transactions: Vec<alloy_primitives::B256>,
    pub receipts: Vec<TransactionReceipt>,
    pub header: alloy::consensus::Header,
    pub raw_transactions: Vec<Vec<u8>>,
}

pub fn validate_covered_block(block: &CoveredBlock) -> Result<()> {
    anyhow::ensure!(
        block.header.number == block.number
            && block.header.hash_slow().0 == block.hash.0
            && block.header.parent_hash.0 == block.parent_hash.0
            && block.header.receipts_root.0 == block.receipts_root.0,
        "HOLD: saved coverage header identity changed"
    );
    let transactions = block
        .raw_transactions
        .iter()
        .map(|raw| {
            let mut bytes = raw.as_slice();
            let transaction = TxEnvelope::decode_2718(&mut bytes)?;
            anyhow::ensure!(
                bytes.is_empty() && transaction.encoded_2718() == *raw,
                "HOLD: noncanonical coverage transaction"
            );
            Ok(transaction)
        })
        .collect::<Result<Vec<_>>>()?;
    let hashes = block
        .raw_transactions
        .iter()
        .map(alloy_primitives::keccak256)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        hashes == block.transactions
            && alloy::consensus::proofs::calculate_transaction_root(&transactions)
                == block.header.transactions_root,
        "HOLD: coverage transaction inventory/trie differs from authenticated header"
    );
    let mut receipts = block.receipts.clone();
    super::proof_composer::validate_receipt_set(
        block.number,
        block.hash.0.into(),
        &hashes,
        block.receipts_root.0.into(),
        &mut receipts,
    )?;
    Ok(())
}

// Shared projection keeps canonical transaction coverage and terminal evidence at every reader.
#[derive(Deserialize)]
struct ExternalCoverageReport {
    schema_version: u32,
    #[serde(default)]
    blocks: Vec<CoveredBlock>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exclusion: Option<super::message_sender::FinalizedReceiptObservation>,
    #[serde(default)]
    fee_emitter: Option<primitive_types::H160>,
    #[serde(default)]
    fee_event_observed: bool,
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

fn validate_report_versions(
    version: u32,
    coverage: &serde_json::Value,
    lifecycle: &serde_json::Value,
) -> Result<()> {
    fn has_field(value: &serde_json::Value, name: &str) -> bool {
        match value {
            serde_json::Value::Object(fields) => {
                fields.contains_key(name) || fields.values().any(|value| has_field(value, name))
            }
            serde_json::Value::Array(values) => values.iter().any(|value| has_field(value, name)),
            _ => false,
        }
    }
    anyhow::ensure!(
        matches!(version, 1 | 2)
            && coverage
                .get("schema_version")
                .and_then(|value| value.as_u64())
                == Some(version.into())
            && lifecycle
                .get("schema_version")
                .and_then(|value| value.as_u64())
                == Some(version.into())
            && (version == 2
                || (!has_field(coverage, "exclusion")
                    && !has_field(coverage, "blocking_redemptions")
                    && !has_field(lifecycle, "exclusion")
                    && !has_field(lifecycle, "blocking_redemptions"))),
        "HOLD: seal/report schema disagreement or v2 evidence smuggled into v1"
    );
    Ok(())
}

fn read_coverage(seal: &CutoverSeal) -> Result<ExternalCoverageReport> {
    let coverage = serde_json::from_slice(&committed_bytes(&seal.coverage.path)?)?;
    let lifecycle = serde_json::from_slice(&committed_bytes(&seal.lifecycle_evidence.path)?)?;
    validate_report_versions(seal.schema_version, &coverage, &lifecycle)?;
    Ok(serde_json::from_value(coverage)?)
}

fn validate_coverage_chain(seal: &CutoverSeal, blocks: &[CoveredBlock]) -> Result<()> {
    anyhow::ensure!(
        blocks.first().is_some_and(|block| {
            block.number == 0
                && block.hash == seal.old_identity.ethereum_genesis_hash
                && block.parent_hash == H256::zero()
        }) && blocks.last().is_some_and(|block| {
            block.number == seal.boundary.execution_number
                && block.hash == seal.boundary.execution_hash
        }),
        "HOLD: canonical coverage does not span the full source boundary"
    );
    for (index, block) in blocks.iter().enumerate() {
        validate_covered_block(block)?;
        if let Some(previous) = index.checked_sub(1).map(|index| &blocks[index]) {
            anyhow::ensure!(
                previous.number.checked_add(1) == Some(block.number)
                    && previous.hash == block.parent_hash,
                "HOLD: canonical coverage has a gap or conflicting parent"
            );
        }
    }
    Ok(())
}

fn validate_admission_block(
    coverage: &ExternalCoverageReport,
    admission: &ExternalCoverageAdmission,
    event: &eth_events_electra_client::EthToVaraEvent,
) -> Result<()> {
    use ethereum_common::trie_db::{HashDB, Trie};
    let execution = &event.proof_block.block.body.execution_payload;
    let block = coverage
        .blocks
        .iter()
        .find(|block| block.number == execution.block_number)
        .context("HOLD: admission has no authenticated covered block")?;
    let index = usize::try_from(event.transaction_index)?;
    let receipt = block
        .receipts
        .iter()
        .find(|receipt| receipt.transaction_index == Some(event.transaction_index))
        .context("HOLD: admission receipt index is outside block")?;
    let expected = ethereum_common::utils::map_receipt_envelope(receipt.as_ref());
    anyhow::ensure!(
        block.hash == admission.original_block_hash
            && execution.block_hash.0 .0 == block.hash.0
            && execution.parent_hash.0 .0 == block.parent_hash.0
            && execution.receipts_root.0 .0 == block.receipts_root.0
            && block.transactions.get(index) == Some(&admission.original_tx_hash)
            && event.receipt_rlp == alloy_rlp::encode(&expected),
        "HOLD: admission proof/header/receipt/index/transaction hash linkage changed"
    );
    let mut db = ethereum_common::memory_db::new();
    for node in &event.proof {
        db.insert(ethereum_common::hash_db::EMPTY_PREFIX, node);
    }
    let root = ethereum_common::Hash256::from(block.receipts_root.0);
    let trie = ethereum_common::patricia_trie::TrieDB::new(&db, &root)
        .map_err(|_| anyhow::anyhow!("HOLD: admission receipt proof has the wrong root"))?;
    let (key, value) =
        ethereum_common::utils::rlp_encode_index_and_receipt(&event.transaction_index, &expected);
    anyhow::ensure!(
        matches!(trie.get(&key), Ok(Some(found)) if found == value),
        "HOLD: admission receipt proof does not authenticate the selected covered receipt"
    );
    Ok(())
}

/// A cutover copy may add observations, never replace an original request or saved disposition.
pub fn validate_exclusion_copy(
    original: &super::message_sender::SignedSubmission,
    copy: &super::message_sender::SignedSubmission,
) -> Result<()> {
    anyhow::ensure!(
        original.same_request(copy)
            && original
                .inclusion_block_number
                .is_none_or(|value| copy.inclusion_block_number == Some(value))
            && original
                .inclusion_block_hash
                .as_ref()
                .is_none_or(|value| copy.inclusion_block_hash.as_ref() == Some(value))
            && original
                .message_id
                .as_ref()
                .is_none_or(|value| copy.message_id.as_ref() == Some(value))
            && original
                .finalized_dispatch_error
                .as_ref()
                .is_none_or(|value| copy.finalized_dispatch_error.as_ref() == Some(value))
            && original
                .finalized_reply
                .as_ref()
                .is_none_or(|value| copy.finalized_reply.as_ref() == Some(value))
            && original
                .finalized_reply_error
                .as_ref()
                .is_none_or(|value| copy.finalized_reply_error.as_ref() == Some(value))
            && original
                .finalized_nonce_consumed
                .is_none_or(|value| copy.finalized_nonce_consumed == Some(value))
            && original.scanned_finalized_number <= copy.scanned_finalized_number
            && (original.scanned_finalized_number != copy.scanned_finalized_number
                || original.scanned_finalized_hash == copy.scanned_finalized_hash)
            && original.reply_scanned_finalized_number <= copy.reply_scanned_finalized_number
            && (original.reply_scanned_finalized_number != copy.reply_scanned_finalized_number
                || original.reply_scanned_finalized_hash == copy.reply_scanned_finalized_hash)
            && original.native_reconciliation_attempts.len()
                == copy.native_reconciliation_attempts.len()
            && original.native_reconciliation.is_some() == copy.native_reconciliation.is_some(),
        "HOLD: exclusion copy changed retained original request or observations"
    );
    for (original, copy) in original
        .native_reconciliation_attempts
        .iter()
        .chain(original.native_reconciliation.as_deref())
        .zip(
            copy.native_reconciliation_attempts
                .iter()
                .chain(copy.native_reconciliation.as_deref()),
        )
    {
        validate_exclusion_copy(original, copy)?;
    }
    Ok(())
}

async fn authenticate_retained_checkpoints(
    provider: &ApiProviderConnection,
    original: &super::message_sender::SignedSubmission,
    maximum: u32,
) -> Result<()> {
    // A work copy can advance a cursor, but cannot hide an already-saved noncanonical pin.
    let chain = provider.client();
    for original in std::iter::once(original)
        .chain(original.native_reconciliation_attempts.iter())
        .chain(original.native_reconciliation.as_deref())
    {
        for (number, hash) in [
            (
                original.prepared_finalized_number,
                original.prepared_finalized_hash.as_str(),
            ),
            (
                original.scanned_finalized_number,
                original.scanned_finalized_hash.as_str(),
            ),
            (
                original.reply_scanned_finalized_number,
                original.reply_scanned_finalized_hash.as_str(),
            ),
        ]
        .into_iter()
        .chain(
            original
                .inclusion_block_number
                .zip(original.inclusion_block_hash.as_deref()),
        )
        .chain(
            original
                .finalized_reply
                .as_ref()
                .map(|reply| {
                    Ok::<_, anyhow::Error>((
                        reply
                            .observation
                            .finalized_block_number
                            .context("HOLD: retained reply height absent")?,
                        reply.observation.finalized_block_hash.as_str(),
                    ))
                })
                .transpose()?,
        ) {
            let hash: H256 = hash.parse()?;
            anyhow::ensure!(
                number <= maximum
                    && hash != H256::zero()
                    && chain.block_number_to_hash(number).await? == hash,
                "HOLD: retained original observation is noncanonical or follows exclusion"
            );
        }
        anyhow::ensure!(
            original.inclusion_block_number.is_some() == original.inclusion_block_hash.is_some(),
            "HOLD: retained original inclusion is incomplete"
        );
    }
    Ok(())
}

#[derive(Default)]
struct AuthenticatedCoverage {
    unsigned: std::collections::BTreeSet<Uuid>,
    excluded_owned: std::collections::BTreeSet<Uuid>,
    fulfilled_handoffs: BTreeMap<(u64, ethereum_client::TxHash), (u64, H256)>,
    accounted_handoffs: BTreeMap<(u64, ethereum_client::TxHash), (u64, H256)>,
}

impl AuthenticatedCoverage {
    fn accounts_for_handoff(
        &self,
        slot: u64,
        tx: ethereum_client::TxHash,
        number: u64,
        hash: H256,
    ) -> bool {
        self.fulfilled_handoffs
            .get(&(slot, tx))
            .or_else(|| self.accounted_handoffs.get(&(slot, tx)))
            == Some(&(number, hash))
    }
}

fn validate_admission_inventory(coverage: &ExternalCoverageReport) -> Result<()> {
    anyhow::ensure!(
        matches!(coverage.schema_version, 1 | 2)
            && (coverage.schema_version == 2
                || coverage
                    .admissions
                    .iter()
                    .all(|row| row.exclusion.is_none())),
        "HOLD: incompatible coverage/exclusion schema"
    );
    let mut receipts = std::collections::BTreeSet::new();
    let mut transactions = std::collections::BTreeSet::new();
    let mut ownership = std::collections::BTreeSet::new();
    for admission in &coverage.admissions {
        anyhow::ensure!(
            receipts.insert(admission.receipt_key)
                && transactions.insert(admission.original_tx_hash)
                && admission
                    .original_lane_uuid
                    .into_iter()
                    .chain(admission.undispatched_old_admission)
                    .all(|uuid| ownership.insert(uuid))
                && !(admission.original_lane_uuid.is_some()
                    && admission.undispatched_old_admission.is_some())
                && (admission.settlement.is_some() ^ admission.exclusion.is_some()),
            "HOLD: duplicate receipt/transaction/ownership or ambiguous terminal coverage"
        );
    }
    Ok(())
}

async fn verify_external_coverage(
    seal: &CutoverSeal,
    journal: &OriginalJournal,
    coverage: &ExternalCoverageReport,
    sender: &MessageSender,
    api: &GearApi,
) -> Result<AuthenticatedCoverage> {
    anyhow::ensure!(
        coverage.schema_version == seal.schema_version,
        "HOLD: incompatible coverage/seal schema"
    );
    validate_admission_inventory(coverage)?;
    if coverage.schema_version == 2 {
        validate_coverage_chain(seal, &coverage.blocks)?;
    }
    let mut authenticated = AuthenticatedCoverage::default();
    let head = api.last_block_hash().await?;
    let finalized = api.block_number_at(head).await?;
    for admission in &coverage.admissions {
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
            "HOLD: proof/key/block is outside original deployment coverage"
        );
        if coverage.schema_version == 2 {
            validate_admission_block(coverage, admission, &event)?;
            anyhow::ensure!(
                admission.fee_event_observed == admission.fee_emitter.is_some(),
                "HOLD: fee evidence changed"
            );
        }
        let pin = admission
            .exclusion
            .as_ref()
            .or(admission.settlement.as_ref())
            .context("HOLD: terminal observation absent")?;
        anyhow::ensure!(
            pin.finalized_block_number
                .is_some_and(|number| number <= seal.retirement.block_number),
            "HOLD: terminal observation follows source retirement"
        );
        if let Some(external) = &admission.external_original {
            anyhow::ensure!(
                external.provenance == "canonical-external"
                    && admission.admission
                        == if admission.exclusion.is_some() {
                            "externally-resolved"
                        } else {
                            "externally-delivered"
                        }
                    && admission.signed.is_none()
                    && admission.attempts.is_empty()
                    && admission.original_lane_uuid.is_none(),
                "HOLD: external provenance is mixed with owned originals"
            );
            // A known owned signature always wins; a foreign original cannot erase its nonce.
            for old in journal
                .transactions
                .values()
                .chain(journal.completed.values())
            {
                if old.tx.tx_hash == admission.original_tx_hash
                    || old
                        .receipt
                        .as_ref()
                        .is_some_and(|receipt| receipt.receipt_key == admission.receipt_key)
                {
                    anyhow::ensure!(
                        is_undispatched_old_admission(old),
                        "HOLD: external original attempts to replace a retained owned signature"
                    );
                }
            }
            let number = external
                .discovered_at
                .finalized_block_number
                .context("HOLD: external discovery has no finalized height")?;
            let hash = external
                .discovered_at
                .finalized_block_hash
                .parse::<H256>()?;
            anyhow::ensure!(
                hash != H256::zero()
                    && number <= finalized
                    && api.get_block_hash(number).await? == hash
                    && pin
                        .finalized_block_number
                        .is_some_and(|terminal| terminal <= number),
                "HOLD: external discovery is not canonical finalized evidence"
            );
            if let Some(exclusion) = &admission.exclusion {
                verify_excluded_original(
                    sender,
                    &sender.api_provider,
                    &seal.old_identity,
                    &event,
                    &external.original,
                    exclusion,
                )
                .await?;
            } else {
                sender
                    .verify_saved_completion(api, &event, &external.original, pin)
                    .await?;
            }
            if let Some(uuid) = admission.undispatched_old_admission {
                let old = journal
                    .transactions
                    .get(&uuid)
                    .context("HOLD: external link is not a retained unsigned original")?;
                anyhow::ensure!(
                    is_undispatched_old_admission(old)
                        && old.tx.tx_hash == admission.original_tx_hash
                        && old.tx.slot_number.0 == admission.receipt_key.0
                        && old
                            .receipt
                            .as_ref()
                            .is_none_or(|_| old.receipt_event().is_ok_and(|original| {
                                original.transaction_index == event.transaction_index
                                    && original.receipt_rlp == event.receipt_rlp
                                    && original.proof_block.block.body.execution_payload.block_hash
                                        == execution.block_hash
                            })),
                    "HOLD: external original does not match retained unsigned admission"
                );
                authenticated.unsigned.insert(uuid);
            }
            anyhow::ensure!(
                api.get_block_hash(number).await? == hash,
                "HOLD: external discovery finalized pin changed"
            );
        } else if let Some(exclusion) = &admission.exclusion {
            anyhow::ensure!(
                admission.undispatched_old_admission.is_none()
                    && admission.admission
                        == if admission.fee_event_observed {
                            "paid"
                        } else {
                            "cutover-sponsored"
                        },
                "HOLD: excluded original fee/provenance changed"
            );
            let copy = admission
                .signed
                .as_ref()
                .context("HOLD: excluded original signature absent")?;
            if let Some(uuid) = admission.original_lane_uuid {
                let old = journal
                    .transactions
                    .get(&uuid)
                    .context("HOLD: excluded owned UUID is not in active original rows")?;
                let receipt = old
                    .receipt
                    .as_ref()
                    .context("HOLD: owned exclusion has no retained proof")?;
                let original = receipt
                    .signed_submission
                    .as_ref()
                    .context("HOLD: owned exclusion has no retained original signature")?;
                authenticate_retained_checkpoints(
                    &sender.api_provider,
                    original,
                    exclusion
                        .finalized_block_number
                        .context("HOLD: exclusion height absent")?,
                )
                .await?;
                anyhow::ensure!(
                    copy.sender
                        == subxt::utils::AccountId32::from(seal.old_identity.gear_sender)
                            .to_string()
                        && old.tx.tx_hash == admission.original_tx_hash
                        && old.tx.slot_number.0 == admission.receipt_key.0
                        && receipt.receipt_key == admission.receipt_key
                        && old.receipt_event()?.encode() == event.encode()
                        && receipt.submission_attempts.len() == admission.attempts.len()
                        && old.saved_settlement_observation.is_none()
                        && !matches!(old.status, super::tx_manager::TxStatus::Completed),
                    "HOLD: exclusion lost owned receipt/proof/signer linkage"
                );
                validate_exclusion_copy(original, copy)?;
                for (original, copy) in receipt.submission_attempts.iter().zip(&admission.attempts)
                {
                    authenticate_retained_checkpoints(
                        &sender.api_provider,
                        &original.signed_submission,
                        exclusion
                            .finalized_block_number
                            .context("HOLD: exclusion height absent")?,
                    )
                    .await?;
                    validate_exclusion_copy(&original.signed_submission, copy)?;
                }
                authenticated.excluded_owned.insert(uuid);
            } else {
                anyhow::ensure!(
                    !journal
                        .transactions
                        .values()
                        .chain(journal.completed.values())
                        .any(|old| {
                            old.tx.tx_hash == admission.original_tx_hash
                                && !is_undispatched_old_admission(old)
                        }),
                    "HOLD: sponsored exclusion replaced known original ownership"
                );
            }
            for original in admission.attempts.iter().chain(std::iter::once(copy)) {
                anyhow::ensure!(
                    admission.original_lane_uuid.is_none()
                        || original.sender
                            == subxt::utils::AccountId32::from(seal.old_identity.gear_sender)
                                .to_string(),
                    "HOLD: excluded owned proof attempt signer changed"
                );
                verify_excluded_original(
                    sender,
                    &sender.api_provider,
                    &seal.old_identity,
                    &event,
                    original,
                    exclusion,
                )
                .await?;
            }
        } else {
            anyhow::ensure!(
                admission.undispatched_old_admission.is_none()
                    && admission.admission
                        == if admission.fee_event_observed {
                            "paid"
                        } else {
                            "cutover-sponsored"
                        },
                "HOLD: original settlement provenance changed"
            );
            if let Some(uuid) = admission.original_lane_uuid {
                let old = journal
                    .completed
                    .get(&uuid)
                    .context("HOLD: settlement UUID is not completed")?;
                anyhow::ensure!(
                    old.tx.tx_hash == admission.original_tx_hash
                        && old.receipt_event()?.encode() == event.encode()
                        && admission.signed.is_none()
                        && admission.attempts.is_empty(),
                    "HOLD: completed original linkage changed"
                );
            } else {
                let original = admission
                    .signed
                    .as_ref()
                    .context("HOLD: sponsored signature absent")?;
                sender
                    .verify_saved_completion(api, &event, original, pin)
                    .await?;
                for attempt in &admission.attempts {
                    validate_original_identity(&seal.old_identity, &event, attempt)?;
                    let call = historical_proxy_client::historical_proxy::io::Redirect::encode_call(
                        event.proof_block.block.slot,
                        event.encode(),
                        seal.old_identity.vft_manager_address.0.into(),
                        vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec(),
                    );
                    authenticate_original_call(
                        sender,
                        &sender.api_provider,
                        attempt,
                        seal.old_identity.historical_proxy_address,
                        &call,
                    )
                    .await?;
                }
            }
        }
        if let Some(exclusion) = &admission.exclusion {
            anyhow::ensure!(
                sender
                    .receipt_resolution_status(api, &event, Some(exclusion))
                    .await?
                    .as_ref()
                    == Some(exclusion),
                "HOLD: saved exclusion no longer authenticates terminal receipt resolution"
            );
            authenticated.accounted_handoffs.insert(
                (admission.receipt_key.0, admission.original_tx_hash),
                (execution.block_number, block_hash),
            );
        } else if admission.external_original.is_some() {
            authenticated.fulfilled_handoffs.insert(
                (admission.receipt_key.0, admission.original_tx_hash),
                (execution.block_number, block_hash),
            );
        }
    }
    let final_head = api.last_block_hash().await?;
    anyhow::ensure!(
        api.block_number_at(final_head).await? >= finalized
            && api.get_block_hash(finalized).await? == head,
        "HOLD: Gear finality changed during coverage qualification"
    );
    Ok(authenticated)
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
            matches!(self.schema_version, 1 | 2)
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
    read_coverage(&seal)?;
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
    read_coverage(seal)?;
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
    let mut transactions = std::collections::BTreeSet::new();
    let mut receipts = std::collections::BTreeSet::new();
    for (uuid, tx) in journal.transactions.iter().chain(journal.completed.iter()) {
        anyhow::ensure!(
            *uuid == tx.uuid
                && !(journal.transactions.contains_key(uuid)
                    && journal.completed.contains_key(uuid))
                && transactions.insert((tx.tx.slot_number.0, tx.tx.tx_hash))
                && tx
                    .receipt
                    .as_ref()
                    .is_none_or(|receipt| receipts.insert(receipt.receipt_key)),
            "HOLD: original receipt UUID/key/transaction ownership changed"
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
    let coverage = read_coverage(seal)?;
    let authenticated = verify_external_coverage(seal, &journal, &coverage, &sender, api).await?;
    anyhow::ensure!(
        journal
            .transactions
            .keys()
            .all(|uuid| authenticated.unsigned.contains(uuid)
                || authenticated.excluded_owned.contains(uuid))
            && journal.failed.keys().all(|uuid| {
                journal.transactions.contains_key(uuid)
                    && (authenticated.unsigned.contains(uuid)
                        || authenticated.excluded_owned.contains(uuid))
            }),
        "HOLD: sealed source retains an unauthenticated active/failed original admission"
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
                authenticated.accounts_for_handoff(slot.0, *tx_hash, block.number.0, block.hash.expect("validated hash")),
                "HOLD: original block handoff has no exact authenticated fulfillment or accounted exclusion"
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
        if seal.schema_version == 2 {
            let signer: gsdk::signer::Signer = api.clone().into();
            let provider = ApiProviderConnection::from_api(signer.api().clone());
            // A v2 digest alone cannot authenticate terminal exceptions or original nonce ownership.
            verify_original_lane(&seal, provider, api).await?;
            verify_markers(path)?;
        }
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

fn validate_original_identity(
    identity: &InboundRuntimeIdentity,
    payload: &eth_events_electra_client::EthToVaraEvent,
    original: &super::message_sender::SignedSubmission,
) -> Result<()> {
    anyhow::ensure!(
        original.chain_genesis_hash == format!("{:#x}", identity.gear_genesis_hash)
            && original.manager == format!("{:#x}", identity.vft_manager_address)
            && original.historical_proxy == format!("{:#x}", identity.historical_proxy_address)
            && original.receipt_key == (payload.proof_block.block.slot, payload.transaction_index)
            && original.payload_hash
                == format!("{:#x}", alloy_primitives::keccak256(payload.encode())),
        "HOLD: original proof identity changed"
    );
    original.validate_native_reconciliation()
}

fn validate_disposition_bound(
    original: &super::message_sender::SignedSubmission,
    maximum: u32,
) -> Result<()> {
    anyhow::ensure!(
        original.prepared_finalized_number <= original.scanned_finalized_number
            && original.scanned_finalized_number <= maximum
            && original.reply_scanned_finalized_number <= maximum
            && original.reply_scanned_finalized_number >= original.prepared_finalized_number
            && original
                .inclusion_block_number
                .is_none_or(|number| number <= maximum)
            && original.finalized_reply.as_ref().is_none_or(|reply| {
                reply
                    .observation
                    .finalized_block_number
                    .is_some_and(|number| number <= maximum)
            })
            && original.has_terminal_reconciliation(),
        "HOLD: original has no terminal nonce/reply disposition at or before exclusion"
    );
    Ok(())
}

async fn authenticate_original_call(
    sender: &MessageSender,
    provider: &ApiProviderConnection,
    original: &super::message_sender::SignedSubmission,
    destination: H256,
    call: &[u8],
) -> Result<()> {
    use std::str::FromStr;
    let chain = provider.client();
    anyhow::ensure!(
        original.chain_genesis_hash == format!("{:#x}", chain.api.genesis_hash()),
        "HOLD: original belongs to another Gear genesis"
    );
    let mut observed = original.clone();
    if original.inclusion_block_number.is_none() {
        observed.scanned_finalized_number = original.prepared_finalized_number;
        observed.scanned_finalized_hash = original.prepared_finalized_hash.clone();
    }
    let included = sender
        .find_signed_extrinsic(&mut observed, destination, call)
        .await?;
    for (number, hash) in [
        (
            original.scanned_finalized_number,
            &original.scanned_finalized_hash,
        ),
        (
            original.reply_scanned_finalized_number,
            &original.reply_scanned_finalized_hash,
        ),
    ] {
        anyhow::ensure!(
            chain.block_number_to_hash(number).await? == H256::from_str(hash)?,
            "HOLD: original saved finalized checkpoint changed"
        );
    }
    if !included {
        let hash = H256::from_str(&original.scanned_finalized_hash)?;
        let latest = chain
            .block_hash_to_number(chain.latest_finalized_block().await?)
            .await?;
        let nonce = u64::from(chain.api.info_at(&original.sender, Some(hash)).await?.nonce);
        anyhow::ensure!(
            original.scanned_finalized_number <= latest
                && nonce > original.nonce
                && original
                    .finalized_nonce_consumed
                    .is_none_or(|saved| saved == nonce)
                && original.inclusion_block_number.is_none()
                && original.inclusion_block_hash.is_none()
                && original.message_id.is_none()
                && original.finalized_dispatch_error.is_none()
                && original.finalized_reply.is_none()
                && original.finalized_reply_error.is_none(),
            "HOLD: absent original has no canonical consumed-nonce disposition"
        );
    } else {
        anyhow::ensure!(
            observed.inclusion_block_number == original.inclusion_block_number
                && observed.inclusion_block_hash == original.inclusion_block_hash
                && observed.message_id == original.message_id
                && observed.finalized_dispatch_error == original.finalized_dispatch_error
                && original.finalized_nonce_consumed.is_none(),
            "HOLD: original dispatch changed"
        );
        if let Some(id) = original.message_id.as_deref() {
            sender
                .find_finalized_reply(&mut observed, H256::from_str(id)?, destination)
                .await?
                .context("HOLD: original reply unavailable")?;
            anyhow::ensure!(
                observed.finalized_reply == original.finalized_reply
                    && observed.finalized_reply_error == original.finalized_reply_error,
                "HOLD: original reply changed"
            );
        } else {
            anyhow::ensure!(
                original.finalized_dispatch_error.is_some()
                    && original.finalized_reply.is_none()
                    && original.finalized_reply_error.is_none(),
                "HOLD: original has an unknown outcome"
            );
        }
    }
    Ok(())
}

async fn verify_archived_proof(
    sender: &MessageSender,
    provider: &ApiProviderConnection,
    identity: &InboundRuntimeIdentity,
    payload: &eth_events_electra_client::EthToVaraEvent,
    original: &super::message_sender::SignedSubmission,
) -> Result<()> {
    anyhow::ensure!(
        original.sender == subxt::utils::AccountId32::from(identity.gear_sender).to_string(),
        "HOLD: archived owned proof signer changed"
    );
    validate_original_identity(identity, payload, original)?;
    for continuation in original
        .native_reconciliation_attempts
        .iter()
        .chain(original.native_reconciliation.as_deref())
    {
        sender
            .authenticate_archived_continuation(continuation)
            .await?;
    }
    let call = historical_proxy_client::historical_proxy::io::Redirect::encode_call(
        payload.proof_block.block.slot,
        payload.encode(),
        identity.vft_manager_address.0.into(),
        vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec(),
    );
    authenticate_original_call(
        sender,
        provider,
        original,
        identity.historical_proxy_address,
        &call,
    )
    .await
}

/// Authenticate nonce disposition, not financial completion. Never mutates the retained journal.
/// Ownership callers additionally pin `original.sender` to their lane's `gear_sender`.
pub async fn verify_excluded_original(
    sender: &MessageSender,
    provider: &ApiProviderConnection,
    identity: &InboundRuntimeIdentity,
    payload: &eth_events_electra_client::EthToVaraEvent,
    original: &super::message_sender::SignedSubmission,
    exclusion: &super::message_sender::FinalizedReceiptObservation,
) -> Result<()> {
    use std::str::FromStr;
    validate_original_identity(identity, payload, original)?;
    anyhow::ensure!(
        sender.receiver_address == identity.vft_manager_address
            && sender.historical_proxy_address == identity.historical_proxy_address,
        "HOLD: exclusion verifier runtime differs from original identity"
    );
    let maximum = exclusion
        .finalized_block_number
        .context("HOLD: exclusion has no finalized height")?;
    let chain = provider.client();
    let latest = chain
        .block_hash_to_number(chain.latest_finalized_block().await?)
        .await?;
    let hash = H256::from_str(&exclusion.finalized_block_hash)?;
    anyhow::ensure!(
        hash != H256::zero()
            && maximum <= latest
            && chain.block_number_to_hash(maximum).await? == hash,
        "HOLD: exclusion is not canonical finalized evidence"
    );
    validate_disposition_bound(original, maximum)?;
    let call = historical_proxy_client::historical_proxy::io::Redirect::encode_call(
        payload.proof_block.block.slot,
        payload.encode(),
        identity.vft_manager_address.0.into(),
        vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec(),
    );
    authenticate_original_call(
        sender,
        provider,
        original,
        identity.historical_proxy_address,
        &call,
    )
    .await?;
    for continuation in original
        .native_reconciliation_attempts
        .iter()
        .chain(original.native_reconciliation.as_deref())
    {
        validate_disposition_bound(continuation, maximum)?;
        let call = vft_manager_client::vft_manager::io::ReconcileReceipt::encode_call(
            original.receipt_key.0,
            original.receipt_key.1,
        );
        authenticate_original_call(
            sender,
            provider,
            continuation,
            identity.vft_manager_address,
            &call,
        )
        .await?;
    }
    anyhow::ensure!(
        chain.block_number_to_hash(maximum).await? == hash,
        "HOLD: exclusion finalized pin changed during original authentication"
    );
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

    fn original_fixture() -> super::super::message_sender::SignedSubmission {
        serde_json::from_value(serde_json::json!({
            "chain_genesis_hash":"original","manager":"original","historical_proxy":"original","sender":"original",
            "receipt_key":[32,0],"payload_hash":"original","nonce":1,"extrinsic_hash":"original","raw_extrinsic":[1,2],
            "prepared_finalized_number":1,"prepared_finalized_hash":"prepared","scanned_finalized_number":2,"scanned_finalized_hash":"scanned",
            "reply_scanned_finalized_number":2,"reply_scanned_finalized_hash":"scanned",
            "inclusion_block_number":null,"inclusion_block_hash":null,"message_id":null
        })).unwrap()
    }

    fn terminal_admission(uuid: Option<Uuid>) -> serde_json::Value {
        serde_json::json!({
            "receipt_key":[32,0],"original_tx_hash":alloy_primitives::B256::repeat_byte(1),
            "original_block_hash":H256::repeat_byte(2),"admission":"paid","proof":[],
            "settlement":{"finalized_block_number":10,"finalized_block_hash":"canonical"},
            "original_lane_uuid":uuid
        })
    }

    fn empty_covered_block(number: u64, parent: H256) -> CoveredBlock {
        let empty_root = alloy_primitives::keccak256([0x80]);
        let header = alloy::consensus::Header {
            number,
            parent_hash: parent.0.into(),
            receipts_root: empty_root,
            transactions_root: empty_root,
            ..Default::default()
        };
        CoveredBlock {
            number,
            hash: H256(header.hash_slow().0),
            parent_hash: parent,
            receipts_root: H256(empty_root.0),
            transactions: Vec::new(),
            receipts: Vec::new(),
            raw_transactions: Vec::new(),
            header,
        }
    }

    #[test]
    fn settlement_only_v1_and_versioned_exclusions_are_not_interchangeable() {
        let v1 = serde_json::json!({"schema_version":1,"admissions":[terminal_admission(None)]});
        let calls1 = serde_json::json!({"schema_version":1});
        validate_report_versions(1, &v1, &calls1).unwrap();
        let report: ExternalCoverageReport = serde_json::from_value(v1.clone()).unwrap();
        validate_admission_inventory(&report).unwrap();
        let mut excluded = v1.clone();
        excluded["admissions"][0]["exclusion"] = excluded["admissions"][0]["settlement"].clone();
        excluded["admissions"][0]["settlement"] = serde_json::Value::Null;
        assert!(validate_report_versions(1, &excluded, &calls1).is_err());
        assert!(
            validate_admission_inventory(&serde_json::from_value(excluded.clone()).unwrap())
                .is_err()
        );
        excluded["schema_version"] = 2.into();
        let calls2 = serde_json::json!({"schema_version":2});
        validate_report_versions(2, &excluded, &calls2).unwrap();
        validate_admission_inventory(&serde_json::from_value(excluded.clone()).unwrap()).unwrap();
        assert!(validate_report_versions(2, &excluded, &calls1).is_err());
        assert!(validate_report_versions(1, &v1, &calls2).is_err());
        let mut smuggled = v1.clone();
        smuggled["admissions"][0]["exclusion"] = serde_json::Value::Null;
        assert!(validate_report_versions(1, &smuggled, &calls1).is_err());
        let old_roles =
            serde_json::json!({"schema_version":1,"role_pins":[{"blocking_redemptions":0}]});
        assert!(validate_report_versions(1, &v1, &old_roles).is_err());
        let mut both = excluded.clone();
        both["admissions"][0]["settlement"] = v1["admissions"][0]["settlement"].clone();
        assert!(validate_admission_inventory(&serde_json::from_value(both).unwrap()).is_err());
        excluded["admissions"][0]["exclusion"] = serde_json::Value::Null;
        assert!(validate_admission_inventory(&serde_json::from_value(excluded).unwrap()).is_err());
    }

    #[test]
    fn duplicate_receipt_transaction_and_owned_uuid_fail_before_chain_queries() {
        let uuid = Uuid::from_u128(1);
        let first = terminal_admission(Some(uuid));
        let mut second = first.clone();
        let check = |second| {
            let report: ExternalCoverageReport = serde_json::from_value(serde_json::json!({
                "schema_version":2,"admissions":[first.clone(),second]
            }))
            .unwrap();
            validate_admission_inventory(&report)
        };
        assert!(check(second.clone()).is_err());
        second["receipt_key"] = serde_json::json!([32, 1]);
        second["original_tx_hash"] =
            serde_json::to_value(alloy_primitives::B256::repeat_byte(3)).unwrap();
        assert!(
            check(second.clone()).is_err(),
            "same ownership UUID cannot excuse two receipts"
        );
        second["original_lane_uuid"] = serde_json::Value::Null;
        check(second.clone()).unwrap();
        second["original_tx_hash"] = first["original_tx_hash"].clone();
        assert!(
            check(second).is_err(),
            "one Ethereum transaction cannot become two admissions"
        );
    }

    #[test]
    fn exclusion_nonce_reply_and_pin_guards_preserve_original_bytes() {
        use super::super::message_sender::{FinalizedReceiptObservation, FinalizedReplyEvidence};
        let original = original_fixture();
        let bytes = serde_json::to_vec(&original).unwrap();
        assert!(validate_disposition_bound(&original, 10).is_err());
        let mut copy = original.clone();
        copy.finalized_nonce_consumed = Some(copy.nonce);
        assert!(validate_disposition_bound(&copy, 10).is_err());
        copy.finalized_nonce_consumed = Some(copy.nonce + 1);
        validate_disposition_bound(&copy, 10).unwrap();
        validate_exclusion_copy(&original, &copy).unwrap();
        assert!(validate_disposition_bound(&copy, 1).is_err());
        copy.inclusion_block_number = Some(3);
        copy.inclusion_block_hash = Some("included".into());
        copy.message_id = Some("child".into());
        copy.finalized_nonce_consumed = None;
        assert!(
            validate_disposition_bound(&copy, 10).is_err(),
            "Resolved cannot excuse a missing original reply"
        );
        let mut reply = vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec();
        reply.extend(
            Result::<(), vft_manager_client::Error>::Err(
                vft_manager_client::Error::ReceiptResolved,
            )
            .encode(),
        );
        copy.finalized_reply = Some(FinalizedReplyEvidence {
            payload: reply,
            observation: FinalizedReceiptObservation {
                finalized_block_number: Some(4),
                finalized_block_hash: "reply".into(),
            },
        });
        validate_disposition_bound(&copy, 10).unwrap();
        assert!(validate_disposition_bound(&copy, 3).is_err());
        validate_exclusion_copy(&original, &copy).unwrap();
        let saved = copy.clone();
        copy.finalized_reply.as_mut().unwrap().payload.push(0);
        assert!(validate_exclusion_copy(&saved, &copy).is_err());
        copy = saved.clone();
        copy.raw_extrinsic.push(0);
        assert!(validate_exclusion_copy(&saved, &copy).is_err());
        copy = saved.clone();
        copy.native_reconciliation = Some(Box::new(original.clone()));
        assert!(validate_exclusion_copy(&saved, &copy).is_err());
        assert_eq!(serde_json::to_vec(&original).unwrap(), bytes);
    }

    #[test]
    fn accounted_handoff_requires_exact_slot_transaction_block_and_hash() {
        let mut coverage = AuthenticatedCoverage::default();
        let tx = alloy_primitives::B256::repeat_byte(1);
        let hash = H256::repeat_byte(2);
        coverage.accounted_handoffs.insert((32, tx), (100, hash));
        assert!(
            coverage.fulfilled_handoffs.is_empty(),
            "exclusion is not financial fulfillment"
        );
        assert!(coverage.accounts_for_handoff(32, tx, 100, hash));
        assert!(!coverage.accounts_for_handoff(33, tx, 100, hash));
        assert!(!coverage.accounts_for_handoff(
            32,
            alloy_primitives::B256::repeat_byte(3),
            100,
            hash
        ));
        assert!(!coverage.accounts_for_handoff(32, tx, 101, hash));
        assert!(!coverage.accounts_for_handoff(32, tx, 100, H256::repeat_byte(3)));
    }

    #[test]
    fn canonical_receipt_fixture_binds_exact_transaction_index_hash_and_trie_proof() {
        use eth_events_electra_client::{
            BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent,
        };
        use ethereum_common::{
            beacon::electra::Block,
            utils::{self, BeaconBlockResponse},
        };
        use std::io::Read;
        #[derive(Deserialize)]
        struct Receipts {
            result: Vec<TransactionReceipt>,
        }
        #[derive(Deserialize)]
        struct Fixture {
            tx_hash: ethereum_client::TxHash,
            tx_index: u64,
            receipts: Receipts,
            block: BeaconBlockResponse<Block>,
        }
        let bytes: &[u8] = include_bytes!("../../../../tests/src/relayer/transactions.json.zst");
        let mut decoder = ruzstd::StreamingDecoder::new(bytes).unwrap();
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        let fixture = serde_json::from_slice::<Vec<Fixture>>(&decoded)
            .unwrap()
            .into_iter()
            .min_by_key(|fixture| fixture.tx_hash)
            .unwrap();
        let block = fixture.block.data.message;
        let execution = &block.body.execution_payload;
        let mapped = fixture
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
        let merkle = utils::generate_merkle_proof(fixture.tx_index, &mapped).unwrap();
        // This test exercises receipt linkage; header/chain authentication is an independent gate.
        let covered = CoveredBlock {
            number: execution.block_number,
            hash: H256::from(execution.block_hash.0 .0),
            parent_hash: H256::from(execution.parent_hash.0 .0),
            receipts_root: H256::from(execution.receipts_root.0 .0),
            transactions: execution
                .transactions
                .iter()
                .map(|raw| alloy_primitives::keccak256(raw.0.as_ref()))
                .collect(),
            raw_transactions: execution
                .transactions
                .iter()
                .map(|raw| raw.0.to_vec())
                .collect(),
            receipts: fixture.receipts.result,
            header: Default::default(),
        };
        let event = EthToVaraEvent {
            proof_block: BlockInclusionProof {
                block: BlockGenericForBlockBody {
                    slot: block.slot,
                    proposer_index: block.proposer_index,
                    parent_root: block.parent_root,
                    state_root: block.state_root,
                    body: block.body.into(),
                },
                headers: Vec::new(),
            },
            proof: merkle.proof,
            transaction_index: fixture.tx_index,
            receipt_rlp: alloy_rlp::encode(&merkle.receipt),
        };
        let mut admission: ExternalCoverageAdmission =
            serde_json::from_value(terminal_admission(None)).unwrap();
        admission.receipt_key = (event.proof_block.block.slot, event.transaction_index);
        admission.original_tx_hash = fixture.tx_hash;
        admission.original_block_hash = covered.hash;
        admission.proof = event.encode();
        let coverage = ExternalCoverageReport {
            schema_version: 2,
            blocks: vec![covered],
            admissions: Vec::new(),
        };
        let mut encoded = admission.proof.as_slice();
        let decoded = EthToVaraEvent::decode(&mut encoded).unwrap();
        assert!(encoded.is_empty());
        validate_admission_block(&coverage, &admission, &decoded).unwrap();
        admission.original_tx_hash = alloy_primitives::B256::repeat_byte(99);
        assert!(validate_admission_block(&coverage, &admission, &decoded).is_err());
        admission.original_tx_hash = fixture.tx_hash;
        let mut wrong = decoded.clone();
        wrong.transaction_index = wrong.transaction_index.checked_add(1).unwrap();
        assert!(validate_admission_block(&coverage, &admission, &wrong).is_err());
        wrong = decoded.clone();
        wrong.proof.clear();
        assert!(validate_admission_block(&coverage, &admission, &wrong).is_err());
        wrong = decoded;
        wrong.receipt_rlp.push(0);
        assert!(validate_admission_block(&coverage, &admission, &wrong).is_err());
    }

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
            fs::write(base.join(name), br#"{"schema_version":1,"admissions":[]}"#).unwrap();
        }
        let seal = fixture_seal(&base, identity());
        let first = empty_covered_block(0, H256::zero());
        let last = empty_covered_block(1, first.hash);
        let mut chain_seal = seal.clone();
        chain_seal.schema_version = 2;
        chain_seal.old_identity.ethereum_start_block = 0;
        chain_seal.new_identity.ethereum_start_block = 2;
        chain_seal.old_identity.ethereum_genesis_hash = first.hash;
        chain_seal.new_identity.ethereum_genesis_hash = first.hash;
        chain_seal.boundary.execution_number = 1;
        chain_seal.boundary.execution_hash = last.hash;
        chain_seal.validate().unwrap();
        validate_coverage_chain(&chain_seal, &[first.clone(), last.clone()]).unwrap();
        assert!(validate_coverage_chain(&chain_seal, &[last.clone()]).is_err());
        let wrong_parent = empty_covered_block(1, H256::repeat_byte(3));
        chain_seal.boundary.execution_hash = wrong_parent.hash;
        assert!(validate_coverage_chain(&chain_seal, &[first.clone(), wrong_parent]).is_err());
        let mut malformed = first;
        malformed.raw_transactions.push(vec![0]);
        assert!(validate_covered_block(&malformed).is_err());
        let mut bad_root = last;
        bad_root.header.receipts_root = alloy_primitives::B256::ZERO;
        bad_root.receipts_root = H256::zero();
        bad_root.hash = H256(bad_root.header.hash_slow().0);
        assert!(validate_covered_block(&bad_root).is_err());
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
