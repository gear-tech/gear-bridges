use alloy::{
    providers::{PendingTransactionBuilder, PendingTransactionError, Provider},
    rpc::types::TransactionReceipt,
};
use ethereum_client::{EthApi, TxHash};
use futures::{stream::FuturesUnordered, StreamExt};
use primitive_types::H256;
use prometheus::{Gauge, IntCounter, IntGauge};
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{submit_merkle_root_to_ethereum, BASE_RETRY_DELAY, MAX_RETRIES},
    prover_interface::FinalProof,
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
        eth_api: &EthApi,
        tx_hash: TxHash,
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
            receipt: PendingTransactionBuilder::new(eth_api.raw_provider().root().clone(), tx_hash)
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
    }
);

pub struct MerkleRootSubmitter {
    eth_api: EthApi,
    storage: Arc<MerkleRootStorage>,
    confirmations: u64,
    metrics: Metrics,
}

impl MeteredService for MerkleRootSubmitter {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl MerkleRootSubmitter {
    pub fn new(eth_api: EthApi, storage: Arc<MerkleRootStorage>, confirmations: u64) -> Self {
        Self {
            eth_api,
            storage,
            confirmations,
            metrics: Metrics::new(),
        }
    }

    async fn process(
        &mut self,
        proofs: &mut UnboundedReceiver<Request>,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<()> {
        let mut pending_transactions = FuturesUnordered::new();
        loop {
            let balance = self.eth_api.get_approx_balance().await?;
            self.metrics.fee_payer_balance.set(balance);
            self.metrics
                .pending_submissions
                .set(pending_transactions.len() as i64);

            tokio::select! {
                request = proofs.recv() => {
                    let Some(request) = request else {
                        log::info!("No more proofs to process, exiting");
                        return Ok(());
                    };

                    if self.storage.is_merkle_root_submitted(request.merkle_root_block, H256::from(request.proof.merkle_root)).await {
                        log::info!(
                            "Merkle root {} for block #{} is already submitted", H256::from(request.proof.merkle_root), request.merkle_root_block);
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

                    self.storage.submitted_merkle_root(request.merkle_root_block, H256::from(request.proof.merkle_root)).await;

                    match submit_merkle_root_to_ethereum(&self.eth_api, request.proof.clone()).await {
                        Ok(tx_hash) => {
                            log::info!("Submitted merkle root to Ethereum, tx hash: {tx_hash}");
                            self.metrics.total_submissions.inc();
                            pending_transactions.push(SubmittedMerkleRoot::new(
                                &self.eth_api,
                                tx_hash,
                                request.era,
                                request.merkle_root_block,
                                request.merkle_root,
                                request.proof,
                                self.confirmations,
                            ));
                        }
                        // How do we get here?
                        // - Relayer crashed and already submitted the merkle root but not yet confirmed it
                        // - Somebody else submitted the merkle root
                        Err(ethereum_client::Error::ErrorDuringContractExecution(err)) => {
                            let root_exists = self.eth_api
                                .read_finalized_merkle_root(request.proof.block_number)
                                .await?
                                .is_some();

                            if root_exists {
                                log::warn!("Merkle root {} for block #{} is already submitted, contract execution failed: {err:?}", H256::from(request.proof.merkle_root), request.merkle_root_block);
                                if responses.send(Response {
                                    era: request.era,
                                    merkle_root_block: request.merkle_root_block,
                                    merkle_root: H256::from(request.proof.merkle_root),
                                    status: ResponseStatus::Submitted,
                                    proof: request.proof,
                                }).is_err() {
                                    return Ok(());
                                };
                            } else {
                                log::error!("Failed to submit merkle root {}: Error during contract execution: {err:?}", H256::from(request.proof.merkle_root));
                                self.metrics.failed_submissions.inc();
                                self.storage.submission_failed(request.proof.block_number, H256::from(request.proof.merkle_root)).await;
                                if responses.send(Response {
                                    era: request.era,
                                    merkle_root_block: request.merkle_root_block,
                                    merkle_root: H256::from(request.proof.merkle_root),
                                    status: ResponseStatus::Failed("Error during contract execution".to_string()),
                                    proof: request.proof,
                                }).is_err() {
                                    return Ok(());
                                };
                            }

                        }

                        Err(err) => {
                            log::error!("Failed to submit merkle root {}: {}", H256::from(request.proof.merkle_root), err);
                            self.metrics.failed_submissions.inc();
                            self.storage.submission_failed(request.merkle_root_block, H256::from(request.proof.merkle_root)).await;
                            if responses.send(Response {
                                era: request.era,
                                merkle_root_block: request.merkle_root_block,
                                merkle_root: H256::from(request.proof.merkle_root),
                                status: ResponseStatus::Failed(err.to_string()),
                                proof: request.proof,
                            }).is_err() {
                                return Ok(());
                            };
                        }
                    }
                },

                Some(result) = pending_transactions.next() => {
                    match result {
                        Ok(submitted) => {
                            if !submitted.receipt.status() {
                                let root_exists = self.eth_api
                                    .read_finalized_merkle_root(submitted.proof.block_number)
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
                                    log::info!("Merkle root {} for block #{} is already submitted", submitted.merkle_root, submitted.merkle_root_block);
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

                            log::info!(
                                "Merkle root {} for block #{} submission confirmed after {} confirmations",
                                submitted.merkle_root,
                                submitted.merkle_root_block,
                                self.confirmations
                            );
                            self.metrics.pending_submissions.dec();
                        }

                        Err(err) => {
                            let root_exists = self.eth_api
                                .read_finalized_merkle_root(err.proof.block_number)
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
                                log::info!("Merkle root {} for block #{} is already submitted", err.merkle_root, err.merkle_root_block);
                                continue;
                            }

                            log::error!("Failed to submit merkle root {}: {}", err.merkle_root, err.error);
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
    let mut attempts = 0;

    loop {
        match this.process(&mut proofs, &responses).await {
            Ok(_) => break,
            Err(e) => {
                attempts += 1;
                let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                log::error!(
                    "Merkle root submitter failed (attempt: {attempts}/{MAX_RETRIES}): {e}. Retrying in {delay:?}",
                );
                if attempts >= MAX_RETRIES {
                    log::error!("Maximum attempts reached, exiting...");
                    break;
                }
                tokio::time::sleep(delay).await;

                match this.eth_api.reconnect().await {
                    Ok(eth_api) => this.eth_api = eth_api,
                    Err(e) => {
                        log::error!("Failed to reconnect to Ethereum API: {e}");
                        break;
                    }
                }
            }
        }
    }
}
