use alloy::{
    network::Ethereum,
    providers::{PendingTransactionBuilder, PendingTransactionError},
    rpc::types::TransactionReceipt,
};
use ethereum_client::EthApi;
use futures::{stream::FuturesUnordered, StreamExt};
use primitive_types::H256;
use prometheus::{
    core::{AtomicU64, GenericCounter, GenericGauge},
    Gauge, IntCounter, IntGauge,
};
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{submit_merkle_root_to_ethereum, MAX_RETRIES},
    prover_interface::FinalProof,
    rpc,
};

use super::storage::MerkleRootStorage;

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

struct SubmittedMerkleRoot {
    era: Option<u64>,
    merkle_root_block: u32,
    merkle_root: H256,
    proof: FinalProof,
    receipt: TransactionReceipt,
}

struct SubmissionError {
    era: Option<u64>,
    merkle_root_block: u32,
    merkle_root: H256,
    proof: FinalProof,
    error: PendingTransactionError,
}

impl SubmittedMerkleRoot {
    async fn new(
        pending_tx: PendingTransactionBuilder<Ethereum>,
        era: Option<u64>,
        merkle_root_block: u32,
        merkle_root: H256,
        proof: FinalProof,
        confirmations: u64,
    ) -> Result<Self, SubmissionError> {
        Ok(Self {
            merkle_root_block,
            merkle_root,
            era,
            proof: proof.clone(),
            receipt: pending_tx
                .with_required_confirmations(confirmations)
                .get_receipt()
                .await
                .map_err(|error| SubmissionError {
                    era,
                    merkle_root_block,
                    merkle_root,
                    error,
                    proof,
                })?,
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

    async fn read_finalized_merkle_root(
        &mut self,
        gear_block: u32,
    ) -> Result<Option<[u8; 32]>, ethereum_client::Error> {
        rpc::retry_eth_bounded(
            &mut self.eth_api,
            "read finalized MessageQueue merkle root",
            MAX_RETRIES,
            |api| async move { api.read_finalized_merkle_root(gear_block).await },
        )
        .await
    }

    async fn process(
        &mut self,
        proofs: &mut UnboundedReceiver<Request>,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        let mut pending_transactions = FuturesUnordered::new();
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
                request = proofs.recv() => {
                    let Some(request) = request else {
                        log::info!(
                            "Merkle root relayer {relayer_id}: no more proofs to process, exiting"
                        );
                        return Ok(());
                    };

                    if self.storage.is_merkle_root_submitted(request.merkle_root_block, H256::from(request.proof.merkle_root)).await {
                        log::info!(
                            "Merkle root relayer {relayer_id}: merkle root {} for block #{} is already submitted", H256::from(request.proof.merkle_root), request.merkle_root_block);
                        if responses.send(Response {
                            era: request.era,
                            merkle_root_block: request.merkle_root_block,
                            merkle_root: request.merkle_root,
                            status: ResponseStatus::Submitted,
                            proof: request.proof,
                        }).is_err() {
                            return Ok(());
                        };
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
                            let root_exists = self.read_finalized_merkle_root(request.proof.block_number)
                                .await?
                                .is_some();

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
                                let root_exists = self.read_finalized_merkle_root(submitted.proof.block_number)
                                    .await?
                                    .is_some();

                                if root_exists {
                                    if responses.send(Response {
                                        era: submitted.era,
                                        merkle_root_block: submitted.merkle_root_block,
                                        merkle_root: submitted.merkle_root,
                                        status: ResponseStatus::Submitted,
                                        proof: submitted.proof.clone(),
                                    }).is_err() {
                                        return Ok(());
                                    };
                                    log::info!("Merkle root relayer {relayer_id}: merkle root {} for block #{} is already submitted", submitted.merkle_root, submitted.merkle_root_block);
                                    continue;
                                }

                                if responses.send(Response {
                                    era: submitted.era,
                                    merkle_root_block: submitted.merkle_root_block,
                                    merkle_root: submitted.merkle_root,
                                    status: ResponseStatus::Failed(format!("Transaction {} failed", submitted.receipt.transaction_hash)),
                                    proof: submitted.proof.clone(),
                                }).is_err() {
                                    return Ok(());
                                };
                                // SECURITY(M-1): a reverted submission must not also fall through to the
                                // Submitted arm below — dual responses desynchronize the listener into a
                                // phantom Finalized state (validated cluster2/W1, rust-poc2 sim).
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
                            let root_exists = self.read_finalized_merkle_root(err.proof.block_number)
                                .await?
                                .is_some();

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
