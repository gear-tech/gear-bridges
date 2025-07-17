use anyhow::Context;
use ethereum_client::{EthApi, TxHash, TxStatus};
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
    pub proof: FinalProof,
}

pub struct Response {
    pub era: Option<u64>,
    pub merkle_root_block: u32,
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
                proof,
            })
            .is_ok()
    }

    pub fn submit_merkle_root(&self, merkle_root_block: u32, proof: FinalProof) -> bool {
        self.requests
            .send(Request {
                era: None,
                merkle_root_block,
                proof,
            })
            .is_ok()
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.responses.recv().await
    }
}

struct SubmittedMerkleRoot {
    eth_api: EthApi,
    tx_hash: TxHash,
    era: Option<u64>,
    merkle_root_block: u32,
    proof: FinalProof,
    retried: bool,
    status: TxStatus,
}

impl SubmittedMerkleRoot {
    fn new(
        eth_api: EthApi,
        tx_hash: TxHash,
        era: Option<u64>,
        merkle_root_block: u32,
        proof: FinalProof,
        retried: bool,
    ) -> Self {
        Self {
            merkle_root_block,
            eth_api,
            tx_hash,
            era,
            proof,
            retried,
            status: TxStatus::Pending,
        }
    }

    async fn finalize(mut self) -> anyhow::Result<Self> {
        self.status = self.eth_api.get_tx_status(self.tx_hash).await?;
        Ok(self)
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

    metrics: Metrics,
}

impl MeteredService for MerkleRootSubmitter {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl MerkleRootSubmitter {
    pub fn new(eth_api: EthApi, storage: Arc<MerkleRootStorage>) -> Self {
        Self {
            eth_api,
            storage,
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
                    self.storage.submitted_merkle_root(H256::from(request.proof.merkle_root)).await;
                    let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, request.proof.clone()).await?;
                    log::info!("Submitted merkle root to Ethereum, tx hash: {tx_hash}");
                    self.metrics.total_submissions.inc();
                    pending_transactions.push(SubmittedMerkleRoot::new(
                        self.eth_api.clone(),
                        tx_hash,
                        request.era,
                        request.merkle_root_block,
                        request.proof,
                        true
                    ).finalize());


                },

                Some(root) = pending_transactions.next() => {
                    let root = root.context("Failed to check transaction status")?;
                    match root.status {
                        TxStatus::Pending => {
                            log::trace!("Merkle root submission is still pending, tx hash: {}", root.tx_hash);
                            pending_transactions.push(root.finalize());
                        }
                        TxStatus::Finalized => {
                            log::info!("Merkle root submission confirmed, tx hash: {}", root.tx_hash);
                            if responses.send(Response {
                                era: root.era,
                                merkle_root_block: root.merkle_root_block,
                            }).is_err() {
                                return Ok(());
                            }
                        }
                        TxStatus::Failed => {
                            let root_exists = self.eth_api
                                .read_finalized_merkle_root(root.merkle_root_block)
                                .await?
                                .is_some();

                            if root_exists {
                                log::info!("Merkle root at block #{} is already finalized", root.merkle_root_block);
                                if responses.send(Response {
                                    era: root.era,
                                    merkle_root_block: root.merkle_root_block,
                                }).is_err() {
                                    return Ok(());
                                };
                                continue;
                            }

                            log::error!("Merkle root submission failed, tx hash: {}", root.tx_hash);
                            if !root.retried {
                                log::info!("Retrying merkle root submission, tx hash: {}", root.tx_hash);
                                let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, root.proof.clone()).await?;
                                pending_transactions.push(SubmittedMerkleRoot::new(
                                    self.eth_api.clone(),
                                    tx_hash,
                                    root.era,
                                    root.merkle_root_block,
                                    root.proof,
                                    true,
                                ).finalize());
                            } else {
                                self.metrics.failed_submissions.inc();
                                log::error!("Merkle root submission failed again, giving up, tx hash: {}", root.tx_hash);
                            }

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
