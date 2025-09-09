use std::{process, time::Duration};

use reqwest::Client as HttpClient;
use thiserror::Error;

use ethereum_client::{EthApi, MerkleRootEntry, TxHash, TxStatus};
use prometheus::{Gauge, IntCounter, IntGauge};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::{
    common::{
        is_rpc_transport_error_recoverable, send_challege_root_to_ethereum,
        submit_merkle_root_to_ethereum,
    },
    message_relayer::{
        common::web_request::{MerkleRootBlocks, MerkleRootsResponse},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
    prover_interface::FinalProof,
};

const REPEAT_PERIOD_SEC: Duration = Duration::from_secs(12);
const ERROR_REPEAT_DELAY: Duration = Duration::from_secs(3);

impl_metered_service! {
    struct Metrics {
        latest_proven_era: IntGauge = IntGauge::new(
            "kill_switch_latest_proven_era",
            "Latest proven era number",
        ),
        latest_observed_gear_era: IntGauge = IntGauge::new(
            "kill_switch_latest_observed_gear_era",
            "Latest era number observed by relayer",
        ),
        fee_payer_balance: Gauge = Gauge::new(
            "kill_switch_fee_payer_balance",
            "Transaction fee payer balance",
        ),
        latest_eth_block: IntGauge = IntGauge::new(
            "kill_switch_latest_eth_block",
            "Latest block number observed",
        ),
        merkle_roots_discovered_cnt: IntCounter = IntCounter::new(
            "kill_switch_merkle_roots_discovered_cnt",
            "Amount of merkle root events discovered",
        ),
        merkle_root_mismatch_cnt: IntCounter = IntCounter::new(
            "kill_switch_merkle_root_mismatch_cnt",
            "Amount of merkle root mismatches found",
        ),
        finality_proof_for_mismatched_root_not_found_cnt: IntCounter = IntCounter::new(
            "kill_switch_finality_proof_for_mismatched_root_not_found_cnt",
            "Amount of not found finality proofs",
        ),
    }
}

#[derive(Debug, Error)]
enum MainLoopError {
    #[error("Ethereum API error: {0}")]
    EthApi(#[from] ethereum_client::Error),
    #[error("Gear API error: {0}")]
    GearApi(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
enum ChallengeRootError {
    #[error("Ethereum API error: {0}")]
    EthApi(#[from] ethereum_client::Error),
}

#[derive(Debug, Error)]
enum SubmitMerkleRootError {
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("No merkle root on block {block_number}")]
    NoMerkleRootOnBlock { block_number: u32 },
    #[error("Other error: {0}")]
    Other(String),
    #[error("Ethereum API error: {0}")]
    EthApi(#[from] ethereum_client::Error),
}

#[derive(Debug, Error)]
enum Error {
    #[error("Main loop error: {0}")]
    MainLoop(#[from] MainLoopError),
    #[error("Challenge root error: {0}")]
    ChallengeRoot(#[from] ChallengeRootError),
    #[error("Submit merkle root error: {0}")]
    SubmitMerkleRoot(#[from] SubmitMerkleRootError),
}

enum State {
    // Normal operation
    Normal,
    // Waiting for challenge root tx to be finalized
    ChallengeRoot { tx_hash: Option<TxHash> },
    // Waiting for submit merkle root tx to be finalized
    SubmitMerkleRoot { tx_hash: Option<TxHash> },
    // Exit
    Exit,
}

pub struct KillSwitchRelayer {
    api_provider: ApiProviderConnection,
    eth_api: EthApi,
    http_client: HttpClient,
    relayer_http_url: String,

    start_from_eth_block: Option<u64>,
    state: State,
    challenged_block: Option<u64>,

    metrics: Metrics,
}

impl MeteredService for KillSwitchRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl KillSwitchRelayer {
    pub async fn new(
        api_provider: ApiProviderConnection,
        eth_api: EthApi,
        http_client: HttpClient,
        from_eth_block: Option<u64>,
        relayer_http_url: String,
    ) -> Self {
        Self {
            api_provider,
            eth_api,
            relayer_http_url,
            http_client,
            start_from_eth_block: from_eth_block,
            state: State::Normal,
            challenged_block: None,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        log::info!("Starting kill switch relayer");

        loop {
            let loop_delay;

            let res = match &self.state {
                State::Normal => self.main_loop().await.map_err(Error::from),
                State::ChallengeRoot { .. } => self.challenge_root().await.map_err(Error::from),
                State::SubmitMerkleRoot { .. } => {
                    self.submit_merkle_root().await.map_err(Error::from)
                }
                State::Exit => {
                    log::info!("Exiting ...");
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    process::exit(1);
                }
            };

            match res {
                Ok(()) => {
                    if let State::Normal = self.state {
                        // Repeat merkle root event scan after a delay
                        loop_delay = REPEAT_PERIOD_SEC;
                    } else {
                        // State changed so we want to immediately process the new state.
                        loop_delay = Duration::ZERO;
                    }
                }
                Err(err) => {
                    loop_delay = ERROR_REPEAT_DELAY;

                    match err {
                        Error::MainLoop(MainLoopError::EthApi(
                            ethereum_client::Error::ErrorInHTTPTransport(err),
                        ))
                        | Error::ChallengeRoot(ChallengeRootError::EthApi(
                            ethereum_client::Error::ErrorInHTTPTransport(err),
                        ))
                        | Error::SubmitMerkleRoot(SubmitMerkleRootError::EthApi(
                            ethereum_client::Error::ErrorInHTTPTransport(err),
                        )) => {
                            if is_rpc_transport_error_recoverable(&err) {
                                // Reconnect on Ethereum API
                                log::error!(
                                    "Recoverable Ethereum transport error: {err}, reconnecting..."
                                );
                                if let Err(e) = self.eth_api.reconnect().await {
                                    log::error!("Ethereum API reconnect failed: {e}");
                                }
                            }
                        }
                        Error::MainLoop(MainLoopError::GearApi(e)) => {
                            // Reconnect on Gear API
                            log::error!("Gear API error: {e}, reconnecting...");
                            if let Err(e) = self.api_provider.reconnect().await {
                                log::error!("Gear API reconnect failed: {e}");
                            }
                        }
                        e => {
                            log::error!("Error in kill switch relayer: {e}");
                        }
                    }
                }
            }

            if loop_delay != Duration::ZERO {
                tokio::time::sleep(loop_delay).await;
            }
        }
    }

    async fn main_loop(&mut self) -> Result<(), MainLoopError> {
        let balance = self.eth_api.get_approx_balance().await?;
        self.metrics.fee_payer_balance.set(balance);

        let last_finalized_block = self
            .eth_api
            .finalized_block_number()
            .await
            .map_err(MainLoopError::GearApi)?;

        // Set the initial value for `from_eth_block` if it's not set yet.
        let start_from_eth_block = self.start_from_eth_block.unwrap_or(last_finalized_block);

        if last_finalized_block < start_from_eth_block {
            log::info!(
                "No new eth block, skipping.. last_processed_eth_block={}, last_finalized_block={}",
                start_from_eth_block.saturating_sub(1),
                last_finalized_block,
            );
            return Ok(());
        } else {
            self.metrics
                .latest_eth_block
                .set(last_finalized_block as i64);
        }

        let events = self
            .eth_api
            .fetch_merkle_roots_in_range(start_from_eth_block, last_finalized_block)
            .await?;

        if !events.is_empty() {
            self.metrics
                .merkle_roots_discovered_cnt
                .inc_by(events.len() as u64);
        }

        for (event, _block_number_eth) in events {
            if !self.compare_merkle_roots(&event).await? {
                // Okay, we have a mismatch,
                // that means for some reason the proof with incorrect merkle root was submitted to relayer contract.
                // We need to challenge it by submitting the correct merkle root.
                log::debug!("Got event with mismatched merkle root: {:?}", &event);

                // Switch to challenge root state
                self.state = State::ChallengeRoot { tx_hash: None };
                self.challenged_block = Some(event.block_number);

                return Ok(());
            }
        }

        // After processing all events, `last_finalized_block` is the last block we've processed.
        // So, we need to increment it by 1 to set the next block to process.
        self.start_from_eth_block = Some(last_finalized_block.saturating_add(1));

        Ok(())
    }

    async fn challenge_root(&mut self) -> Result<(), ChallengeRootError> {
        if let State::ChallengeRoot {
            tx_hash: Some(tx_hash),
        } = &self.state
        {
            let tx_status = self.eth_api.get_tx_status(*tx_hash).await?;

            match tx_status {
                TxStatus::Finalized => {
                    log::info!("Challenge root tx finalized, resuming normal operation");
                    self.state = State::SubmitMerkleRoot { tx_hash: None };
                    return Ok(());
                }
                TxStatus::Pending => {
                    log::info!("Challenge root tx is still pending, waiting ..");
                    return Ok(());
                }
                TxStatus::Failed => {
                    log::warn!("Tx #{tx_hash} failed. Re-trying challenge root tx finalization");
                }
            }
        };

        self.state = State::ChallengeRoot { tx_hash: None };
        let tx_hash = send_challege_root_to_ethereum(&self.eth_api).await?;
        self.state = State::ChallengeRoot {
            tx_hash: Some(tx_hash),
        };

        Ok(())
    }

    async fn submit_merkle_root(&mut self) -> Result<(), SubmitMerkleRootError> {
        if let State::SubmitMerkleRoot {
            tx_hash: Some(tx_hash),
        } = &self.state
        {
            let tx_status = self.eth_api.get_tx_status(*tx_hash).await?;

            match tx_status {
                TxStatus::Finalized => {
                    log::info!("Submit merkle root tx finalized, resuming normal operation");
                    self.state = State::Exit;
                    return Ok(());
                }
                TxStatus::Pending => {
                    log::info!("Submit merkle root tx is still pending, waiting ..");
                    return Ok(());
                }
                TxStatus::Failed => {
                    log::warn!(
                        "Tx #{tx_hash} failed. Re-trying submit merkle root tx finalization"
                    );
                }
            }
        };

        self.state = State::SubmitMerkleRoot { tx_hash: None };
        let proof = self
            .fetch_merkle_root_proof_from_relayer(self.challenged_block.expect("bad state"))
            .await?;
        let tx_hash = submit_merkle_root_to_ethereum(&self.eth_api, proof).await?;
        self.state = State::SubmitMerkleRoot {
            tx_hash: Some(tx_hash),
        };

        Ok(())
    }

    async fn compare_merkle_roots(&self, event: &MerkleRootEntry) -> Result<bool, MainLoopError> {
        let gear_api = self.api_provider.client();
        let block_hash = gear_api
            .block_number_to_hash(event.block_number as u32)
            .await?;
        let merkle_root = gear_api.fetch_queue_merkle_root(block_hash).await?;

        let is_matches = merkle_root == event.merkle_root;

        if !is_matches {
            log::info!(
                "Merkle root mismatch for block #{}, hash {:X?}, expected: {}, got: {}",
                event.block_number,
                block_hash,
                merkle_root,
                event.merkle_root,
            );
            self.metrics.merkle_root_mismatch_cnt.inc();
        }

        Ok(is_matches)
    }

    async fn fetch_merkle_root_proof_from_relayer(
        &self,
        block: u64,
    ) -> Result<FinalProof, SubmitMerkleRootError> {
        let body = MerkleRootBlocks {
            blocks: vec![block as u32],
        };

        let response: Vec<MerkleRootsResponse> = self
            .http_client
            .post(format!(
                "{}/get_merkle_root_proof",
                self.relayer_http_url.as_str()
            ))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let response = response
            .first()
            .ok_or_else(|| SubmitMerkleRootError::Other("Empty response from relayer".to_string()))?
            .clone();

        let proof = match response {
            MerkleRootsResponse::MerkleRootProof {
                proof,
                merkle_root,
                block_number,
                block_hash,
            } => {
                log::info!("Got proof for block {block_number}: {:?}", &proof);
                log::info!("Merkle root: {:?}", &merkle_root);
                log::info!("Block hash: {block_hash:?}");
                FinalProof {
                    proof,
                    merkle_root: merkle_root.to_fixed_bytes(),
                    block_number,
                }
            }
            MerkleRootsResponse::NoMerkleRootOnBlock { block_number } => {
                log::error!("No Merkle root found on block {block_number}");
                return Err(SubmitMerkleRootError::NoMerkleRootOnBlock { block_number });
            }
            MerkleRootsResponse::Failed { message } => {
                log::error!("Fetch merkle root request failed: {message}");
                return Err(SubmitMerkleRootError::Other(message));
            }
        };

        Ok(proof)
    }
}
