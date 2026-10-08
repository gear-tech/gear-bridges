use ethereum_client::{prepared_content_message_identity, PreparedContentMessage, TxHash};
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
    pub uuid: Uuid,
    pub message: MessageInBlock,
    pub message_hash: [u8; 32],
    pub status: TxStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethereum_submission: Option<PreparedContentMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_evidence: Option<serde_json::Value>,
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
            journal_version: 3,
            completion: None,
            uuid,
            status,
            message_hash: message_hash(&message.message),
            message,
            ethereum_submission: None,
            legacy_evidence: None,
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
            matches!(self.journal_version, 2 | 3),
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
            anyhow::ensure!(
                self.journal_version == 3 && self.ethereum_tx_hash == Some(signed.hash),
                "HOLD: completion version/hash changed"
            );
        }
        anyhow::ensure!(
            self.message_hash == message_hash(&self.message.message),
            "HOLD: outbound message identity changed for {}",
            self.uuid
        );
        if matches!(
            &self.status,
            TxStatus::WaitForMerkleRoot
                | TxStatus::FetchMerkleRoot(_)
                | TxStatus::PrepareMessage(..)
        ) {
            anyhow::ensure!(
                self.ethereum_tx_attempts.is_empty() && self.ethereum_tx_hash.is_none(),
                "HOLD: outbound unsigned state discarded a prior transaction identity"
            );
        }
        if let Some(signed) = &self.ethereum_submission {
            prepared_content_message_identity(signed)?;
            anyhow::ensure!(
                !matches!(
                    &self.status,
                    TxStatus::WaitForMerkleRoot
                        | TxStatus::FetchMerkleRoot(_)
                        | TxStatus::PrepareMessage(..)
                ),
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
    Active(Box<Transaction>),
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

            metrics: Metrics::new(),
        }
    }

    pub async fn fail_transaction(&self, tx_uuid: Uuid, reason: String) {
        self.failed.write().await.insert(tx_uuid, reason);
        self.metrics.failed_transactions.inc();
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
        ) {
            return Ok(false);
        }
        let signed = current
            .ethereum_submission
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("HOLD: completion has no original signed bytes"))?;
        status_fetcher::validate_completion(&current.message.message, signed, &evidence)?;
        let ethereum_tx_hash = Some(signed.hash);
        let mut tx = active
            .remove(&uuid)
            .expect("active transaction was checked");
        drop(active);

        let attempted_hash = match &tx.status {
            TxStatus::WaitConfirmations(hash) => Some(*hash),
            _ => None,
        };
        if let Some(hash) = attempted_hash {
            tx.remember_attempt(hash);
        }
        tx.status = TxStatus::Completed;
        tx.journal_version = 3;
        tx.completion = Some(evidence);
        tx.ethereum_tx_hash = ethereum_tx_hash;
        self.completed.write().await.insert(uuid, tx);
        self.failed.write().await.remove(&uuid);
        self.metrics.completed_transactions.inc();
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
                tx.message == *message,
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
                tx.message == *message,
                "HOLD: source message conflicts with active nonce {}",
                hex::encode(nonce)
            );
            return Ok(Some(ExistingTransaction::Active(Box::new(tx.clone()))));
        }
        Ok(None)
    }

    async fn accept_message(&self, message: MessageInBlock) -> anyhow::Result<Option<Transaction>> {
        let nonce = message.message.nonce_be;
        match self.existing_transaction(&message).await? {
            Some(ExistingTransaction::Completed) => {
                self.storage.save(self).await?;
                self.storage.ack_event_pair(nonce).await?;
                return Ok(None);
            }
            Some(ExistingTransaction::Active(tx)) => {
                log::info!(
                    "Skipping active outbound nonce {} in status {:?}",
                    hex::encode(nonce),
                    tx.status
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
        self.storage
            .block_storage()
            .complete_transaction(&message)
            .await;
        let tx = Transaction::new(message, TxStatus::WaitForMerkleRoot);
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
        for tx in self.completed.read().await.values() {
            tx.validate_journal()?;
            let signed = tx
                .ethereum_submission
                .as_ref()
                .expect("validated signed completion");
            let receipt = api
                .get_finalized_receipt(signed.hash)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "HOLD: saved completion has no original canonical finalized receipt"
                    )
                })?;
            let observed =
                status_fetcher::completion_evidence(api, &tx.message.message, signed, receipt)
                    .await?;
            let saved = tx
                .completion
                .as_ref()
                .expect("validated completion evidence");
            anyhow::ensure!(
                observed.receipt.block_hash == saved.receipt.block_hash
                    && observed.token_delivery == saved.token_delivery,
                "HOLD: saved completion inclusion/effect policy changed"
            );
            self.storage
                .ack_event_pair(tx.message.message.nonce_be)
                .await?;
        }
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
                TxStatus::NeedsReconciliation { diagnostic } | TxStatus::Failed { diagnostic } => {
                    anyhow::bail!("HOLD: outbound transaction {} nonce {} retains attempts {:?}: {diagnostic}", tx.uuid, hex::encode(tx.message.message.nonce_be), tx.ethereum_tx_attempts);
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
    ) -> anyhow::Result<()> {
        {
            let mut transactions = self.transactions.write().await;
            let Some(tx) = transactions.get_mut(&tx_uuid) else {
                log::warn!("Received HOLD for unknown transaction: {tx_uuid}");
                return Ok(());
            };
            if let TxStatus::WaitConfirmations(hash) = &tx.status {
                let hash = *hash;
                tx.remember_attempt(hash);
            }
            tx.status = TxStatus::NeedsReconciliation {
                diagnostic: diagnostic.clone(),
            };
        }
        self.storage.save(self).await?;
        anyhow::bail!("HOLD: outbound transaction {tx_uuid}: {diagnostic}")
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

        // Saved signatures reserve their original nonces before any fresh preparation.
        for tx in transactions
            .values()
            .filter(|tx| tx.ethereum_submission.is_some())
            .chain(
                transactions
                    .values()
                    .filter(|tx| tx.ethereum_submission.is_none()),
            )
        {
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
                    ) {
                        log::warn!("Status fetcher stopped accepting requests, exiting");
                        return Ok(false);
                    }
                }

                TxStatus::NeedsReconciliation { ref diagnostic }
                | TxStatus::Failed { ref diagnostic } => {
                    anyhow::bail!("HOLD: outbound transaction {}: {diagnostic}", tx.uuid)
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

                if !accumulator.send_message(uuid, authority_set_id, block, block_hash, source) {
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
                                tx.message.authority_set_id == authority_set_id && tx.message.block == block,
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

                    accumulator::Response::Stuck { tx_uuid, .. } => {
                        if self
                            .fail_active_transaction(tx_uuid, "Message stuck".to_string())
                            .await
                            .is_none()
                        {
                            log::warn!("Received stuck response for unknown transaction: {tx_uuid}");
                        }
                    }
                }
            }

            message = proof_fetcher.recv_message() => {
                let Some(message) = message else {
                    log::info!("No more messages from proof fetcher, exiting");
                    return Ok(false);
                };

                if let Some(original_message) = self
                    .journal_prepare_message(message.tx_uuid, message.merkle_root, message.proof.clone())
                    .await?
                {
                    if !message_sender.send(
                        original_message,
                        message.merkle_root,
                        message.proof,
                        message.tx_uuid,
                        None,
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
                                Some((tx.message.message.clone(), tx.ethereum_submission.clone()))
                            })
                        };
                        if let Some((message, submission)) = original {
                            self.storage.save(self).await?;
                            if !status_fetcher.send_request(tx_uuid, tx_hash, message, submission) {
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
                        self.hold_active_transaction(tx_uuid, reason).await?;
                    }
                }
            }

            message = status_fetcher.recv_message() => {
                let Some(status) = message else {
                    log::info!("No more messages from status fetcher, exiting");
                    return Ok(false);
                };

                match status {
                    (uuid, Ok(evidence)) => {
                        if self.complete_transaction(uuid, evidence).await? {
                            self.storage.save(self).await?;
                            self.ack_completed_event(uuid).await?;
                            log::info!(
                                "Transaction {uuid} completed with original finalized receipt and matching effects"
                            );
                        } else {
                            log::warn!("Received success response for unknown transaction: {uuid}");
                        }
                    }

                    (uuid, Err(error)) => {
                        // Without the original raw bytes/account nonce, a replacement
                        // after any uncertain outcome is not safe to create automatically.
                        self.hold_active_transaction(uuid, error).await?;
                    }
                }
            }
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
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

    fn inclusion(tx: &Transaction) -> (RelayedMerkleRoot, MerkleProof) {
        let root = RelayedMerkleRoot {
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
            chain_id: 560048,
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
            chain_id: 560048,
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
                destination_chain_id: 560048,
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
    async fn ineligible_operator_handoff_keeps_journal_clean_then_valid_pair_admits_once() {
        for queued_before_request in [false, true] {
            let path =
                std::env::temp_dir().join(format!("gear-operator-admission-{}", Uuid::new_v4()));
            let storage = Arc::new(JSONStorage::new(&path));
            storage
                .bind_outbound_lane(OutboundLaneIdentity {
                    destination_chain_id: 560048,
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
            let manager = TransactionManager::new(storage.clone());
            manager.update_storage().await.unwrap();
            let mut before = BTreeMap::new();
            for name in [
                "gear_events.json",
                "failed",
                "merkle_roots",
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
                    destination_chain_id: 560048,
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
            let manager = TransactionManager::new(storage.clone());
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
        let error = restored.load_from_storage().await.unwrap_err().to_string();
        assert!(error.contains("HOLD") && error.contains(&uuid.to_string()));
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
        let error = manager
            .hold_active_transaction(uuid, "no canonical receipt yet".into())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("HOLD") && error.contains(&uuid.to_string()));

        let restored = TransactionManager::new(storage.clone());
        assert!(restored
            .load_from_storage()
            .await
            .unwrap_err()
            .to_string()
            .contains("HOLD"));
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
