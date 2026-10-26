use crate::lane_policy::{LaneApprovals, LanePolicy};
use ethereum_client::{
    prepared_content_message_identity, FinalizedSubmissionSnapshot, PreparedContentMessage, TxHash,
};
use gear_rpc_client::dto::{MerkleProof, Message};
use prometheus::IntCounter;
use sails_rs::ActorId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{mpsc::UnboundedReceiver, RwLock};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

use crate::message_relayer::{
    common::{
        ethereum::{
            accumulator::{self, utils::MerkleRoots, AccumulatorIo},
            message_sender::{self, MessageSenderIo},
            status_fetcher::{self, StatusFetcherIo},
        },
        gear::merkle_proof_fetcher::MerkleRootFetcherIo,
        message_hash, MessageInBlock, RelayedMerkleRoot,
    },
    gear_to_eth::storage::Storage,
};

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub journal_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<status_fetcher::CompletionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_evidence: Option<status_fetcher::FinalizedFailureEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce_disposition: Option<FinalizedSubmissionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane_policy: Option<LanePolicy>,
    pub uuid: Uuid,
    pub message: MessageInBlock,
    pub message_hash: [u8; 32],
    pub status: TxStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethereum_submission: Option<PreparedContentMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_evidence: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_evidence: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ethereum_tx_attempts: Vec<TxHash>,
    // Only a successful canonical inclusion can populate this payout hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethereum_tx_hash: Option<TxHash>,
}

impl Transaction {
    pub fn new(message: MessageInBlock, status: TxStatus) -> Self {
        let uuid = Uuid::now_v7();
        Self {
            journal_version: 6,
            completion: None,
            failure_evidence: None,
            nonce_disposition: None,
            lane_policy: None,
            uuid,
            status,
            message_hash: message_hash(&message.message),
            message,
            ethereum_submission: None,
            legacy_evidence: None,
            historical_evidence: None,
            ethereum_tx_attempts: Vec::new(),
            ethereum_tx_hash: None,
        }
    }

    fn remember_attempt(&mut self, hash: TxHash) {
        if !self.ethereum_tx_attempts.contains(&hash) {
            self.ethereum_tx_attempts.push(hash);
        }
    }

    /// Validate persisted identity/effects; callers must separately recheck canonical finality.
    pub fn validate_journal(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(self.journal_version, 2..=6),
            "HOLD: unsupported outbound transaction journal version {}",
            self.journal_version
        );
        if matches!(self.status, TxStatus::Completed) {
            let signed = self.ethereum_submission.as_ref().ok_or_else(|| {
                anyhow::anyhow!("HOLD: completed journal lacks original signed bytes")
            })?;
            let evidence = self.completion.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "HOLD: historical completed journal lacks original finalized receipt/effects"
                )
            })?;
            status_fetcher::validate_completion(&self.message.message, signed, evidence)?;
            if self.journal_version >= 5 {
                anyhow::ensure!(
                    evidence.inclusion.is_some() && evidence.checkpoint.is_some(),
                    "HOLD: version 5 completion lacks authenticated receipt evidence"
                );
            }
            anyhow::ensure!(
                self.ethereum_tx_hash == Some(signed.hash),
                "HOLD: completion version/hash changed"
            );
            if self.journal_version >= 6 {
                let policy = self.lane_policy.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HOLD: completion lacks approved lane policy")
                })?;
                policy.validate(&self.message.message, signed)?;
                anyhow::ensure!(
                    evidence.token_delivery == policy.token_delivery_required,
                    "HOLD: completion token effect disagrees with approved lane policy"
                );
            }
        }
        if let Some(evidence) = &self.failure_evidence {
            let signed = self
                .ethereum_submission
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("HOLD: failed original lacks signed bytes"))?;
            status_fetcher::validate_failure(&self.message.message, signed, evidence)?;
            anyhow::ensure!(
                matches!(self.status, TxStatus::Failed { .. })
                    && self.completion.is_none()
                    && self.ethereum_tx_hash.is_none(),
                "HOLD: failed original claims completion"
            );
        }
        if self.journal_version >= 5
            && matches!(self.status, TxStatus::Failed { .. })
            && self.ethereum_submission.is_some()
        {
            anyhow::ensure!(
                self.failure_evidence.as_ref().is_some_and(
                    |evidence| evidence.inclusion.is_some() && evidence.checkpoint.is_some()
                ),
                "HOLD: version 5 signed failure lacks authenticated original revert"
            );
        }
        if matches!(self.status, TxStatus::Failed { .. }) && self.ethereum_submission.is_none() {
            anyhow::ensure!(
                self.ethereum_tx_attempts.is_empty(),
                "HOLD: failed original has attempts but no retained signed transaction"
            );
        }
        if let Some(snapshot) = &self.nonce_disposition {
            anyhow::ensure!(
                self.ethereum_submission.is_some()
                    && snapshot.pinned_nonce_consumed()
                    && snapshot.block_hash() != alloy::primitives::B256::ZERO,
                "HOLD: invalid original nonce disposition"
            );
        }
        anyhow::ensure!(
            matches!(self.status, TxStatus::Completed)
                || (self.completion.is_none() && self.ethereum_tx_hash.is_none()),
            "HOLD: unresolved original claims completion"
        );
        anyhow::ensure!(
            self.message_hash == message_hash(&self.message.message),
            "HOLD: outbound message identity changed for {}",
            self.uuid
        );
        let unsigned = matches!(
            &self.status,
            TxStatus::WaitForMerkleRoot
                | TxStatus::FetchMerkleRoot(_)
                | TxStatus::PrepareMessage(..)
        );
        if unsigned {
            anyhow::ensure!(
                self.ethereum_tx_attempts.is_empty() && self.ethereum_tx_hash.is_none(),
                "HOLD: outbound unsigned state discarded a prior transaction identity"
            );
        }
        if let Some(signed) = &self.ethereum_submission {
            status_fetcher::validate_original_submission(&self.message.message, signed)?;
            if self.journal_version >= 6 {
                self.lane_policy
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!("HOLD: signed original lacks approved lane policy")
                    })?
                    .validate(&self.message.message, signed)?;
            }
            anyhow::ensure!(
                !unsigned,
                "HOLD: signed outbound submission has an unsigned status"
            );
            if let TxStatus::WaitConfirmations(hash) = &self.status {
                anyhow::ensure!(
                    *hash == signed.hash,
                    "HOLD: outbound confirmation watcher changed original hash"
                );
            }
            anyhow::ensure!(
                self.ethereum_tx_attempts.contains(&signed.hash),
                "HOLD: outbound original signed hash is missing from attempts"
            );
            if let Some(receipt) = self.ethereum_tx_hash {
                anyhow::ensure!(
                    receipt == signed.hash,
                    "HOLD: outbound receipt hash differs from original submission"
                );
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TxStatus {
    WaitForMerkleRoot,
    FetchMerkleRoot(RelayedMerkleRoot),
    PrepareMessage(RelayedMerkleRoot, MerkleProof),
    SendMessage(RelayedMerkleRoot, MerkleProof),
    WaitConfirmations(TxHash),
    NeedsReconciliation { diagnostic: String },
    Failed { diagnostic: String },
    Completed,
}

enum ExistingTransaction {
    Completed,
    Active(TxStatus),
}

impl_metered_service!(
    struct Metrics {
        total_transactions: IntCounter = IntCounter::new(
            "eth_gear_transaction_manager_total_transactions",
            "Total number of transactions processed by the transaction manager",
        ),
        completed_transactions: IntCounter = IntCounter::new(
            "eth_gear_transaction_manager_completed_transactions",
            "Total number of completed transactions",
        ),
        failed_transactions: IntCounter = IntCounter::new(
            "eth_geartransaction_manager_failed_transactions",
            "Total number of failed transactions",
        ),
    }
);

pub struct TransactionManager {
    pub merkle_roots: Arc<RwLock<MerkleRoots>>,

    pub transactions: RwLock<BTreeMap<Uuid, Transaction>>,
    pub failed: RwLock<BTreeMap<Uuid, String>>,
    pub completed: RwLock<BTreeMap<Uuid, Transaction>>,
    pub storage: Arc<dyn Storage>,
    lane_approvals: LaneApprovals,
    policy_api: Option<ethereum_client::EthApi>,
    source_api: Option<gear_rpc_client::GearApi>,

    metrics: Metrics,
}

impl MeteredService for TransactionManager {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl TransactionManager {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            merkle_roots: Arc::new(RwLock::new(MerkleRoots::new(100))),
            transactions: RwLock::new(BTreeMap::new()),
            failed: RwLock::new(BTreeMap::new()),
            completed: RwLock::new(BTreeMap::new()),
            storage,
            lane_approvals: LaneApprovals::default(),
            policy_api: None,
            source_api: None,

            metrics: Metrics::new(),
        }
    }

    pub fn with_source_api(mut self, api: gear_rpc_client::GearApi) -> Self {
        self.source_api = Some(api);
        self
    }

    pub fn with_lane_approvals(
        mut self,
        api: ethereum_client::EthApi,
        approvals: LaneApprovals,
    ) -> Self {
        self.policy_api = Some(api);
        self.lane_approvals = approvals;
        self
    }

    pub async fn fail_transaction(&self, tx_uuid: Uuid, reason: String) {
        if self.failed.write().await.insert(tx_uuid, reason).is_none() {
            self.metrics.failed_transactions.inc();
        }
    }

    async fn fail_active_transaction(&self, tx_uuid: Uuid, reason: String) -> Option<[u8; 32]> {
        let mut active = self.transactions.write().await;
        let tx = active.get_mut(&tx_uuid)?;
        tx.status = TxStatus::Failed {
            diagnostic: reason.clone(),
        };
        let nonce = tx.message.message.nonce_be;
        drop(active);
        self.fail_transaction(tx_uuid, reason).await;
        Some(nonce)
    }

    pub async fn add_transaction(&self, tx: Transaction) {
        self.metrics.total_transactions.inc();

        match tx.status {
            TxStatus::Completed => {
                self.completed.write().await.insert(tx.uuid, tx);
                self.metrics.completed_transactions.inc();
            }

            _ => {
                self.transactions.write().await.insert(tx.uuid, tx);
            }
        }
    }

    async fn complete_transaction(
        &self,
        uuid: Uuid,
        evidence: status_fetcher::CompletionEvidence,
    ) -> anyhow::Result<bool> {
        let mut active = self.transactions.write().await;
        let Some(current) = active.get(&uuid) else {
            return Ok(false);
        };
        if !matches!(
            &current.status,
            TxStatus::PrepareMessage(..)
                | TxStatus::SendMessage(..)
                | TxStatus::WaitConfirmations(_)
                | TxStatus::NeedsReconciliation { .. }
                | TxStatus::Failed { .. }
        ) {
            return Ok(false);
        }
        anyhow::ensure!(
            current.failure_evidence.is_none(),
            "HOLD: finalized failed original cannot become successful"
        );
        let signed = current
            .ethereum_submission
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("HOLD: completion has no original signed bytes"))?;
        status_fetcher::validate_completion(&current.message.message, signed, &evidence)?;
        let policy = current
            .lane_policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("HOLD: completion lacks approved lane policy"))?;
        policy.validate(&current.message.message, signed)?;
        anyhow::ensure!(
            evidence.token_delivery == policy.token_delivery_required,
            "HOLD: completion token effect disagrees with approved lane policy"
        );
        anyhow::ensure!(
            evidence.inclusion.is_some() && evidence.checkpoint.is_some(),
            "HOLD: completion lacks authenticated original receipt"
        );
        let ethereum_tx_hash = Some(signed.hash);
        let mut tx = active
            .remove(&uuid)
            .expect("active transaction was checked");
        drop(active);

        if let TxStatus::WaitConfirmations(hash) = &tx.status {
            let hash = *hash;
            tx.remember_attempt(hash);
        }
        tx.status = TxStatus::Completed;
        tx.journal_version = 6;
        tx.completion = Some(evidence);
        tx.ethereum_tx_hash = ethereum_tx_hash;
        self.completed.write().await.insert(uuid, tx);
        self.failed.write().await.remove(&uuid);
        self.metrics.completed_transactions.inc();
        Ok(true)
    }

    async fn record_finalized_failure(
        &self,
        uuid: Uuid,
        evidence: status_fetcher::FinalizedFailureEvidence,
    ) -> anyhow::Result<bool> {
        let reason;
        {
            let mut active = self.transactions.write().await;
            let Some(tx) = active.get_mut(&uuid) else {
                return Ok(false);
            };
            let signed = tx
                .ethereum_submission
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("HOLD: failed original lacks signed bytes"))?;
            status_fetcher::validate_failure(&tx.message.message, signed, &evidence)?;
            anyhow::ensure!(
                evidence.inclusion.is_some() && evidence.checkpoint.is_some(),
                "HOLD: failure lacks authenticated original revert"
            );
            reason = format!("Original transaction {} finalized reverted", signed.hash);
            if let Some(saved) = &tx.failure_evidence {
                anyhow::ensure!(
                    saved.receipt == evidence.receipt
                        && saved.included_block_number == evidence.included_block_number
                        && saved.included_block_hash == evidence.included_block_hash,
                    "HOLD: failed original inclusion changed"
                );
            } else {
                tx.failure_evidence = Some(evidence);
            }
            tx.status = TxStatus::Failed {
                diagnostic: reason.clone(),
            };
            tx.journal_version = 6;
        }
        self.fail_transaction(uuid, reason).await;
        // The sender cannot unlock the account until the complete failed-original journal is durable.
        self.storage.save(self).await?;
        Ok(true)
    }

    async fn ack_completed_event(&self, uuid: Uuid) -> anyhow::Result<()> {
        let nonce = self
            .completed
            .read()
            .await
            .get(&uuid)
            .map(|tx| tx.message.message.nonce_be)
            .ok_or_else(|| anyhow::anyhow!("Completed transaction {uuid} is missing"))?;
        self.storage.ack_event_pair(nonce).await
    }

    async fn existing_transaction(
        &self,
        message: &MessageInBlock,
    ) -> anyhow::Result<Option<ExistingTransaction>> {
        let nonce = message.message.nonce_be;
        if let Some(tx) = self
            .completed
            .read()
            .await
            .values()
            .find(|tx| tx.message.message.nonce_be == nonce)
        {
            anyhow::ensure!(
                tx.message.same_original(message),
                "HOLD: source message conflicts with completed nonce {}",
                hex::encode(nonce)
            );
            return Ok(Some(ExistingTransaction::Completed));
        }
        if let Some(tx) = self
            .transactions
            .read()
            .await
            .values()
            .find(|tx| tx.message.message.nonce_be == nonce)
        {
            anyhow::ensure!(
                tx.message.same_original(message),
                "HOLD: source message conflicts with active nonce {}",
                hex::encode(nonce)
            );
            return Ok(Some(ExistingTransaction::Active(tx.status.clone())));
        }
        Ok(None)
    }

    async fn accept_message(
        &self,
        mut message: MessageInBlock,
    ) -> anyhow::Result<Option<Transaction>> {
        let nonce = message.message.nonce_be;
        match self.existing_transaction(&message).await? {
            Some(ExistingTransaction::Completed) => {
                self.storage.save(self).await?;
                self.storage.ack_event_pair(nonce).await?;
                return Ok(None);
            }
            Some(ExistingTransaction::Active(status)) => {
                log::info!(
                    "Skipping active outbound nonce {} in status {:?}",
                    hex::encode(nonce),
                    status
                );
                return Ok(None);
            }
            None => {}
        }
        if !self.storage.message_is_eligible(&message).await? {
            log::info!(
                "Deferring outbound nonce {} until its durable queued/fee evidence is available",
                hex::encode(nonce)
            );
            return Ok(None);
        }
        let api = self
            .policy_api
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("HOLD: no approved outbound lane configuration"))?;
        let policy = self.lane_approvals.qualify(api, &message.message, None)?;
        if message.queue_id.is_none() {
            message
                .qualify_queue(self.source_api.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("HOLD: source identity requalification unavailable")
                })?)
                .await?;
        }
        self.storage
            .block_storage()
            .complete_transaction(&message)
            .await;
        let mut tx = Transaction::new(message, TxStatus::WaitForMerkleRoot);
        tx.lane_policy = Some(policy);
        self.add_transaction(tx.clone()).await;
        self.storage.save(self).await?;
        // The original event observations remain until durable completion.
        Ok(Some(tx))
    }

    pub async fn update_storage(&self) -> anyhow::Result<()> {
        self.storage.save(self).await
    }

    pub async fn load_from_storage(&self) -> anyhow::Result<()> {
        self.storage.load(self).await?;
        self.hold_ambiguous_send().await?;
        Ok(())
    }

    pub async fn verify_completed(&self, api: &ethereum_client::EthApi) -> anyhow::Result<()> {
        // No mutation or acknowledgement until every original and saved anchor qualifies.
        let mut completed = self.completed.read().await.clone();
        let mut active = self.transactions.read().await.clone();
        let mut view = api.verified_finalized_view().await?;
        for tx in completed.values_mut().chain(active.values_mut()) {
            tx.validate_journal()?;
            if tx.journal_version < 6 && tx.historical_evidence.is_none() {
                tx.historical_evidence = Some(serde_json::to_value(&*tx)?);
            }
            let had_approved_policy = tx.lane_policy.is_some();
            let policy = match &tx.lane_policy {
                Some(policy) => {
                    if let Some(signed) = &tx.ethereum_submission {
                        self.lane_approvals.validate_saved(
                            policy,
                            api,
                            &tx.message.message,
                            signed,
                        )?;
                    } else {
                        anyhow::ensure!(
                            self.lane_approvals
                                .qualify(api, &tx.message.message, None)?
                                == *policy,
                            "HOLD: saved unsigned lane policy differs from independent approval"
                        );
                    }
                    policy.clone()
                }
                None => self.lane_approvals.qualify(
                    api,
                    &tx.message.message,
                    tx.ethereum_submission.as_ref(),
                )?,
            };
            tx.lane_policy = Some(policy.clone());
            if let Some(signed) = &tx.ethereum_submission {
                policy.validate(&tx.message.message, signed)?;
                if let Some(saved) = &mut tx.completion {
                    Self::verify_saved_receipt(
                        api,
                        &mut view,
                        signed,
                        &saved.receipt,
                        saved.finalized_block_number,
                        saved.finalized_block_hash,
                        saved.inclusion.as_ref(),
                        saved.checkpoint.as_ref(),
                    )
                    .await?;
                    let receipt =
                        api.get_finalized_receipt(signed.hash)
                            .await?
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "HOLD: saved completion has no original finalized receipt"
                                )
                            })?;
                    let observed = status_fetcher::completion_evidence(
                        api,
                        &tx.message.message,
                        signed,
                        receipt,
                        &policy,
                    )
                    .await?;
                    anyhow::ensure!(
                        Self::same_original_receipt(&observed.receipt, &saved.receipt)
                            && (!had_approved_policy
                                || observed.token_delivery == saved.token_delivery),
                        "HOLD: saved completion receipt/effect policy changed"
                    );
                    saved.token_delivery = observed.token_delivery;
                    if saved.inclusion.is_none()
                        || saved
                            .checkpoint
                            .as_ref()
                            .is_none_or(|checkpoint| checkpoint.beacon().is_none())
                    {
                        saved.finalized_block_number = observed.finalized_block_number;
                        saved.finalized_block_hash = observed.finalized_block_hash;
                        saved.inclusion = observed.inclusion;
                        saved.checkpoint = observed.checkpoint;
                    }
                }
                if matches!(tx.status, TxStatus::Failed { .. }) {
                    if let Some(saved) = &tx.failure_evidence {
                        Self::verify_saved_receipt(
                            api,
                            &mut view,
                            signed,
                            &saved.receipt,
                            saved.finalized_block_number,
                            saved.finalized_block_hash,
                            saved.inclusion.as_ref(),
                            saved.checkpoint.as_ref(),
                        )
                        .await?;
                    }
                    let receipt =
                        api.get_finalized_receipt(signed.hash)
                            .await?
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "HOLD: signed failed original receipt is unavailable"
                                )
                            })?;
                    let observed =
                        status_fetcher::failure_evidence(api, &tx.message.message, signed, receipt)
                            .await?;
                    if let Some(saved) = &mut tx.failure_evidence {
                        anyhow::ensure!(
                            Self::same_original_receipt(&observed.receipt, &saved.receipt)
                                && observed.included_block_number == saved.included_block_number
                                && observed.included_block_hash == saved.included_block_hash,
                            "HOLD: failed original receipt changed"
                        );
                        if saved.inclusion.is_none()
                            || saved
                                .checkpoint
                                .as_ref()
                                .is_none_or(|checkpoint| checkpoint.beacon().is_none())
                        {
                            saved.finalized_block_number = observed.finalized_block_number;
                            saved.finalized_block_hash = observed.finalized_block_hash;
                            saved.inclusion = observed.inclusion;
                            saved.checkpoint = observed.checkpoint;
                        }
                    } else {
                        tx.failure_evidence = Some(observed);
                    }
                }
                if let Some(saved) = &tx.nonce_disposition {
                    let qualified = api
                        .verify_finalized_submission_snapshot(&signed.raw_transaction, saved)
                        .await?;
                    if tx.historical_evidence.is_none() && saved.legacy().is_some() {
                        tx.historical_evidence =
                            Some(serde_json::json!({"nonce_disposition": saved}));
                    }
                    tx.nonce_disposition = Some(qualified);
                }
            }
            tx.journal_version = 6;
            tx.validate_journal()?;
        }
        view.recheck(api.raw_provider()).await?;
        *self.completed.write().await = completed;
        *self.transactions.write().await = active;
        // Existing save.pending protocol durably writes UUID records before acknowledgements.
        self.storage.save(self).await?;
        for tx in self.completed.read().await.values() {
            self.storage
                .ack_event_pair(tx.message.message.nonce_be)
                .await?;
        }
        Ok(())
    }

    fn same_original_receipt(
        left: &alloy::rpc::types::TransactionReceipt,
        right: &alloy::rpc::types::TransactionReceipt,
    ) -> bool {
        left.inner == right.inner
            && left.transaction_hash == right.transaction_hash
            && left.transaction_index == right.transaction_index
            && left.block_hash == right.block_hash
            && left.block_number == right.block_number
            && left.from == right.from
            && left.to == right.to
    }

    #[allow(clippy::too_many_arguments)]
    async fn verify_saved_receipt(
        api: &ethereum_client::EthApi,
        view: &mut ethereum_client::FinalizedView,
        signed: &PreparedContentMessage,
        receipt: &alloy::rpc::types::TransactionReceipt,
        finalized_number: u64,
        finalized_hash: alloy::primitives::B256,
        inclusion: Option<&ethereum_client::inclusion::RecordedReceiptProof>,
        checkpoint: Option<&ethereum_client::FinalityCheckpoint>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            receipt.block_number != Some(finalized_number)
                || receipt.block_hash == Some(finalized_hash),
            "HOLD: saved finality and inclusion conflict at the same height"
        );
        view.verify_blocks(
            api.raw_provider(),
            &BTreeMap::from([
                (finalized_number, finalized_hash),
                (
                    receipt
                        .block_number
                        .ok_or_else(|| anyhow::anyhow!("HOLD: missing inclusion height"))?,
                    receipt
                        .block_hash
                        .ok_or_else(|| anyhow::anyhow!("HOLD: missing inclusion hash"))?,
                ),
            ]),
        )
        .await?;
        if let Some(checkpoint) = checkpoint {
            view.verify_checkpoint(api.raw_provider(), checkpoint)
                .await?;
        }
        if let Some(inclusion) = inclusion {
            inclusion.verify_original_inclusion(view, &signed.raw_transaction, signed.hash)?;
        }
        Ok(())
    }

    pub(super) async fn requalify_unsigned_roots(
        &self,
        api: &ethereum_client::EthApi,
    ) -> anyhow::Result<()> {
        let mut transactions = self.transactions.write().await;
        let mut view = None;
        let roots = self.merkle_roots.read().await;
        for tx in transactions.values_mut() {
            if tx.ethereum_submission.is_some() {
                continue;
            }
            if matches!(
                tx.status,
                TxStatus::WaitForMerkleRoot
                    | TxStatus::FetchMerkleRoot(_)
                    | TxStatus::PrepareMessage(..)
            ) {
                tx.message
                    .qualify_queue(self.source_api.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("HOLD: source identity requalification unavailable")
                    })?)
                    .await?;
            }
            let mut root = match &tx.status {
                TxStatus::FetchMerkleRoot(root) | TxStatus::PrepareMessage(root, _) => *root,
                _ => continue,
            };
            let saved_queue = root.queue_id;
            root.queue_id = Some(
                crate::message_relayer::common::gear::queue_id_at(
                    self.source_api.as_ref().expect("source qualified"),
                    root.block.0,
                    root.block_hash,
                )
                .await?,
            );
            anyhow::ensure!(
                saved_queue.is_none() || saved_queue == root.queue_id,
                "HOLD: saved root QueueId changed"
            );
            if view.is_none() {
                view = Some(api.verified_finalized_view().await?);
            }
            let pin = view.as_ref().expect("finalized pin established");
            let observed = api
                .read_finalized_merkle_root_at(root.block.0, pin.block_number(), pin.block_hash())
                .await?;
            tx.status = if root.covers(
                tx.message.queue_id.expect("message qualified"),
                tx.message.block,
            ) && observed == Some(root.merkle_root.0)
                && roots
                    .qualified_roots()
                    .iter()
                    .any(|qualified| qualified.root == root)
            {
                // Rebuild an unsigned saved proof from the authenticated source rather than trust legacy cache bytes.
                TxStatus::FetchMerkleRoot(root)
            } else {
                TxStatus::WaitForMerkleRoot
            };
        }
        if let Some(pin) = view {
            anyhow::ensure!(
                api.is_finalized_block(
                    pin.block_number(),
                    primitive_types::H256(pin.block_hash().0)
                )
                .await?,
                "HOLD: unsigned root qualification pin changed"
            );
        }
        drop(roots);
        drop(transactions);
        self.storage.save(self).await?;
        Ok(())
    }

    fn ambiguous_send_error(tx: &Transaction) -> anyhow::Error {
        anyhow::anyhow!(
            "HOLD: persisted outbound SendMessage transaction {} at source block {} nonce {} has no recorded signed account nonce/raw transaction/hash; it may already be broadcast. Reconcile the original submission before restart; refusing a replacement send",
            tx.uuid,
            tx.message.block.0,
            hex::encode(tx.message.message.nonce_be),
        )
    }

    async fn hold_ambiguous_send(&self) -> anyhow::Result<()> {
        for tx in self.transactions.read().await.values() {
            tx.validate_journal()?;
            match &tx.status {
                TxStatus::SendMessage(..) if tx.ethereum_submission.is_none() => {
                    return Err(Self::ambiguous_send_error(tx))
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn journal_signed_submission(
        &self,
        tx_uuid: Uuid,
        signed: PreparedContentMessage,
    ) -> anyhow::Result<bool> {
        prepared_content_message_identity(&signed)?;
        {
            let mut transactions = self.transactions.write().await;
            let Some(tx) = transactions.get_mut(&tx_uuid) else {
                return Ok(false);
            };
            tx.lane_policy
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!("HOLD: signing has no durable approved lane policy")
                })?
                .validate(&tx.message.message, &signed)?;
            if let Some(original) = &tx.ethereum_submission {
                anyhow::ensure!(
                    original == &signed,
                    "HOLD: outbound signed submission identity changed for {tx_uuid}"
                );
                anyhow::ensure!(
                    matches!(&tx.status, TxStatus::SendMessage(..)),
                    "HOLD: stale outbound signature handoff"
                );
            } else {
                let TxStatus::PrepareMessage(root, proof) = &tx.status else {
                    anyhow::bail!("HOLD: outbound signature handoff has no durable unsigned intent for {tx_uuid}");
                };
                tx.status = TxStatus::SendMessage(*root, proof.clone());
                tx.remember_attempt(signed.hash);
                tx.ethereum_submission = Some(signed);
            }
        }
        self.storage.save(self).await?;
        Ok(true)
    }

    async fn journal_prepare_message(
        &self,
        tx_uuid: Uuid,
        merkle_root: RelayedMerkleRoot,
        proof: MerkleProof,
    ) -> anyhow::Result<Option<Message>> {
        let message = {
            let mut transactions = self.transactions.write().await;
            let Some(tx) = transactions.get_mut(&tx_uuid) else {
                return Ok(None);
            };
            if let TxStatus::FetchMerkleRoot(expected) = &tx.status {
                anyhow::ensure!(
                    expected == &merkle_root,
                    "HOLD: proof root changed for outbound transaction {tx_uuid}"
                );
            } else {
                log::warn!(
                    "Ignoring stale merkle proof for transaction {tx_uuid} in status {:?}",
                    tx.status
                );
                return Ok(None);
            }
            anyhow::ensure!(
                tx.message
                    .queue_id
                    .is_some_and(|queue| merkle_root.covers(queue, tx.message.block))
                    && proof.root == merkle_root.merkle_root.0,
                "HOLD: proof does not cover the original message QueueId/height/root"
            );
            tx.status = TxStatus::PrepareMessage(merkle_root, proof);
            tx.message.message.clone()
        };
        // This status proves no broadcast occurred; signing requires a separate durable acknowledgement.
        self.storage.save(self).await?;
        Ok(Some(message))
    }

    async fn hold_active_transaction(
        &self,
        tx_uuid: Uuid,
        diagnostic: String,
        nonce_disposition: Option<FinalizedSubmissionSnapshot>,
    ) -> anyhow::Result<bool> {
        let release = {
            let mut transactions = self.transactions.write().await;
            let Some(tx) = transactions.get_mut(&tx_uuid) else {
                log::warn!("Received HOLD for unknown transaction: {tx_uuid}");
                return Ok(false);
            };
            if tx.failure_evidence.is_some() {
                anyhow::bail!(
                    "HOLD: saved failed original evidence cannot be replaced: {diagnostic}"
                );
            }
            if let Some(saved) = &tx.nonce_disposition {
                if saved.legacy().is_some() {
                    let api = self.policy_api.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("HOLD: legacy nonce fence requires Beacon authority")
                    })?;
                    let signed = tx.ethereum_submission.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("HOLD: legacy nonce fence lacks original signed bytes")
                    })?;
                    // Requalify this exact retained fence, never substitute the watcher's newer pin.
                    let qualified = api
                        .verify_finalized_submission_snapshot(&signed.raw_transaction, saved)
                        .await?;
                    if tx.historical_evidence.is_none() {
                        tx.historical_evidence =
                            Some(serde_json::json!({"nonce_disposition": saved}));
                    }
                    tx.nonce_disposition = Some(qualified);
                }
            } else {
                tx.nonce_disposition = nonce_disposition;
            }
            if let TxStatus::WaitConfirmations(hash) = &tx.status {
                let hash = *hash;
                tx.remember_attempt(hash);
            }
            tx.status = TxStatus::NeedsReconciliation {
                diagnostic: diagnostic.clone(),
            };
            tx.nonce_disposition.as_ref().is_some_and(|snapshot| {
                snapshot.beacon().is_some() && snapshot.pinned_nonce_consumed()
            })
        };
        self.storage.save(self).await?;
        log::warn!("HOLD: outbound transaction {tx_uuid}: {diagnostic}");
        Ok(release)
    }

    pub(super) async fn resume(
        &self,
        accumulator: &mut AccumulatorIo,
        proof_fetcher: &mut MerkleRootFetcherIo,
        message_sender: &mut MessageSenderIo,
        status_fetcher: &mut StatusFetcherIo,
    ) -> anyhow::Result<bool> {
        self.hold_ambiguous_send().await?;
        let transactions = self.transactions.write().await;
        for tx in transactions.values().filter(|tx| {
            tx.ethereum_submission.is_some()
                || !tx.ethereum_tx_attempts.is_empty()
                || matches!(tx.status, TxStatus::WaitConfirmations(_))
        }) {
            message_sender.retain_original(tx.uuid);
        }

        // Saved signatures reserve their original nonces before any fresh preparation.
        for tx in resume_order(&transactions) {
            if let Some(signed) = &tx.ethereum_submission {
                if !message_sender.register_original(
                    tx.uuid,
                    tx.message.message.clone(),
                    signed.clone(),
                ) || !status_fetcher.send_request(
                    tx.uuid,
                    signed.hash,
                    tx.message.message.clone(),
                    Some(signed.clone()),
                    tx.lane_policy.clone(),
                ) {
                    return Ok(false);
                }
                continue;
            }
            match tx.status {
                TxStatus::WaitForMerkleRoot => {
                    self.storage
                        .block_storage()
                        .complete_transaction(&tx.message)
                        .await;
                    log::info!(
                        "Transaction {}, nonce={} is waiting for merkle root",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_be)
                    );
                    if !accumulator.send_message(
                        tx.uuid,
                        tx.message.authority_set_id,
                        tx.message
                            .queue_id
                            .ok_or_else(|| anyhow::anyhow!("HOLD: missing message QueueId"))?,
                        None,
                        tx.message.block,
                        tx.message.block_hash,
                        ActorId::from(tx.message.message.source),
                    ) {
                        log::warn!("Accumulator stopped accepting messages, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::FetchMerkleRoot(ref merkle_root) => {
                    log::info!(
                        "Transaction {}, nonce={} is fetching merkle root for block #{}",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_be),
                        merkle_root.block
                    );
                    if !proof_fetcher.send_request(
                        tx.uuid,
                        tx.message.block.0,
                        tx.message_hash,
                        tx.message.message.nonce_be,
                        *merkle_root,
                    ) {
                        log::warn!("Merkle root fetcher stopped accepting requests, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::PrepareMessage(ref root, ref proof)
                | TxStatus::SendMessage(ref root, ref proof) => {
                    if !message_sender.send(
                        tx.message.message.clone(),
                        *root,
                        proof.clone(),
                        tx.uuid,
                        tx.ethereum_submission.clone(),
                        tx.lane_policy.clone(),
                    ) {
                        log::warn!("Message sender stopped accepting requests, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::WaitConfirmations(tx_hash) => {
                    log::info!(
                        "Transaction {}, nonce={} is waiting for confirmations, tx_hash={}",
                        tx.uuid,
                        hex::encode(tx.message.message.nonce_be),
                        tx_hash
                    );
                    if !status_fetcher.send_request(
                        tx.uuid,
                        tx_hash,
                        tx.message.message.clone(),
                        tx.ethereum_submission.clone(),
                        tx.lane_policy.clone(),
                    ) {
                        log::warn!("Status fetcher stopped accepting requests, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::NeedsReconciliation { ref diagnostic }
                | TxStatus::Failed { ref diagnostic } => {
                    log::warn!("HOLD: outbound transaction {}: {diagnostic}", tx.uuid);
                }
                TxStatus::Completed => {}
            }
        }

        Ok(true)
    }

    pub async fn run(
        self,
        mut accumulator: AccumulatorIo,
        mut queued_messages: UnboundedReceiver<MessageInBlock>,
        mut proof_fetcher: MerkleRootFetcherIo,
        mut message_sender: MessageSenderIo,
        mut status_fetcher: StatusFetcherIo,
    ) -> anyhow::Result<()> {
        if !self
            .resume(
                &mut accumulator,
                &mut proof_fetcher,
                &mut message_sender,
                &mut status_fetcher,
            )
            .await?
        {
            log::warn!("Failed to resume transaction manager, exiting");
            return Ok(());
        }

        loop {
            let result = self
                .process(
                    &mut accumulator,
                    &mut queued_messages,
                    &mut proof_fetcher,
                    &mut message_sender,
                    &mut status_fetcher,
                )
                .await;

            self.update_storage().await?;

            match result {
                Ok(false) => {
                    log::warn!("No new transactions to process, exiting");
                    break;
                }

                Ok(true) => continue,
                Err(err) => {
                    log::error!("Transaction manager error: {err}");
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    pub async fn process(
        &self,
        accumulator: &mut AccumulatorIo,
        queued_messages: &mut UnboundedReceiver<MessageInBlock>,
        proof_fetcher: &mut MerkleRootFetcherIo,
        message_sender: &mut MessageSenderIo,
        status_fetcher: &mut StatusFetcherIo,
    ) -> anyhow::Result<bool> {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!("Received Ctrl+C signal, exiting");
                return Ok(false);
            }

            message = queued_messages.recv() => {
                let Some(message) = message else {
                    log::info!("No more messages to process, exiting");
                    return Ok(false);
                };

                let Some(tx) = self.accept_message(message).await? else {
                    return Ok(true);
                };

                let source = ActorId::from(tx.message.message.source);
                let block_hash = tx.message.block_hash;
                let authority_set_id = tx.message.authority_set_id;
                let block = tx.message.block;
                let uuid = tx.uuid;

                log::info!(
                    "New transaction {uuid}, nonce={} from source {source} at block #{} ({block_hash})",
                    hex::encode(tx.message.message.nonce_be),
                    block.0,
                );

                if !accumulator.send_message(uuid, authority_set_id,
                    tx.message.queue_id.ok_or_else(|| anyhow::anyhow!("HOLD: missing message QueueId"))?,
                    None, block, block_hash, source) {
                    log::warn!("Failed to send message to accumulator, exiting");
                    return Ok(false);
                }
            }

            message = accumulator.recv_message() => {
                let Some(message) = message else {
                    log::info!("No more messages from accumulator, exiting");
                    return Ok(false);
                };

                match message {
                    accumulator::Response::Success { tx_uuid, merkle_root, authority_set_id, block } => {
                        if let Some(tx) = self.transactions.write().await.get_mut(&tx_uuid) {
                            if !matches!(&tx.status, TxStatus::WaitForMerkleRoot) {
                                log::warn!("Ignoring stale accumulator response for transaction {tx_uuid}");
                                return Ok(true);
                            }
                            anyhow::ensure!(
                                tx.message.authority_set_id == authority_set_id && tx.message.block == block
                                    && tx.message.queue_id.is_some_and(|queue| merkle_root.covers(queue, block)),
                                "HOLD: accumulator response source block/set mismatch for {tx_uuid}"
                            );
                            tx.status = TxStatus::FetchMerkleRoot(merkle_root);
                            log::info!(
                                "Transaction {} at block #{}({}), hash={}, nonce={} got merkle root {} for block #{}",
                                tx_uuid,
                                tx.message.block.0,
                                tx.message.block_hash,
                                hex::encode(message_hash(&tx.message.message)),
                                hex::encode(tx.message.message.nonce_be),
                                merkle_root.merkle_root,
                                merkle_root.block
                            );
                            if !proof_fetcher.send_request(
                                tx_uuid,
                                tx.message.block.0,
                                tx.message_hash,
                                tx.message.message.nonce_be,
                                merkle_root,
                            ) {
                                log::warn!("Merkle root fetcher stopped accepting requests, exiting");
                                return Ok(false);
                            }
                        } else {
                            log::warn!("Received success response for unknown transaction: {tx_uuid}");
                        }
                    }

                    accumulator::Response::Overflowed(message) => {
                        if self
                            .fail_active_transaction(
                                message.tx_uuid,
                                "Message overflowed".to_string(),
                            )
                            .await
                            .is_none()
                        {
                            log::warn!(
                                "Received overflow response for unknown transaction: {}",
                                message.tx_uuid
                            );
                        }
                    }

                }
            }

            message = proof_fetcher.recv_message() => {
                let Some(message) = message else {
                    log::info!("No more messages from proof fetcher, exiting");
                    return Ok(false);
                };
                let Some(proof) = message.proof else {
                    let retry = {
                        let mut transactions = self.transactions.write().await;
                        if let Some(tx) = transactions.get_mut(&message.tx_uuid) {
                            if matches!(&tx.status, TxStatus::FetchMerkleRoot(root) if *root == message.merkle_root)
                                && tx.ethereum_submission.is_none() {
                                tx.status = TxStatus::WaitForMerkleRoot;
                                Some(tx.clone())
                            } else { None }
                        } else { None }
                    };
                    if let Some(tx) = retry {
                        // Null is unavailable evidence, not authenticated non-membership.
                        self.storage.save(self).await?;
                        anyhow::ensure!(accumulator.send_message(
                            tx.uuid, tx.message.authority_set_id,
                            tx.message.queue_id.ok_or_else(|| anyhow::anyhow!("HOLD: missing message QueueId"))?,
                            Some(message.merkle_root), tx.message.block, tx.message.block_hash,
                            ActorId::from(tx.message.message.source),
                        ), "Accumulator stopped accepting candidate retries");
                    }
                    return Ok(true);
                };

                if let Some(original_message) = self
                    .journal_prepare_message(message.tx_uuid, message.merkle_root, proof.clone())
                    .await?
                {
                    if !message_sender.send(
                        original_message,
                        message.merkle_root,
                        proof,
                        message.tx_uuid,
                        None,
                        self.transactions.read().await.get(&message.tx_uuid).and_then(|tx| tx.lane_policy.clone()),
                    ) {
                        log::warn!("Message sender stopped accepting messages, exiting");
                        return Ok(false);
                    }
                } else {
                    log::warn!("Received merkle proof for unknown transaction: {}", message.tx_uuid);
                }
            }

            message = message_sender.recv() => {
                let Some(message) = message else {
                    log::info!("No more messages from message sender, exiting");
                    return Ok(false);
                };

                match message {
                    message_sender::Response::Prepared { tx_uuid, submission, durable } => {
                        if self.journal_signed_submission(tx_uuid, submission).await? {
                            message_sender.retain_original(tx_uuid);
                            let _ = durable.send(());
                        }
                    }
                    message_sender::Response::ProcessingStarted(tx_hash, tx_uuid) => {
                        let original = {
                            let mut transactions = self.transactions.write().await;
                            transactions.get_mut(&tx_uuid).and_then(|tx| {
                                if !matches!(&tx.status, TxStatus::SendMessage(..)) {
                                    return None;
                                }
                                if tx.ethereum_submission.as_ref().is_none_or(|saved| saved.hash != tx_hash) {
                                    return None;
                                }
                                tx.remember_attempt(tx_hash);
                                tx.status = TxStatus::WaitConfirmations(tx_hash);
                                Some((tx.message.message.clone(), tx.ethereum_submission.clone(), tx.lane_policy.clone()))
                            })
                        };
                        if let Some((message, submission, policy)) = original {
                            self.storage.save(self).await?;
                            if !status_fetcher.send_request(tx_uuid, tx_hash, message, submission, policy) {
                                log::warn!("Status fetcher stopped accepting requests, exiting");
                                return Ok(false);
                            }
                        } else {
                            log::warn!("Received message for unknown transaction: {tx_uuid}");
                        }
                    }

                    message_sender::Response::Failed(tx_uuid, error) => {
                        if let Some(nonce) = self.fail_active_transaction(tx_uuid, error.clone()).await {
                            self.storage.save(self).await?;
                            log::error!(
                                "Transaction {tx_uuid}, nonce={} failed before broadcast: {error}",
                                hex::encode(nonce)
                            );
                        } else {
                            log::warn!("Received failure for unknown transaction: {tx_uuid}");
                        }
                    }
                    message_sender::Response::Hold(tx_uuid, reason) => {
                        self.hold_active_transaction(tx_uuid, reason, None).await?;
                    }
                }
            }

            message = status_fetcher.recv_message() => {
                let Some(status) = message else {
                    log::info!("No more messages from status fetcher, exiting");
                    return Ok(false);
                };

                match status {
                    (uuid, status_fetcher::SubmissionOutcome::Completed(evidence)) => {
                        if self.complete_transaction(uuid, evidence).await? {
                            self.storage.save(self).await?;
                            self.ack_completed_event(uuid).await?;
                            message_sender.reconcile_original(uuid);
                            log::info!(
                                "Transaction {uuid} completed with original finalized receipt and matching effects"
                            );
                        } else {
                            log::warn!("Received success response for unknown transaction: {uuid}");
                        }
                    }

                    (uuid, status_fetcher::SubmissionOutcome::FinalizedRevert(evidence)) => {
                        if self.record_finalized_failure(uuid, evidence).await? {
                            message_sender.reconcile_original(uuid);
                        }
                    }
                    (uuid, status_fetcher::SubmissionOutcome::NeedsReconciliation { diagnostic, nonce_disposition }) => {
                        let release = self.hold_active_transaction(uuid, diagnostic, nonce_disposition).await?;
                        if release { message_sender.reconcile_original(uuid); }
                    }
                }
            }
        }

        Ok(true)
    }
}

fn resume_order(transactions: &BTreeMap<Uuid, Transaction>) -> Vec<&Transaction> {
    let mut ordered: Vec<_> = transactions.values().collect();
    // Mining waits for lower nonces, including originals the RPC mempool forgot.
    ordered.sort_unstable_by_key(|tx| match &tx.ethereum_submission {
        Some(signed) => (false, signed.nonce),
        None => (true, 0),
    });
    ordered
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::message_relayer::{
        common::{AuthoritySetId, GearBlockNumber},
        gear_to_eth::storage::{JSONStorage, NoStorage, OutboundLaneIdentity},
    };
    use alloy::{
        consensus::{SignableTransaction, TxEip1559, TxEnvelope},
        eips::Encodable2718,
        network::TxSigner,
        primitives::{Address, Bytes, TxKind, B256, U256},
        signers::local::PrivateKeySigner,
        sol_types::SolCall,
    };
    use ethereum_client::abi::IMessageQueue::{self, VaraMessage};
    use gear_rpc_client::dto::Message;
    use primitive_types::H256;

    fn test_signer() -> PrivateKeySigner {
        PrivateKeySigner::from_bytes(&B256::from([7; 32])).unwrap()
    }

    pub(crate) fn policy(
        message: &Message,
        signed: &PreparedContentMessage,
        token_delivery: bool,
    ) -> LanePolicy {
        status_fetcher::tests::lane_policy(message, signed, B256::repeat_byte(1), token_delivery)
    }

    async fn approved_manager(
        storage: Arc<dyn Storage>,
        message: &Message,
    ) -> (status_fetcher::tests::WatchRpc, TransactionManager) {
        let rpc = status_fetcher::tests::watch_rpc_with_policy(false, false)
            .await
            .unwrap();
        let signed = status_fetcher::tests::signed_message(message).await;
        let policy = status_fetcher::tests::lane_policy(
            message,
            &signed,
            rpc.api.finality_identity().unwrap().1,
            false,
        );
        let manager = TransactionManager::new(storage)
            .with_lane_approvals(rpc.api.clone(), LaneApprovals::from_policy(policy));
        (rpc, manager)
    }

    #[tokio::test]
    async fn legacy_nonce_fence_is_qualified_and_saved_before_lane_release() {
        let path =
            std::env::temp_dir().join(format!("gear-nonce-qualification-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let rpc = status_fetcher::tests::watch_rpc_with_policy(false, false)
            .await
            .unwrap();
        rpc.advance_finalized_to(101);
        let mut tx = transaction();
        let signed = status_fetcher::tests::signed_message(&tx.message.message).await;
        let qualified = rpc
            .api
            .get_finalized_submission_snapshot(&signed.raw_transaction)
            .await
            .unwrap()
            .unwrap();
        assert!(qualified.pinned_nonce_consumed());
        let legacy = serde_json::json!({
            "block_number": qualified.block_number(),
            "block_hash": qualified.block_hash(),
            "pinned_nonce_consumed": true,
            "message_processed": false
        });
        let saved: FinalizedSubmissionSnapshot = serde_json::from_value(legacy.clone()).unwrap();
        tx.journal_version = 5;
        tx.status = TxStatus::NeedsReconciliation {
            diagnostic: "retained legacy fence".into(),
        };
        tx.remember_attempt(signed.hash);
        tx.nonce_disposition = Some(saved);
        tx.lane_policy = Some(status_fetcher::tests::lane_policy(
            &tx.message.message,
            &signed,
            rpc.api.finality_identity().unwrap().1,
            false,
        ));
        tx.ethereum_submission = Some(signed);
        let uuid = tx.uuid;
        let manager = TransactionManager::new(storage).with_lane_approvals(
            rpc.api.clone(),
            LaneApprovals::from_policy(tx.lane_policy.clone().unwrap()),
        );
        manager.add_transaction(tx).await;
        manager.update_storage().await.unwrap();
        // The incoming pin is deliberately invalid: only the exact saved fence may authorize release.
        let mut newer = serde_json::to_value(&qualified).unwrap();
        newer["block_number"] = serde_json::json!(qualified.block_number() + 1);
        let newer = serde_json::from_value(newer).unwrap();
        assert!(manager
            .hold_active_transaction(uuid, "watcher advanced".into(), Some(newer))
            .await
            .unwrap());
        let persisted: Transaction =
            serde_json::from_slice(&tokio::fs::read(path.join(uuid.to_string())).await.unwrap())
                .unwrap();
        let snapshot = persisted.nonce_disposition.as_ref().unwrap();
        assert!(snapshot.beacon().is_some());
        assert_eq!(snapshot.block_number(), qualified.block_number());
        assert_eq!(snapshot.block_hash(), qualified.block_hash());
        assert_eq!(
            persisted.historical_evidence.unwrap()["nonce_disposition"],
            legacy
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_lane_holds_without_creating_a_journal_or_signature() {
        let path = std::env::temp_dir().join(format!("gear-unknown-lane-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let rpc = status_fetcher::tests::watch_rpc_with_policy(false, false)
            .await
            .unwrap();
        let manager = TransactionManager::new(storage.clone())
            .with_lane_approvals(rpc.api.clone(), LaneApprovals::default());
        let message = transaction().message;
        let error = manager.accept_message(message.clone()).await.err().unwrap();
        assert!(error.to_string().contains("no independently approved"));
        assert!(manager.transactions.read().await.is_empty());
        assert!(!storage
            .outbound_nonce_owned(message.message.nonce_be)
            .await
            .unwrap());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn historical_original_requires_exact_approval_and_retains_immutable_evidence() {
        let path = std::env::temp_dir().join(format!("gear-historical-policy-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let rpc = status_fetcher::tests::watch_rpc_with_policy(false, false)
            .await
            .unwrap();
        let mut tx = transaction();
        let signed = signed_content(&tx, 36).await;
        tx.journal_version = 5;
        tx.status = TxStatus::WaitConfirmations(signed.hash);
        tx.remember_attempt(signed.hash);
        tx.ethereum_submission = Some(signed.clone());
        let uuid = tx.uuid;
        let original = serde_json::to_value(&tx).unwrap();
        let manager = TransactionManager::new(storage.clone());
        manager.add_transaction(tx.clone()).await;
        manager.update_storage().await.unwrap();
        let journal = path.join(uuid.to_string());
        let bytes = tokio::fs::read(&journal).await.unwrap();
        let mut policy = status_fetcher::tests::lane_policy(
            &tx.message.message,
            &signed,
            rpc.api.finality_identity().unwrap().1,
            false,
        );
        for original_hash in [None, Some(B256::repeat_byte(99))] {
            policy.original_hash = original_hash;
            let rejected = TransactionManager::new(storage.clone())
                .with_lane_approvals(rpc.api.clone(), LaneApprovals::from_policy(policy.clone()));
            rejected.load_from_storage().await.unwrap();
            assert!(rejected.verify_completed(&rpc.api).await.is_err());
            assert_eq!(tokio::fs::read(&journal).await.unwrap(), bytes);
        }
        policy.original_hash = Some(signed.hash);
        let approved = TransactionManager::new(storage.clone())
            .with_lane_approvals(rpc.api.clone(), LaneApprovals::from_policy(policy.clone()));
        approved.load_from_storage().await.unwrap();
        approved.verify_completed(&rpc.api).await.unwrap();
        let saved: Transaction =
            serde_json::from_slice(&tokio::fs::read(&journal).await.unwrap()).unwrap();
        assert_eq!(saved.journal_version, 6);
        assert_eq!(saved.lane_policy, Some(policy.clone()));
        assert!(saved.ethereum_submission == Some(signed));
        assert_eq!(saved.historical_evidence, Some(original));
        let qualified_bytes = tokio::fs::read(&journal).await.unwrap();
        policy.approval_sha256 = B256::repeat_byte(99);
        let changed_approval = TransactionManager::new(storage)
            .with_lane_approvals(rpc.api.clone(), LaneApprovals::from_policy(policy));
        changed_approval.load_from_storage().await.unwrap();
        assert!(changed_approval.verify_completed(&rpc.api).await.is_err());
        assert_eq!(tokio::fs::read(&journal).await.unwrap(), qualified_bytes);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn completed_journal_rejects_policy_downgrade_even_with_valid_generic_effects() {
        let mut tx = transaction();
        tx.message.message.payload = vec![6; 104];
        tx.message_hash = message_hash(&tx.message.message);
        let signed = signed_content(&tx, 36).await;
        tx.status = TxStatus::Completed;
        tx.remember_attempt(signed.hash);
        tx.ethereum_tx_hash = Some(signed.hash);
        tx.completion = Some(status_fetcher::tests::evidence(
            &tx.message.message,
            &signed,
            false,
        ));
        tx.lane_policy = Some(policy(&tx.message.message, &signed, true));
        tx.ethereum_submission = Some(signed);
        assert!(tx
            .validate_journal()
            .unwrap_err()
            .to_string()
            .contains("token effect"));
    }

    fn inclusion(tx: &Transaction) -> (RelayedMerkleRoot, MerkleProof) {
        let root = RelayedMerkleRoot {
            queue_id: Some(0),
            block: GearBlockNumber(43),
            block_hash: H256::repeat_byte(8),
            timestamp: 123,
            authority_set_id: AuthoritySetId(6),
            merkle_root: H256(tx.message_hash),
        };
        let proof = MerkleProof {
            root: tx.message_hash,
            proof: vec![],
            num_leaves: 1,
            leaf_index: 0,
        };
        (root, proof)
    }

    async fn signed_content(tx: &Transaction, nonce: u64) -> PreparedContentMessage {
        let message = &tx.message.message;
        let input = IMessageQueue::processMessageCall {
            blockNumber: U256::from(43),
            totalLeaves: U256::from(1),
            leafIndex: U256::ZERO,
            message: VaraMessage {
                nonce: U256::from_be_bytes(message.nonce_be),
                source: B256::from(message.source),
                destination: Address::from(message.destination),
                payload: Bytes::from(message.payload.clone()),
            },
            proof: vec![],
        }
        .abi_encode();
        let signer = test_signer();
        let contract = Address::from([2; 20]);
        let mut transaction = TxEip1559 {
            chain_id: 31337,
            nonce,
            gas_limit: 100_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(contract),
            input: Bytes::from(input),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut transaction).await.unwrap();
        let raw_transaction =
            TxEnvelope::Eip1559(transaction.into_signed(signature)).encoded_2718();
        PreparedContentMessage {
            chain_id: 31337,
            contract,
            sender: signer.address(),
            nonce,
            hash: alloy::primitives::keccak256(&raw_transaction),
            raw_transaction,
        }
    }

    fn transaction() -> Transaction {
        Transaction::new(
            MessageInBlock {
                queue_id: Some(0),
                message: Message {
                    nonce_be: [7; 32],
                    source: [1; 32],
                    destination: [2; 20],
                    payload: vec![3, 4],
                },
                block: GearBlockNumber(42),
                block_hash: H256::from_low_u64_be(99),
                authority_set_id: AuthoritySetId(5),
            },
            TxStatus::WaitForMerkleRoot,
        )
    }

    async fn bound_storage(path: &std::path::Path) -> Arc<JSONStorage> {
        let storage = Arc::new(JSONStorage::new(path));
        storage
            .bind_outbound_lane(OutboundLaneIdentity {
                destination_chain_id: 31337,
                destination_genesis_hash: H256::repeat_byte(1),
                message_queue_address: primitive_types::H160::repeat_byte(2),
                bridging_payment_address: None,
                fee_exempt_sources: Default::default(),
                sender_address: primitive_types::H160::from_slice(
                    test_signer().address().as_slice(),
                ),
            })
            .await
            .unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(9), 42)
            .await
            .unwrap();
        storage
    }

    #[tokio::test]
    async fn legacy_terminal_records_require_qualification_before_version_five() {
        let mut tx = transaction();
        let signed = signed_content(&tx, 36).await;
        tx.remember_attempt(signed.hash);
        tx.ethereum_submission = Some(signed.clone());
        tx.status = TxStatus::Failed {
            diagnostic: "saved legacy failure".into(),
        };
        tx.journal_version = 4;
        tx.validate_journal().unwrap();
        tx.journal_version = 5;
        assert!(
            tx.validate_journal().is_err(),
            "signed failure needs original reverted inclusion"
        );
        tx.failure_evidence = Some(status_fetcher::tests::reverted_evidence(
            &tx.message.message,
            &signed,
        ));
        tx.validate_journal().unwrap();
        tx.failure_evidence.as_mut().unwrap().checkpoint = None;
        assert!(
            tx.validate_journal().is_err(),
            "partial authentication cannot qualify a failure"
        );

        tx.failure_evidence = None;
        tx.status = TxStatus::Completed;
        tx.ethereum_tx_hash = Some(signed.hash);
        tx.completion = Some(status_fetcher::tests::evidence(
            &tx.message.message,
            &signed,
            false,
        ));
        tx.validate_journal().unwrap();
        let evidence = tx.completion.as_mut().unwrap();
        evidence.inclusion = None;
        evidence.checkpoint = None;
        assert!(
            tx.validate_journal().is_err(),
            "version five cannot accept legacy completion"
        );
        for version in 2..=4 {
            tx.journal_version = version;
            tx.validate_journal().unwrap();
        }
    }

    #[tokio::test]
    async fn failed_original_is_durable_counted_once_and_never_reprepared() {
        let path = std::env::temp_dir().join(format!("gear-original-revert-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let mut tx = transaction();
        let signed = signed_content(&tx, 36).await;
        let evidence = status_fetcher::tests::reverted_evidence(&tx.message.message, &signed);
        tx.status = TxStatus::WaitConfirmations(signed.hash);
        tx.lane_policy = Some(policy(&tx.message.message, &signed, false));
        tx.ethereum_submission = Some(signed.clone());
        tx.remember_attempt(signed.hash);
        let uuid = tx.uuid;
        let message = tx.message.clone();
        manager.add_transaction(tx).await;
        manager.update_storage().await.unwrap();
        let snapshot: FinalizedSubmissionSnapshot = serde_json::from_value(serde_json::json!({
            "block_number": 107,
            "block_hash": alloy::primitives::B256::from([4; 32]),
            "pinned_nonce_consumed": true,
            "message_processed": true
        }))
        .unwrap();
        manager
            .hold_active_transaction(
                uuid,
                "original absent; competitor processed".into(),
                Some(snapshot.clone()),
            )
            .await
            .unwrap();
        assert_eq!(manager.metrics.failed_transactions.get(), 0);
        assert!(manager.completed.read().await.is_empty());
        assert!(manager
            .record_finalized_failure(uuid, evidence.clone())
            .await
            .unwrap());
        assert!(manager
            .record_finalized_failure(uuid, evidence)
            .await
            .unwrap());
        assert_eq!(manager.metrics.failed_transactions.get(), 1);
        let restored = TransactionManager::new(storage);
        restored.load_from_storage().await.unwrap();
        assert!(restored.accept_message(message).await.unwrap().is_none());
        let active = restored.transactions.read().await;
        let tx = &active[&uuid];
        assert!(tx.ethereum_submission.as_ref() == Some(&signed));
        assert!(matches!(tx.status, TxStatus::Failed { .. }));
        assert_eq!(
            tx.failure_evidence
                .as_ref()
                .unwrap()
                .receipt
                .transaction_hash,
            signed.hash
        );
        assert_eq!(
            serde_json::to_value(&tx.nonce_disposition).unwrap(),
            serde_json::to_value(Some(snapshot)).unwrap()
        );
        assert!(tx.ethereum_tx_hash.is_none() && tx.completion.is_none());
        drop(active);
        assert!(restored.completed.read().await.is_empty());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn ineligible_operator_handoff_keeps_journal_clean_then_valid_pair_admits_once() {
        for queued_before_request in [false, true] {
            let path =
                std::env::temp_dir().join(format!("gear-operator-admission-{}", Uuid::new_v4()));
            let storage = Arc::new(JSONStorage::new(&path));
            storage
                .bind_outbound_lane(OutboundLaneIdentity {
                    destination_chain_id: 31337,
                    destination_genesis_hash: H256::repeat_byte(1),
                    message_queue_address: primitive_types::H160::repeat_byte(2),
                    bridging_payment_address: Some(H256::repeat_byte(3)),
                    fee_exempt_sources: Default::default(),
                    sender_address: primitive_types::H160::repeat_byte(4),
                })
                .await
                .unwrap();
            storage
                .bind_event_chain(H256::repeat_byte(9), 42)
                .await
                .unwrap();
            storage
                .record_queued_block(42, H256::repeat_byte(42), &[])
                .await
                .unwrap();
            storage
                .record_paid_block(42, H256::repeat_byte(42), &[])
                .await
                .unwrap();
            let mut message = transaction().message;
            message.block = GearBlockNumber(43);
            message.block_hash = H256::repeat_byte(43);
            if queued_before_request {
                storage
                    .record_queued_block(43, message.block_hash, std::slice::from_ref(&message))
                    .await
                    .unwrap();
            }
            let (_rpc, manager) = approved_manager(storage.clone(), &message.message).await;
            manager.update_storage().await.unwrap();
            let mut before = BTreeMap::new();
            for name in [
                "gear_events.json",
                "failed",
                "blocks.json",
                "transaction_status.json",
            ] {
                before.insert(name, tokio::fs::read(path.join(name)).await.unwrap());
            }
            assert!(manager
                .accept_message(message.clone())
                .await
                .unwrap()
                .is_none());
            assert!(manager.transactions.read().await.is_empty());
            assert!(manager.completed.read().await.is_empty());
            for (name, bytes) in before {
                assert_eq!(
                    tokio::fs::read(path.join(name)).await.unwrap(),
                    bytes,
                    "ineligible request must not write {name}"
                );
            }
            assert!(!path.join(".state/save.pending").exists());
            assert!(tokio::fs::read_dir(path.join(".state"))
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none());
            if !queued_before_request {
                storage
                    .record_queued_block(43, message.block_hash, std::slice::from_ref(&message))
                    .await
                    .unwrap();
            }
            storage
                .record_paid_block(43, message.block_hash, &[message.message.nonce_be])
                .await
                .unwrap();
            let accepted = manager
                .accept_message(message.clone())
                .await
                .unwrap()
                .unwrap();
            let original = tokio::fs::read(path.join(accepted.uuid.to_string()))
                .await
                .unwrap();
            assert_eq!(accepted.message, message);
            assert!(manager.accept_message(message).await.unwrap().is_none());
            assert_eq!(manager.transactions.read().await.len(), 1);
            assert_eq!(
                tokio::fs::read(path.join(accepted.uuid.to_string()))
                    .await
                    .unwrap(),
                original
            );
            assert!(!path.join(".state/save.pending").exists());
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn manual_all_token_and_exempt_admission_preserve_no_payment_policy() {
        for policy in ["manual", "all-token", "exempt"] {
            let path =
                std::env::temp_dir().join(format!("gear-no-payment-admission-{}", Uuid::new_v4()));
            let message = transaction().message;
            let storage = Arc::new(JSONStorage::new(&path));
            storage
                .bind_outbound_lane(OutboundLaneIdentity {
                    destination_chain_id: 31337,
                    destination_genesis_hash: H256::repeat_byte(1),
                    message_queue_address: primitive_types::H160::repeat_byte(2),
                    bridging_payment_address: (policy == "exempt").then(|| H256::repeat_byte(3)),
                    fee_exempt_sources: if policy == "exempt" {
                        [message.message.source].into_iter().collect()
                    } else {
                        Default::default()
                    },
                    sender_address: primitive_types::H160::repeat_byte(4),
                })
                .await
                .unwrap();
            storage
                .bind_event_chain(H256::repeat_byte(9), message.block.0)
                .await
                .unwrap();
            if policy != "manual" {
                storage
                    .record_queued_block(
                        message.block.0,
                        message.block_hash,
                        std::slice::from_ref(&message),
                    )
                    .await
                    .unwrap();
            }
            let (_rpc, manager) = approved_manager(storage.clone(), &message.message).await;
            let accepted = manager
                .accept_message(message.clone())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(accepted.message, message);
            assert!(storage.paid_observations().await.unwrap().is_empty());
            assert!(storage
                .outbound_nonce_owned(message.message.nonce_be)
                .await
                .unwrap());
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn complete_transaction_moves_active_transaction_to_completed() {
        let manager = TransactionManager::new(Arc::new(NoStorage::new()));
        let mut tx = transaction();
        let uuid = tx.uuid;
        let message_hash = tx.message_hash;
        let signed = status_fetcher::tests::signed_message(&tx.message.message).await;
        let tx_hash = signed.hash;
        let evidence = status_fetcher::tests::evidence(&tx.message.message, &signed, false);
        tx.lane_policy = Some(policy(&tx.message.message, &signed, false));
        tx.ethereum_submission = Some(signed);
        tx.remember_attempt(tx_hash);
        tx.status = TxStatus::WaitConfirmations(tx_hash);

        manager.add_transaction(tx).await;
        manager
            .failed
            .write()
            .await
            .insert(uuid, "previous attempt failed".to_string());

        assert!(manager
            .complete_transaction(uuid, evidence.clone())
            .await
            .unwrap());
        assert!(!manager.transactions.read().await.contains_key(&uuid));
        assert!(!manager.failed.read().await.contains_key(&uuid));

        let completed = manager.completed.read().await;
        let completed_tx = completed.get(&uuid).unwrap();
        assert_eq!(completed_tx.message_hash, message_hash);
        assert_eq!(completed_tx.ethereum_tx_hash, Some(tx_hash));
        assert!(matches!(completed_tx.status, TxStatus::Completed));
        drop(completed);

        assert!(!manager
            .complete_transaction(uuid, evidence.clone())
            .await
            .unwrap());
        assert_eq!(manager.metrics.completed_transactions.get(), 1);
    }

    #[tokio::test]
    async fn existing_transactions_are_classified_for_duplicate_suppression() {
        let manager = TransactionManager::new(Arc::new(NoStorage::new()));
        let mut tx = transaction();
        let uuid = tx.uuid;
        let message = tx.message.clone();
        let signed = status_fetcher::tests::signed_message(&tx.message.message).await;
        let evidence = status_fetcher::tests::evidence(&tx.message.message, &signed, false);
        tx.status = TxStatus::WaitConfirmations(signed.hash);
        tx.remember_attempt(signed.hash);
        tx.lane_policy = Some(policy(&tx.message.message, &signed, false));
        tx.ethereum_submission = Some(signed);

        manager.add_transaction(tx).await;
        assert!(matches!(
            manager.existing_transaction(&message).await.unwrap(),
            Some(ExistingTransaction::Active(_))
        ));

        let mut conflict = message.clone();
        conflict.message.payload.push(0xff);
        assert!(manager
            .existing_transaction(&conflict)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("HOLD"));
        assert!(manager.complete_transaction(uuid, evidence).await.unwrap());
        assert!(matches!(
            manager.existing_transaction(&message).await.unwrap(),
            Some(ExistingTransaction::Completed)
        ));
        let mut conflict = message.clone();
        conflict.block_hash = H256::repeat_byte(0xff);
        assert!(manager
            .existing_transaction(&conflict)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("HOLD"));
        let mut unseen = message;
        unseen.message.nonce_be = [0; 32];
        assert!(manager
            .existing_transaction(&unseen)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn terminal_failure_keeps_full_message_and_blocks_replay_after_restart() {
        let path = std::env::temp_dir().join(format!("gear-outbound-failed-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let tx = transaction();
        let uuid = tx.uuid;
        let message = tx.message.clone();
        storage
            .bind_event_chain(H256::repeat_byte(9), tx.message.block.0)
            .await
            .unwrap();
        storage
            .record_queued_block(
                tx.message.block.0,
                tx.message.block_hash,
                std::slice::from_ref(&tx.message),
            )
            .await
            .unwrap();
        storage
            .record_paid_block(
                tx.message.block.0,
                H256::repeat_byte(8),
                &[tx.message.message.nonce_be],
            )
            .await
            .unwrap();
        manager.add_transaction(tx).await;
        assert!(manager
            .fail_active_transaction(uuid, "terminal failure".into())
            .await
            .is_some());
        manager.update_storage().await.unwrap();
        let view: serde_json::Value = serde_json::from_slice(
            &tokio::fs::read(path.join("transaction_status.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(view["active"][uuid.to_string()], "Failed");
        assert_eq!(view["failed"], serde_json::json!([uuid]));

        let restored = TransactionManager::new(storage.clone());
        restored.load_from_storage().await.unwrap();
        assert!(matches!(
            restored.existing_transaction(&message).await.unwrap(),
            Some(ExistingTransaction::Active(_))
        ));
        assert!(matches!(
            &restored.transactions.read().await[&uuid].status,
            TxStatus::Failed { .. }
        ));
        assert_eq!(storage.pending_event_pairs().await.unwrap().len(), 1);
        assert_eq!(
            restored.failed.read().await.get(&uuid).map(String::as_str),
            Some("terminal failure")
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn every_active_status_suppresses_duplicate_submission() {
        let root = RelayedMerkleRoot {
            queue_id: Some(0),
            block: GearBlockNumber(43),
            block_hash: H256::from_low_u64_be(100),
            timestamp: 123,
            authority_set_id: AuthoritySetId(6),
            merkle_root: H256::from_low_u64_be(101),
        };
        let proof = MerkleProof {
            root: [0; 32],
            proof: vec![],
            num_leaves: 1,
            leaf_index: 0,
        };
        let statuses = [
            TxStatus::WaitForMerkleRoot,
            TxStatus::FetchMerkleRoot(root),
            TxStatus::PrepareMessage(root, proof.clone()),
            TxStatus::SendMessage(root, proof),
            TxStatus::WaitConfirmations(TxHash::from([0; 32])),
            TxStatus::NeedsReconciliation {
                diagnostic: "unsettled".into(),
            },
            TxStatus::Failed {
                diagnostic: "failed".into(),
            },
            TxStatus::Completed,
        ];

        for status in statuses {
            let manager = TransactionManager::new(Arc::new(NoStorage::new()));
            let mut tx = transaction();
            tx.status = status;
            let message = tx.message.clone();
            manager.transactions.write().await.insert(tx.uuid, tx);

            assert!(matches!(
                manager.existing_transaction(&message).await.unwrap(),
                Some(ExistingTransaction::Active(_))
            ));
            for changed in ["payload", "block", "hash", "authority"] {
                let mut conflict = message.clone();
                match changed {
                    "payload" => conflict.message.payload.push(0xff),
                    "block" => conflict.block = GearBlockNumber(43),
                    "hash" => conflict.block_hash = H256::repeat_byte(0xff),
                    "authority" => conflict.authority_set_id = AuthoritySetId(6),
                    _ => unreachable!(),
                }
                assert!(
                    manager
                        .existing_transaction(&conflict)
                        .await
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("HOLD"),
                    "{changed}"
                );
            }
        }
    }
    #[tokio::test]
    async fn persisted_send_message_holds_before_sender_start_without_changing_its_journal() {
        let path = std::env::temp_dir().join(format!("gear-outbound-ambiguous-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let mut tx = transaction();
        let root = RelayedMerkleRoot {
            queue_id: Some(0),
            block: GearBlockNumber(43),
            block_hash: H256::from_low_u64_be(100),
            timestamp: 123,
            authority_set_id: AuthoritySetId(6),
            merkle_root: H256::from_low_u64_be(101),
        };
        let proof = MerkleProof {
            root: [0; 32],
            proof: vec![[0; 32]],
            num_leaves: 1,
            leaf_index: 0,
        };
        tx.status = TxStatus::SendMessage(root, proof);
        let committed_path = path.join(tx.uuid.to_string());
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        manager.update_storage().await.unwrap();
        let expected = serde_json::to_value(&manager.transactions.read().await[&uuid]).unwrap();
        let committed = tokio::fs::read(&committed_path).await.unwrap();

        let restored = TransactionManager::new(storage);
        let error = restored.load_from_storage().await.unwrap_err().to_string();
        assert!(error.contains("HOLD"));
        assert!(error.contains(&uuid.to_string()));
        assert_eq!(restored.transactions.read().await.len(), 1);
        assert_eq!(
            serde_json::to_value(&restored.transactions.read().await[&uuid]).unwrap(),
            expected
        );
        assert!(restored.failed.read().await.is_empty());
        assert_eq!(tokio::fs::read(committed_path).await.unwrap(), committed);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn nonce_only_historical_completion_holds_without_rewriting_journal() {
        let path =
            std::env::temp_dir().join(format!("gear-outbound-old-completed-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let mut tx = transaction();
        tx.status = TxStatus::Completed;
        tx.journal_version = 2;
        tx.ethereum_tx_attempts.push(TxHash::from([0x11; 32]));
        let journal = path.join(tx.uuid.to_string());
        let bytes = serde_json::to_vec(&tx).unwrap();
        tokio::fs::write(&journal, &bytes).await.unwrap();
        let restored = TransactionManager::new(storage);
        let error = restored.load_from_storage().await.unwrap_err();
        assert!(
            format!("{error:#}").contains("HOLD: completed journal lacks original signed bytes")
        );
        assert_eq!(tokio::fs::read(&journal).await.unwrap(), bytes);
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn dropped_attempt_holds_with_original_hash_and_never_requeues_message() {
        let path = std::env::temp_dir().join(format!("gear-outbound-dropped-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let mut tx = transaction();
        let uuid = tx.uuid;
        let message = tx.message.clone();
        let original = TxHash::from([0x22; 32]);
        tx.status = TxStatus::WaitConfirmations(original);
        manager.add_transaction(tx).await;
        manager.update_storage().await.unwrap();
        manager
            .hold_active_transaction(uuid, "no canonical receipt yet".into(), None)
            .await
            .unwrap();

        let restored = TransactionManager::new(storage.clone());
        restored.load_from_storage().await.unwrap();
        let active = restored.transactions.read().await;
        assert!(matches!(
            &active[&uuid].status,
            TxStatus::NeedsReconciliation { .. }
        ));
        assert_eq!(active[&uuid].ethereum_tx_attempts, vec![original]);
        assert!(active[&uuid].ethereum_tx_hash.is_none());
        drop(active);
        assert!(matches!(
            restored.existing_transaction(&message).await.unwrap(),
            Some(ExistingTransaction::Active(_))
        ));
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn stale_proof_cannot_revive_held_failed_or_already_sent_transfer() {
        let manager = TransactionManager::new(Arc::new(NoStorage::new()));
        let root = RelayedMerkleRoot {
            queue_id: Some(0),
            block: GearBlockNumber(43),
            block_hash: H256::from_low_u64_be(100),
            timestamp: 123,
            authority_set_id: AuthoritySetId(6),
            merkle_root: H256::from_low_u64_be(101),
        };
        let proof = MerkleProof {
            root: [0; 32],
            proof: vec![[0; 32]],
            num_leaves: 1,
            leaf_index: 0,
        };
        for status in [
            TxStatus::Failed {
                diagnostic: "terminal failure".into(),
            },
            TxStatus::NeedsReconciliation {
                diagnostic: "uncertain nonce".into(),
            },
            TxStatus::SendMessage(root, proof.clone()),
        ] {
            let mut tx = transaction();
            tx.status = status;
            let uuid = tx.uuid;
            let before = serde_json::to_value(&tx).unwrap();
            manager.add_transaction(tx).await;
            assert!(manager
                .journal_prepare_message(uuid, root, proof.clone())
                .await
                .unwrap()
                .is_none());
            assert_eq!(
                serde_json::to_value(&manager.transactions.read().await[&uuid]).unwrap(),
                before
            );
        }

        let mut tx = transaction();
        tx.status = TxStatus::FetchMerkleRoot(root);
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        let wrong_root = RelayedMerkleRoot {
            queue_id: Some(0),
            merkle_root: H256::repeat_byte(0xff),
            ..root
        };
        assert!(manager
            .journal_prepare_message(uuid, wrong_root, proof)
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
        assert!(
            matches!(&manager.transactions.read().await[&uuid].status, TxStatus::FetchMerkleRoot(remaining) if remaining == &root)
        );
    }

    /// Reconstructed from supplied HTML VB-15 / audited 7fe1b032.
    #[tokio::test]
    async fn proof_handoff_requires_original_queue_and_covered_height() {
        let manager = TransactionManager::new(Arc::new(NoStorage::new()));
        let original = transaction();
        let (root, proof) = inclusion(&original);
        for candidate in [
            RelayedMerkleRoot {
                queue_id: Some(1),
                ..root
            },
            RelayedMerkleRoot {
                queue_id: None,
                ..root
            },
            RelayedMerkleRoot {
                block: GearBlockNumber(original.message.block.0 - 1),
                ..root
            },
        ] {
            let mut tx = original.clone();
            tx.status = TxStatus::FetchMerkleRoot(candidate);
            let before = serde_json::to_value(&tx).unwrap();
            manager.add_transaction(tx).await;
            assert!(manager
                .journal_prepare_message(original.uuid, candidate, proof.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("QueueId"));
            assert_eq!(
                serde_json::to_value(&manager.transactions.read().await[&original.uuid]).unwrap(),
                before
            );
        }
        let mut tx = original.clone();
        let later = RelayedMerkleRoot {
            block: GearBlockNumber(root.block.0 + 1),
            ..root
        };
        tx.status = TxStatus::FetchMerkleRoot(later);
        manager.add_transaction(tx).await;
        assert_eq!(
            manager
                .journal_prepare_message(original.uuid, later, proof)
                .await
                .unwrap(),
            Some(original.message.message)
        );
    }

    #[tokio::test]
    async fn restored_originals_resume_by_account_nonce_before_unsigned_work() {
        let mut transactions = BTreeMap::new();
        for (id, nonce) in [(1, None), (2, Some(37)), (3, Some(36))] {
            let mut message = transaction().message;
            message.message.nonce_be = [id as u8; 32];
            let mut tx = Transaction::new(message, TxStatus::WaitForMerkleRoot);
            tx.uuid = Uuid::from_u128(id);
            let (root, proof) = inclusion(&tx);
            tx.status = TxStatus::PrepareMessage(root, proof.clone());
            if let Some(nonce) = nonce {
                let signed = signed_content(&tx, nonce).await;
                tx.ethereum_tx_attempts = vec![signed.hash];
                tx.ethereum_submission = Some(signed);
                tx.status = TxStatus::SendMessage(root, proof);
            }
            transactions.insert(tx.uuid, tx);
        }
        let order: Vec<_> = resume_order(&transactions)
            .into_iter()
            .map(|tx| tx.uuid)
            .collect();
        assert_eq!(order, [Uuid::from_u128(3), Uuid::from_u128(2), Uuid::from_u128(1)],
            "a higher saved nonce must not wait for finality while its prerequisite is still queued");
    }

    #[tokio::test]
    async fn original_signed_submission_survives_handoff_restart_and_completion_without_replacement(
    ) {
        let path = std::env::temp_dir().join(format!("gear-outbound-signed-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let mut tx = transaction();
        let uuid = tx.uuid;
        let signed = signed_content(&tx, 36).await;
        let replacement = signed_content(&tx, 37).await;
        let (root, proof) = inclusion(&tx);
        tx.status = TxStatus::FetchMerkleRoot(root);
        tx.lane_policy = Some(policy(&tx.message.message, &signed, false));
        manager.add_transaction(tx).await;
        manager
            .journal_prepare_message(uuid, root, proof)
            .await
            .unwrap()
            .unwrap();
        let view_path = path.join("transaction_status.json");
        let view = tokio::fs::read(&view_path).await.unwrap();
        let active: serde_json::Value = serde_json::from_slice(&view).unwrap();
        assert_eq!(active["active"][uuid.to_string()], "PrepareMessage");
        tokio::fs::write(&view_path, br#"{"version":1,"active":{},"failed":[]}"#)
            .await
            .unwrap();
        assert!(TransactionManager::new(storage.clone())
            .load_from_storage()
            .await
            .unwrap_err()
            .to_string()
            .contains("status view disagrees"));
        tokio::fs::write(&view_path, view).await.unwrap();
        let before_signing = TransactionManager::new(storage.clone());
        before_signing.load_from_storage().await.unwrap();
        assert!(matches!(
            &before_signing.transactions.read().await[&uuid].status,
            TxStatus::PrepareMessage(..)
        ));
        assert!(before_signing.transactions.read().await[&uuid]
            .ethereum_submission
            .is_none());
        assert!(before_signing
            .journal_signed_submission(uuid, signed.clone())
            .await
            .unwrap());
        let committed = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
        let resumed = TransactionManager::new(storage.clone());
        resumed.load_from_storage().await.unwrap();
        {
            let active = resumed.transactions.read().await;
            let restored = &active[&uuid];
            assert!(matches!(&restored.status, TxStatus::SendMessage(..)));
            assert!(restored.ethereum_submission.as_ref() == Some(&signed));
            assert_eq!(restored.ethereum_tx_attempts, vec![signed.hash]);
            assert!(restored.ethereum_tx_hash.is_none());
        }
        assert!(resumed
            .journal_signed_submission(uuid, replacement)
            .await
            .unwrap_err()
            .to_string()
            .contains("identity changed"));
        assert!(tokio::fs::read(path.join(uuid.to_string())).await.unwrap() == committed);
        assert!(resumed
            .journal_signed_submission(uuid, signed.clone())
            .await
            .unwrap());
        let evidence = status_fetcher::tests::evidence(
            &resumed.transactions.read().await[&uuid].message.message,
            &signed,
            false,
        );
        assert!(resumed
            .complete_transaction(uuid, evidence.clone())
            .await
            .unwrap());
        resumed.update_storage().await.unwrap();
        resumed.ack_completed_event(uuid).await.unwrap();
        let terminal: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(view_path).await.unwrap()).unwrap();
        assert_eq!(
            terminal,
            serde_json::json!({"version": 1, "active": {}, "failed": []})
        );
        let completed = TransactionManager::new(storage);
        completed.load_from_storage().await.unwrap();
        let records = completed.completed.read().await;
        assert!(matches!(&records[&uuid].status, TxStatus::Completed));
        assert!(records[&uuid].ethereum_submission.as_ref() == Some(&signed));
        assert_eq!(records[&uuid].ethereum_tx_hash, Some(signed.hash));
        assert_eq!(records[&uuid].ethereum_tx_attempts, vec![signed.hash]);
        drop(records);
        assert!(!completed
            .complete_transaction(uuid, evidence)
            .await
            .unwrap());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn incomplete_signed_handoff_cannot_acknowledge_or_overwrite_unsigned_journal() {
        let path =
            std::env::temp_dir().join(format!("gear-outbound-sign-fsync-{}", Uuid::new_v4()));
        let storage = bound_storage(&path).await;
        let manager = TransactionManager::new(storage.clone());
        let mut tx = transaction();
        let uuid = tx.uuid;
        let signed = signed_content(&tx, 36).await;
        let (root, proof) = inclusion(&tx);
        tx.status = TxStatus::FetchMerkleRoot(root);
        tx.lane_policy = Some(policy(&tx.message.message, &signed, false));
        manager.add_transaction(tx).await;
        manager
            .journal_prepare_message(uuid, root, proof)
            .await
            .unwrap()
            .unwrap();
        let original = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
        tokio::fs::write(
            path.join(".state").join(format!("{uuid}.new")),
            b"{interrupted",
        )
        .await
        .unwrap();
        assert!(manager
            .journal_signed_submission(uuid, signed)
            .await
            .is_err());
        assert!(tokio::fs::read(path.join(uuid.to_string())).await.unwrap() == original);
        let restarted = TransactionManager::new(storage);
        assert!(restarted.load_from_storage().await.is_err());
        assert!(path.join(".state").join(format!("{uuid}.new")).exists());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
}
