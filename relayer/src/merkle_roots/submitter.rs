use alloy::{
    network::Ethereum, providers::PendingTransactionBuilder, rpc::types::TransactionReceipt,
};
use anyhow::Context;
use ethereum_client::EthApi;
use futures::{stream::FuturesUnordered, StreamExt};
use primitive_types::H256;
use prometheus::{
    core::{AtomicU64, GenericCounter, GenericGauge},
    Gauge, IntCounter, IntGauge,
};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{submit_merkle_root_to_ethereum, MAX_RETRIES},
    prover_interface::FinalProof,
    rpc,
};

use super::storage::MerkleRootStorage;
const FINALIZATION_POLL_INTERVAL: Duration = Duration::from_secs(12);
const FINALIZATION_TIMEOUT: Duration = Duration::from_secs(30 * 60);

async fn finalized_submission_exists(
    eth_api: &mut EthApi,
    gear_block: u32,
    expected_root: [u8; 32],
) -> Result<bool, ethereum_client::Error> {
    if expected_root == [0; 32] {
        return rpc::retry_eth_bounded(
            eth_api,
            "read finalized MessageQueue max block number",
            MAX_RETRIES,
            |api| async move { api.finalized_max_block_number().await },
        )
        .await
        .map(|block| block >= gear_block);
    }

    rpc::retry_eth_bounded(
        eth_api,
        "read finalized MessageQueue merkle root",
        MAX_RETRIES,
        |api| async move { api.read_finalized_merkle_root(gear_block).await },
    )
    .await
    .map(|root| root == Some(expected_root))
}

async fn wait_for_persisted_submission(
    eth_api: &mut EthApi,
    gear_block: u32,
    expected_root: [u8; 32],
) -> anyhow::Result<bool> {
    match tokio::time::timeout(FINALIZATION_TIMEOUT, async {
        loop {
            if finalized_submission_exists(eth_api, gear_block, expected_root).await? {
                return Ok::<bool, ethereum_client::Error>(true);
            }
            tokio::time::sleep(FINALIZATION_POLL_INTERVAL).await;
        }
    })
    .await
    {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Ok(false),
    }
}

async fn wait_for_finalized_merkle_root(
    eth_api: &mut EthApi,
    gear_block: u32,
    expected_root: [u8; 32],
) -> anyhow::Result<bool> {
    tokio::time::timeout(FINALIZATION_TIMEOUT, async {
        loop {
            if finalized_submission_exists(eth_api, gear_block, expected_root).await? {
                return Ok::<bool, ethereum_client::Error>(true);
            }

            let exists_at_chainhead = if expected_root == [0; 32] {
                rpc::retry_eth_bounded(
                    eth_api,
                    "read chainhead MessageQueue max block number",
                    MAX_RETRIES,
                    |api| async move { api.max_block_number().await },
                )
                .await?
                    >= gear_block
            } else {
                rpc::retry_eth_bounded(
                    eth_api,
                    "read chainhead MessageQueue merkle root",
                    MAX_RETRIES,
                    |api| async move { api.read_chainhead_merkle_root(gear_block).await },
                )
                .await?
                    == Some(expected_root)
            };
            if !exists_at_chainhead {
                return Ok(false);
            }
            tokio::time::sleep(FINALIZATION_POLL_INTERVAL).await;
        }
    })
    .await
    .context("Timed out waiting for finalized MessageQueue merkle root")?
    .map_err(Into::into)
}

pub struct Request {
    pub era: Option<u64>,
    pub merkle_root_block: u32,
    pub merkle_root: H256,
    pub proof: FinalProof,
}

pub struct Response {
    pub era: Option<u64>,
    pub merkle_root_block: u32,
    pub merkle_root: H256,
    pub proof: FinalProof,
    pub status: ResponseStatus,
}

#[derive(Debug)]
pub enum ResponseStatus {
    Submitted,
    Failed(String),
}

pub struct SubmitterIo {
    requests: UnboundedSender<Request>,
    responses: UnboundedReceiver<Response>,
}

impl SubmitterIo {
    pub fn new(requests: UnboundedSender<Request>, responses: UnboundedReceiver<Response>) -> Self {
        Self {
            requests,
            responses,
        }
    }

    pub fn submit_era_root(&self, era: u64, merkle_root_block: u32, proof: FinalProof) -> bool {
        self.requests
            .send(Request {
                era: Some(era),
                merkle_root_block,
                merkle_root: H256::from(proof.merkle_root),
                proof,
            })
            .is_ok()
    }

    pub fn submit_merkle_root(
        &self,
        merkle_root_block: u32,
        merkle_root: H256,
        proof: FinalProof,
    ) -> bool {
        self.requests
            .send(Request {
                era: None,
                merkle_root_block,
                merkle_root,
                proof,
            })
            .is_ok()
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses.recv().await
    }
}

struct ReconciledSubmission {
    request: Request,
    finalized: bool,
}

async fn reconcile_submission(
    mut eth_api: EthApi,
    request: Request,
) -> anyhow::Result<ReconciledSubmission> {
    let finalized = wait_for_persisted_submission(
        &mut eth_api,
        request.proof.block_number,
        request.proof.merkle_root,
    )
    .await?;
    Ok(ReconciledSubmission { request, finalized })
}

struct SubmittedMerkleRoot {
    era: Option<u64>,
    merkle_root_block: u32,
    merkle_root: H256,
    proof: FinalProof,
    receipt: TransactionReceipt,
    root_is_finalized: bool,
}

struct SubmissionError {
    era: Option<u64>,
    merkle_root_block: u32,
    merkle_root: H256,
    proof: FinalProof,
    error: anyhow::Error,
}

impl SubmittedMerkleRoot {
    async fn new(
        pending_tx: PendingTransactionBuilder<Ethereum>,
        mut eth_api: EthApi,
        era: Option<u64>,
        merkle_root_block: u32,
        merkle_root: H256,
        proof: FinalProof,
        confirmations: u64,
    ) -> Result<Self, SubmissionError> {
        let receipt = pending_tx
            .with_required_confirmations(confirmations)
            .get_receipt()
            .await
            .map_err(|error| SubmissionError {
                era,
                merkle_root_block,
                merkle_root,
                error: error.into(),
                proof: proof.clone(),
            })?;
        let root_is_finalized = if receipt.status() {
            wait_for_finalized_merkle_root(&mut eth_api, proof.block_number, proof.merkle_root)
                .await
                .map_err(|error| SubmissionError {
                    era,
                    merkle_root_block,
                    merkle_root,
                    error,
                    proof: proof.clone(),
                })?
        } else {
            false
        };

        Ok(Self {
            merkle_root_block,
            merkle_root,
            era,
            proof,
            receipt,
            root_is_finalized,
        })
    }
}

impl_metered_service!(
    struct Metrics {
        fee_payer_balance: Gauge = Gauge::new(
            "merkle_root_relayer_fee_payer_balance",
            "Transaction fee payer balance",
        ),

        total_submissions: IntCounter = IntCounter::new(
            "merkle_root_relayer_total_submissions",
            "Total number of merkle root submissions",
        ),

        failed_submissions: IntCounter = IntCounter::new(
            "merkle_root_relayer_failed_submissions",
            "Total number of failed merkle root submissions",
        ),

        pending_submissions: IntGauge = IntGauge::new(
            "merkle_root_relayer_pending_submissions",
            "Total number of pending merkle root submissions",
        ),
        last_submitted_block: IntGauge = IntGauge::new(
            "merkle_root_relayer_last_submitted_block_to_eth",
            "Last submitted merkle root block number to Ethereum",
        ),

        total_gas_used: GenericCounter<AtomicU64> = GenericCounter::new(
            "merkle_root_relayer_total_gas_used",
            "Total gas used for merkle root submissions since relayer start",
        ),

        max_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "merkle_root_relayer_max_gas_used",
            "Maximum gas used for a single merkle root submission",
        ),

        min_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "merkle_root_relayer_min_gas_used",
            "Minimum gas used for a single merkle root submission",
        ),

        last_gas_used: GenericGauge<AtomicU64> = GenericGauge::new(
            "merkle_root_relayer_last_gas_used",
            "Gas used for the last merkle root submission",
        ),
    }
);

pub struct MerkleRootSubmitter {
    eth_api: EthApi,
    storage: Arc<MerkleRootStorage>,
    confirmations: u64,
    relayer_id: String,
    metrics: Metrics,
}

impl MeteredService for MerkleRootSubmitter {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl MerkleRootSubmitter {
    pub fn new(
        eth_api: EthApi,
        storage: Arc<MerkleRootStorage>,
        confirmations: u64,
        relayer_id: String,
    ) -> Self {
        Self {
            eth_api,
            storage,
            confirmations,
            relayer_id,
            metrics: Metrics::new(),
        }
    }

    async fn process(
        &mut self,
        proofs: &mut UnboundedReceiver<Request>,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        let mut pending_transactions = FuturesUnordered::new();
        let mut pending_reconciliations = FuturesUnordered::new();
        let mut retry_requests = VecDeque::new();
        loop {
            let relayer_id = self.relayer_id.clone();
            match self.eth_api.get_approx_balance().await {
                Ok(balance) => self.metrics.fee_payer_balance.set(balance),
                Err(err) => log::warn!(
                    "Merkle root relayer {relayer_id}: failed to update Ethereum fee payer balance metric: {err}"
                ),
            }
            self.metrics
                .pending_submissions
                .set(pending_transactions.len() as i64);

            tokio::select! {
                request = async {
                    match retry_requests.pop_front() {
                        Some(request) => Some(request),
                        None => proofs.recv().await,
                    }
                } => {
                    let Some(request) = request else {
                        log::info!(
                            "Merkle root relayer {relayer_id}: no more proofs to process, exiting"
                        );
                        return Ok(());
                    };

                    if self
                        .storage
                        .is_merkle_root_submitted(
                            request.merkle_root_block,
                            H256::from(request.proof.merkle_root),
                        )
                        .await
                    {
                        pending_reconciliations.push(reconcile_submission(
                            self.eth_api.clone(),
                            request,
                        ));
                        continue;
                    }

                    match rpc::retry_eth_bounded(
                        &mut self.eth_api,
                        "submit merkle root",
                        MAX_RETRIES,
                        |api| {
                            let proof = request.proof.clone();
                            async move { submit_merkle_root_to_ethereum(&api, proof).await }
                        },
                    )
                    .await
                    {
                        Ok(pending_tx) => {
                            log::info!(
                                "Merkle root relayer {relayer_id}: submitted merkle root to Ethereum, tx hash: {}",
                                pending_tx.tx_hash()
                            );
                            self.storage.submitted_merkle_root(request.merkle_root_block, H256::from(request.proof.merkle_root)).await;
                            self.metrics.total_submissions.inc();
                            pending_transactions.push(SubmittedMerkleRoot::new(
                                pending_tx,
                                self.eth_api.clone(),
                                request.era,
                                request.merkle_root_block,
                                request.merkle_root,
                                request.proof,
                                self.confirmations,
                            ));
                            continue;
                        }
                        Err(err)
                            if rpc::classify_ethereum_error(&err)
                                == rpc::RetryDecision::Retry =>
                        {
                            return Err(err.into());
                        }
                        // How do we get here?
                        // - Relayer crashed and already submitted the merkle root but not yet confirmed it
                        // - Somebody else submitted the merkle root
                        Err(ethereum_client::Error::ErrorDuringContractExecution(err)) => {
                            let root_exists = finalized_submission_exists(
                                &mut self.eth_api,
                                request.proof.block_number,
                                request.proof.merkle_root,
                            )
                            .await?;
                            if root_exists {
                                log::warn!("Merkle root relayer {relayer_id}: merkle root {} for block #{} is already submitted, contract execution failed: {err:?}", H256::from(request.proof.merkle_root), request.merkle_root_block);
                                if responses.send(Response {
                                    era: request.era,
                                    merkle_root_block: request.merkle_root_block,
                                    merkle_root: request.merkle_root,
                                    status: ResponseStatus::Submitted,
                                    proof: request.proof.clone(),
                                }).is_err() {
                                    return Ok(());
                                };
                                continue;
                            } else {
                                log::error!("Merkle root relayer {relayer_id}: failed to submit merkle root {}: Error during contract execution: {err:?}", H256::from(request.proof.merkle_root));
                                self.metrics.failed_submissions.inc();
                                self.storage.submission_failed(request.proof.block_number, H256::from(request.proof.merkle_root)).await;
                                if responses.send(Response {
                                    era: request.era,
                                    merkle_root_block: request.merkle_root_block,
                                    merkle_root: request.merkle_root,
                                    status: ResponseStatus::Failed("Error during contract execution".to_string()),
                                    proof: request.proof.clone(),
                                }).is_err() {
                                    return Ok(());
                                };
                                continue;
                            }
                        }
                        Err(err) => {

                            log::error!("Merkle root relayer {relayer_id}: failed to submit merkle root {}: {}", H256::from(request.proof.merkle_root), err);
                            self.metrics.failed_submissions.inc();
                            self.storage.submission_failed(request.proof.block_number, H256::from(request.proof.merkle_root)).await;
                            if responses.send(Response {
                                era: request.era,
                                merkle_root_block: request.merkle_root_block,
                                merkle_root: request.merkle_root,
                                status: ResponseStatus::Failed(err.to_string()),
                                proof: request.proof.clone(),
                            }).is_err() {
                                return Ok(());
                            };
                            continue;
                        }
                    }
                },

                Some(result) = pending_reconciliations.next() => {
                    let reconciled = result?;
                    let request = reconciled.request;
                    if reconciled.finalized {
                        log::info!(
                            "Merkle root relayer {relayer_id}: merkle root {} for block #{} is already finalized",
                            H256::from(request.proof.merkle_root),
                            request.merkle_root_block
                        );
                        if responses
                            .send(Response {
                                era: request.era,
                                merkle_root_block: request.merkle_root_block,
                                merkle_root: request.merkle_root,
                                status: ResponseStatus::Submitted,
                                proof: request.proof,
                            })
                            .is_err()
                        {
                            return Ok(());
                        }
                    } else {
                        log::warn!(
                            "Merkle root relayer {relayer_id}: persisted submission marker for merkle root {} at block #{} is not on the canonical chain; resubmitting",
                            H256::from(request.proof.merkle_root),
                            request.merkle_root_block
                        );
                        self.storage
                            .submission_failed(
                                request.merkle_root_block,
                                H256::from(request.proof.merkle_root),
                            )
                            .await;
                        retry_requests.push_back(request);
                    }
                },

                Some(result) = pending_transactions.next() => {
                    match result {
                        Ok(submitted) => {
                            // update gas used metrics
                            let gas_used = submitted.receipt.gas_used;
                            self.metrics.total_gas_used.inc_by(gas_used);
                            if self.metrics.max_gas_used.get() < gas_used {
                                self.metrics.max_gas_used.set(gas_used);
                            }
                            if self.metrics.min_gas_used.get() == 0 || self.metrics.min_gas_used.get() > gas_used {
                                self.metrics.min_gas_used.set(gas_used);
                            }

                            self.metrics.last_gas_used.set(gas_used);

                            if !submitted.receipt.status() {
                                let root_exists = finalized_submission_exists(
                                    &mut self.eth_api,
                                    submitted.proof.block_number,
                                    submitted.proof.merkle_root,
                                )
                                .await?;

                                if root_exists {
                                    if responses
                                        .send(Response {
                                            era: submitted.era,
                                            merkle_root_block: submitted.merkle_root_block,
                                            merkle_root: submitted.merkle_root,
                                            status: ResponseStatus::Submitted,
                                            proof: submitted.proof.clone(),
                                        })
                                        .is_err()
                                    {
                                        return Ok(());
                                    };
                                    log::info!(
                                        "Merkle root relayer {relayer_id}: merkle root {} for block #{} is already submitted",
                                        submitted.merkle_root,
                                        submitted.merkle_root_block
                                    );
                                    continue;
                                }

                                self.storage
                                    .submission_failed(
                                        submitted.merkle_root_block,
                                        H256::from(submitted.proof.merkle_root),
                                    )
                                    .await;
                                if responses
                                    .send(Response {
                                        era: submitted.era,
                                        merkle_root_block: submitted.merkle_root_block,
                                        merkle_root: submitted.merkle_root,
                                        status: ResponseStatus::Failed(format!(
                                            "Transaction {} failed",
                                            submitted.receipt.transaction_hash
                                        )),
                                        proof: submitted.proof.clone(),
                                    })
                                    .is_err()
                                {
                                    return Ok(());
                                };
                                continue;
                            }

                            if !submitted.root_is_finalized {
                                self.storage
                                    .submission_failed(
                                        submitted.merkle_root_block,
                                        H256::from(submitted.proof.merkle_root),
                                    )
                                    .await;
                                if responses
                                    .send(Response {
                                        era: submitted.era,
                                        merkle_root_block: submitted.merkle_root_block,
                                        merkle_root: submitted.merkle_root,
                                        status: ResponseStatus::Failed(
                                            "Confirmed transaction did not store the expected merkle root"
                                                .to_string(),
                                        ),
                                        proof: submitted.proof.clone(),
                                    })
                                    .is_err()
                                {
                                    return Ok(());
                                }
                                continue;
                            }

                            if responses.send(Response {
                                era: submitted.era,
                                merkle_root_block: submitted.merkle_root_block,
                                merkle_root: submitted.merkle_root,
                                status: ResponseStatus::Submitted,
                                proof: submitted.proof.clone(),
                            }).is_err() {
                                return Ok(());
                            };
                            self.metrics.last_submitted_block.set(submitted.merkle_root_block as i64);
                            log::info!(
                                "Merkle root relayer {relayer_id}: merkle root {} for block #{} submission confirmed after {} confirmations",
                                submitted.merkle_root,
                                submitted.merkle_root_block,
                                self.confirmations
                            );
                            self.metrics.pending_submissions.dec();
                        }

                        Err(err) => {
                            let root_exists = finalized_submission_exists(
                                &mut self.eth_api,
                                err.proof.block_number,
                                err.proof.merkle_root,
                            )
                            .await?;

                            if root_exists {
                                if responses.send(Response {
                                    era: err.era,
                                    merkle_root_block: err.merkle_root_block,
                                    merkle_root: err.merkle_root,
                                    status: ResponseStatus::Submitted,
                                    proof: err.proof,
                                }).is_err() {
                                    return Ok(());
                                };
                                log::info!("Merkle root relayer {relayer_id}: merkle root {} for block #{} is already submitted", err.merkle_root, err.merkle_root_block);
                                continue;
                            }

                            log::error!("Merkle root relayer {relayer_id}: failed to submit merkle root {}: {}", err.merkle_root, err.error);
                            self.metrics.pending_submissions.dec();
                            self.metrics.failed_submissions.inc();
                            self.storage.submission_failed(err.merkle_root_block, H256::from(err.proof.merkle_root)).await;
                            if responses.send(Response {
                                era: err.era,
                                merkle_root_block: err.merkle_root_block,
                                merkle_root: err.merkle_root,
                                status: ResponseStatus::Failed(err.error.to_string()),
                                proof: err.proof,
                            }).is_err() {
                                return Ok(());
                            };
                        }
                    }
                }
            }
        }
    }

    pub fn run(self) -> SubmitterIo {
        let (tx, rx) = unbounded_channel();
        let (response_tx, response_rx) = unbounded_channel();

        tokio::task::spawn(task(self, rx, response_tx));

        SubmitterIo::new(tx, response_rx)
    }
}

async fn task(
    mut this: MerkleRootSubmitter,
    mut proofs: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
) {
    if let Err(err) = this.process(&mut proofs, &responses).await {
        // Restarting only this task would lose requests and receipt results that
        // `process` already removed from its in-memory queues. Closing the response
        // channel makes the parent relayer restart from its durable root state.
        log::error!(
            "Merkle root relayer {} submitter failed, exiting: {err}",
            this.relayer_id
        );
    }
}
