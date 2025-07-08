use anyhow::Context;
use ethereum_client::{EthApi, TxHash, TxStatus};
use futures::{stream::FuturesUnordered, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::{
    common::{submit_merkle_root_to_ethereum, BASE_RETRY_DELAY, MAX_RETRIES},
    prover_interface::FinalProof,
};

pub struct MerkleRootSubmitter {
    eth_api: EthApi,
}

impl MerkleRootSubmitter {
    pub fn new(eth_api: EthApi) -> Self {
        Self { eth_api }
    }

    async fn process(&mut self, proofs: &mut UnboundedReceiver<FinalProof>) -> anyhow::Result<()> {
        let mut pending_transactions = FuturesUnordered::new();
        loop {
            tokio::select! {
                proof = proofs.recv() => {
                    match proof {
                        Some(proof) => {
                            let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, proof.clone()).await?;
                            log::info!("Submitted merkle root to Ethereum, tx hash: {tx_hash}");

                            pending_transactions.push(SubmittedMerkleRoot::new(
                                self.eth_api.clone(),
                                tx_hash,
                                proof,
                                true
                            ).finalize());
                        }

                        None => {
                            log::info!("No more proofs to process, exiting.");
                            return Ok(());
                        }
                    }
                },

                Some(root) = pending_transactions.next() => {
                    let root = root.context("Failed to check transaction status")?;
                    match root.status {
                        TxStatus::Pending => {
                            log::info!("Merkle root submission is still pending, tx hash: {}", root.tx_hash);
                            pending_transactions.push(root.finalize());
                        }
                        TxStatus::Finalized => {
                            log::info!("Merkle root submission confirmed, tx hash: {}", root.tx_hash);
                        }
                        TxStatus::Failed => {
                            log::error!("Merkle root submission failed, tx hash: {}", root.tx_hash);
                            if !root.retried {
                                log::info!("Retrying merkle root submission, tx hash: {}", root.tx_hash);
                                let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, root.proof.clone()).await?;
                                pending_transactions.push(SubmittedMerkleRoot::new(
                                    self.eth_api.clone(),
                                    tx_hash,
                                    root.proof,
                                    true,
                                ).finalize());
                            } else {
                                log::error!("Merkle root submission failed again, giving up, tx hash: {}", root.tx_hash);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn run(self) -> UnboundedSender<FinalProof> {
        let (tx, rx) = unbounded_channel();
        tokio::task::spawn(task(self, rx));
        tx
    }
}

struct SubmittedMerkleRoot {
    eth_api: EthApi,
    tx_hash: TxHash,
    proof: FinalProof,
    retried: bool,
    status: TxStatus,
}

impl SubmittedMerkleRoot {
    fn new(eth_api: EthApi, tx_hash: TxHash, proof: FinalProof, retried: bool) -> Self {
        Self {
            eth_api,
            tx_hash,
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

async fn task(mut this: MerkleRootSubmitter, mut proofs: UnboundedReceiver<FinalProof>) {
    let mut attempts = 0;

    loop {
        match this.process(&mut proofs).await {
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
