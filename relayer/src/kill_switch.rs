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

const SCAN_EVENTS_PERIOD_SEC: Duration = Duration::from_secs(12);
const ERROR_REPEAT_DELAY: Duration = Duration::from_secs(3);

impl_metered_service! {
    struct Metrics {
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
        challenge_sent_cnt: IntCounter = IntCounter::new(
            "kill_switch_challenge_sent_cnt",
            "Amount of challenge sends finalized",
        ),
        merkle_root_proof_not_found_on_block_cnt: IntCounter = IntCounter::new(
            "kill_switch_merkle_root_proof_not_found_on_block",
            "Amount of merkle root proofs not found on block",
        ),
    }
}

#[derive(Debug, Error)]
enum ScanForEventsError {
    #[error("Ethereum API error: {0}")]
    EthApi(#[from] ethereum_client::Error),
    #[error("Gear API error: {0}")]
    GearApi(#[from] anyhow::Error),
    #[error("Other error: {0}")]
    Other(anyhow::Error),
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
    #[error("Scan for events error: {0}")]
    ScanForEvents(#[from] ScanForEventsError),
    #[error("Challenge root error: {0}")]
    ChallengeRoot(#[from] ChallengeRootError),
    #[error("Submit merkle root error: {0}")]
    SubmitMerkleRoot(#[from] SubmitMerkleRootError),
}

enum State {
    // Scan for events (normal operation)
    ScanForEvents,
    // Waiting for challenge root tx to be finalized
    ChallengeRoot { tx_hash: Option<TxHash> },
    // Waiting for submit merkle root tx to be finalized
    SubmitMerkleRoot { tx_hash: Option<TxHash> },
    // Exit
    Exit,
}

pub struct KillSwitchRelayer {
    api_provider: ApiProviderConnection,
    eth_observer_api: EthApi,
    eth_admin_api: Option<EthApi>,
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
        eth_observer_api: EthApi,
        eth_admin_api: Option<EthApi>,
        http_client: HttpClient,
        from_eth_block: Option<u64>,
        relayer_http_url: String,
    ) -> Self {
        Self {
            api_provider,
            eth_observer_api,
            eth_admin_api,
            relayer_http_url,
            http_client,
            start_from_eth_block: from_eth_block,
            state: State::ScanForEvents,
            challenged_block: None,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        log::info!(
            "Starting kill switch relayer, with {}",
            if self.eth_admin_api.is_some() {
                "observer + admin roles"
            } else {
                "observer role"
            }
        );

        loop {
            let loop_delay;

            let res = match &self.state {
                State::ScanForEvents => self.scan_for_events().await.map_err(Error::from),
                State::ChallengeRoot { .. } => self.challenge_root().await.map_err(Error::from),
                State::SubmitMerkleRoot { .. } => {
                    self.submit_merkle_root().await.map_err(Error::from)
                }
                State::Exit => {
                    process::exit(1);
                }
            };

            match res {
                Ok(()) => {
                    loop_delay = if let State::ScanForEvents = self.state {
                        // Repeat merkle root event scan after a delay
                        SCAN_EVENTS_PERIOD_SEC
                    } else {
                        // New state entered or polling in Challenge/Submit states
                        Duration::from_secs(5)
                    };
                }
                Err(err) => {
                    loop_delay = ERROR_REPEAT_DELAY;

                    let eth_api = if let State::SubmitMerkleRoot { .. } = self.state {
                        self.eth_admin_api.as_ref().expect("bad state")
                    } else {
                        &self.eth_observer_api
                    };

                    match err {
                        Error::ScanForEvents(ScanForEventsError::EthApi(
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
                                if let Err(e) = eth_api.reconnect().await {
                                    log::error!("Ethereum API reconnect failed: {e}");
                                }
                            }
                        }
                        Error::ScanForEvents(ScanForEventsError::GearApi(e)) => {
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

            tokio::time::sleep(loop_delay).await;
        }
    }

    async fn scan_for_events(&mut self) -> Result<(), ScanForEventsError> {
        let balance = self.eth_observer_api.get_approx_balance().await?;
        self.metrics.fee_payer_balance.set(balance);

        let last_finalized_block = self
            .eth_observer_api
            .finalized_block_number()
            .await
            .map_err(downcast_anyhow_to_ethereum_client)?;

        let latest_block = self
            .eth_observer_api
            .latest_block_number()
            .await
            .map_err(downcast_anyhow_to_ethereum_client)?;

        // Set the initial value for `from_eth_block` if it's not set yet.
        let start_from_eth_block = self.start_from_eth_block.unwrap_or(last_finalized_block);

        log::info!(
            "Scanning.. last_processed_eth_block={}, last_finalized_block={}, latest_block={}",
            start_from_eth_block.saturating_sub(1),
            last_finalized_block,
            latest_block,
        );

        if latest_block < start_from_eth_block {
            log::info!("No new eth block, skipping..");
            return Ok(());
        } else {
            self.metrics.latest_eth_block.set(latest_block as i64);
        }

        let events = self
            .eth_observer_api
            .fetch_merkle_roots_in_range(start_from_eth_block, latest_block)
            .await?;

        if !events.is_empty() {
            // FIXME: doesn't work as intended, discovered events can be duplicated
            self.metrics
                .merkle_roots_discovered_cnt
                .inc_by(events.len() as u64);
        }

        for (event, _block_number_eth) in events {
            if !self.compare_merkle_roots(&event).await? {
                // Okay, we have a mismatch,
                // that means for some reason the proof with incorrect merkle root was submitted to relayer MQ contract.
                // We need to challenge it by submitting the correct merkle root.
                log::debug!("Got event with mismatched merkle root: {:?}", &event);

                // Switch to challenge root state
                self.state = State::ChallengeRoot { tx_hash: None };
                self.challenged_block = Some(event.block_number);

                return Ok(());
            }
        }

        // After processing events till `latest` block, due to possible reorgs we cannot count them as processed.
        // Instead we set the last finalized block as processed.
        // So, we need to increment it by 1 to set the next block to process.
        self.start_from_eth_block = Some(last_finalized_block.saturating_add(1));

        Ok(())
    }

    async fn challenge_root(&mut self) -> Result<(), ChallengeRootError> {
        if let State::ChallengeRoot {
            tx_hash: Some(tx_hash),
        } = &self.state
        {
            let tx_status = self.eth_observer_api.get_tx_status(*tx_hash).await?;

            match tx_status {
                TxStatus::Finalized => {
                    // For submit merkle root we need admin role
                    if self.eth_admin_api.is_some() {
                        log::info!("Challenge root TX {tx_hash:#x} finalized, switching to submit merkle root state");
                        self.state = State::SubmitMerkleRoot { tx_hash: None };
                    } else {
                        log::info!("Challenge root TX {tx_hash:#x} finalized, exiting ..");
                        self.state = State::Exit;
                    }
                    self.metrics.challenge_sent_cnt.inc();

                    return Ok(());
                }
                TxStatus::Pending => {
                    log::info!("Challenge root TX {tx_hash:#x} is still pending, waiting ..");
                    return Ok(());
                }
                TxStatus::Failed => {
                    log::warn!("Challenge root TX {tx_hash:#x} failed. Re-trying challenge root tx finalization");
                }
            }
        };

        self.state = State::ChallengeRoot { tx_hash: None };
        let tx_hash = send_challege_root_to_ethereum(&self.eth_observer_api).await?;
        self.state = State::ChallengeRoot {
            tx_hash: Some(tx_hash),
        };

        Ok(())
    }

    async fn submit_merkle_root(&mut self) -> Result<(), SubmitMerkleRootError> {
        let eth_admin_api = self
            .eth_admin_api
            .as_ref()
            .expect("PK for admin role is required to submit merkle root");

        if let State::SubmitMerkleRoot {
            tx_hash: Some(tx_hash),
        } = &self.state
        {
            let tx_status = eth_admin_api.get_tx_status(*tx_hash).await?;

            match tx_status {
                TxStatus::Finalized => {
                    log::info!("Submit merkle root TX {tx_hash:#x} finalized, exiting ..");
                    self.state = State::Exit;
                    return Ok(());
                }
                TxStatus::Pending => {
                    log::info!("Submit merkle root TX {tx_hash:#x} is still pending, waiting ..");
                    return Ok(());
                }
                TxStatus::Failed => {
                    log::warn!(
                        "Submit merkle root TX {tx_hash:#x} failed. Re-trying submit merkle root tx finalization"
                    );
                }
            }
        };

        self.state = State::SubmitMerkleRoot { tx_hash: None };
        let proof = self
            .fetch_merkle_root_proof_from_relayer(self.challenged_block.expect("bad state"))
            .await?;
        let tx_hash = *submit_merkle_root_to_ethereum(eth_admin_api, proof)
            .await?
            .tx_hash();
        self.state = State::SubmitMerkleRoot {
            tx_hash: Some(tx_hash),
        };

        Ok(())
    }

    async fn compare_merkle_roots(
        &self,
        event: &MerkleRootEntry,
    ) -> Result<bool, ScanForEventsError> {
        let gear_api = self.api_provider.client();
        let res = gear_api
            .block_number_to_hash(event.block_number as u32)
            .await
            .convert()?;

        let Some(block_hash) = res else {
            log::info!(
                "Block #{} is not present on Gear RPC node, cannot compare merkle roots",
                event.block_number,
            );
            return Ok(false);
        };

        let Some((_, merkle_root)) = gear_api
            .fetch_queue_merkle_root(block_hash)
            .await
            .convert()?
        else {
            log::info!(
                "Block #{} is not present on Gear RPC node, cannot compare merkle roots",
                event.block_number,
            );
            return Ok(false);
        };

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
                proof_block_number: _,
                merkle_root,
                block_number,
                block_hash,
            } => {
                log::debug!("Fetched proof for block {block_number}: {:?}", &proof);
                log::debug!("Merkle root: {:?}", &merkle_root);
                log::debug!("Block hash: {block_hash:?}");
                FinalProof {
                    proof,
                    merkle_root: merkle_root.to_fixed_bytes(),
                    block_number,
                }
            }
            MerkleRootsResponse::NoMerkleRootOnBlock { block_number } => {
                log::error!("No Merkle proof root found on block {block_number}");
                self.metrics.merkle_root_proof_not_found_on_block_cnt.inc();
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

fn is_block_no_present_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("not present on RPC node")
}

trait ConvertToOptGearApiError<T> {
    fn convert(self) -> anyhow::Result<Option<T>>;
}

impl<T> ConvertToOptGearApiError<T> for anyhow::Result<T> {
    fn convert(self) -> anyhow::Result<Option<T>> {
        match self {
            Err(err) if is_block_no_present_error(&err) => Ok(None),
            Err(err) => Err(err),
            Ok(val) => Ok(Some(val)),
        }
    }
}

fn downcast_anyhow_to_ethereum_client(err: anyhow::Error) -> ScanForEventsError {
    match err.downcast::<ethereum_client::Error>() {
        Ok(e) => ScanForEventsError::EthApi(e),
        Err(e) => ScanForEventsError::Other(e),
    }
}
