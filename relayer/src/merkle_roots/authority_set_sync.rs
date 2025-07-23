use crate::{
    common::{sync_authority_set_id, SyncStepCount},
    message_relayer::{common::GearBlock, eth_to_gear::api_provider::ApiProviderConnection},
    proof_storage::ProofStorage,
};
use futures::executor::block_on;
use prometheus::IntGauge;
use prover::proving::GenesisConfig;
use std::sync::Arc;
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct AuthoritySetSyncIo {
    response: UnboundedReceiver<Response>,
    requests: UnboundedSender<GearBlock>,
}

pub enum Response {
    AuthoritySetSynced(u64, u32),
}

impl AuthoritySetSyncIo {
    pub fn new(
        response: UnboundedReceiver<Response>,
        requests: UnboundedSender<GearBlock>,
    ) -> Self {
        Self { response, requests }
    }

    pub async fn recv(&mut self) -> Option<Response> {
        self.response.recv().await
    }

    pub fn send(&self, block: GearBlock) -> bool {
        self.requests.send(block).is_ok()
    }
}

impl_metered_service!(
    struct Metrics {
        latest_proven_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_proven_era",
            "Latest proven era number",
        ),
        latest_observed_gear_era: IntGauge = IntGauge::new(
            "merkle_root_relayer_latest_observed_gear_era",
            "Latest era number observed by relayer",
        ),
    }
);

/// Authority set sync task which is responsible for synchronizing
/// authority set in proof-storage and generating new proofs.
///
/// Once proof is generated it is sent to merkle root relayer for further
/// processing.
pub struct AuthoritySetSync {
    api_provider: ApiProviderConnection,
    proof_storage: Arc<dyn ProofStorage>,
    genesis_config: GenesisConfig,

    metrics: Metrics,
}

impl MeteredService for AuthoritySetSync {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources().into_iter()
    }
}

impl AuthoritySetSync {
    pub async fn new(
        api_provider: ApiProviderConnection,
        proof_storage: Arc<dyn ProofStorage>,
        genesis_config: GenesisConfig,
    ) -> Self {
        Self {
            api_provider,
            proof_storage,
            genesis_config,

            metrics: Metrics::new(),
        }
    }

    pub fn run(mut self, mut blocks: Receiver<GearBlock>) -> AuthoritySetSyncIo {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (req_tx, mut req_rx) = tokio::sync::mpsc::unbounded_channel();

        let io = AuthoritySetSyncIo::new(rx, req_tx);

        tokio::task::spawn_blocking(move || {
            block_on(async move {
                loop {
                    if let Err(err) = self.process(&mut blocks, &tx, &mut req_rx).await {
                        log::error!("Authority set sync task failed: {err}");

                        match self.api_provider.reconnect().await {
                            Ok(_) => {
                                log::info!("Reconnected to Gear API, resuming authority set sync");
                                continue;
                            }
                            Err(err) => {
                                log::error!("Failed to reconnect to Gear API: {err}");
                                return;
                            }
                        }
                    } else {
                        log::info!("Authority set sync task terminated");
                        break;
                    }
                }
            })
        });

        io
    }

    async fn process(
        &mut self,
        blocks: &mut Receiver<GearBlock>,
        responses: &UnboundedSender<Response>,
        force_sync: &mut UnboundedReceiver<GearBlock>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                block = force_sync.recv() => {
                    match block {
                        Some(block) => {
                            log::info!("Force syncing authority set for block #{}", block.number());
                            let Some(_) = self.sync_authority_set_completely(&block, blocks, responses).await? else {
                                return Ok(());
                            };
                        }
                        None => {
                            log::warn!("Force sync channel closed, exiting");
                            return Ok(());
                        }
                    }
                }

                block = blocks.recv() => {
                    match block {
                        Ok(block) => {
                            if !super::storage::authority_set_changed(&block) {
                                continue;
                            }

                            let Some(_) = self.sync_authority_set_completely(&block, blocks, responses).await? else {
                                return Ok(());
                            };
                        }

                        Err(RecvError::Lagged(n)) => {
                            log::error!(
                                "Gear block listener lagged behind {n} blocks, skipping some blocks"
                            );
                            continue;
                        }

                        Err(RecvError::Closed) => {
                            log::warn!("Gear block listener connection closed, exiting");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    async fn sync_authority_set_completely(
        &mut self,
        initial_block: &GearBlock,
        blocks: &mut Receiver<GearBlock>,
        responses: &UnboundedSender<Response>,
    ) -> anyhow::Result<Option<u64>> {
        let (sync_steps, authority_set_id) = self.sync_authority_set(initial_block).await?;
        if sync_steps == 0 {
            log::info!(
                "Authority set #{authority_set_id} is already in sync at block #{}",
                initial_block.number()
            );
            return Ok(Some(authority_set_id));
        }

        log::info!("Syncing authority set #{authority_set_id}");
        loop {
            let (sync_steps, _) = match blocks.recv().await {
                Ok(block) => self.sync_authority_set(&block).await?,

                Err(RecvError::Closed) => {
                    log::warn!("Gear block listener connection closed");
                    return Ok(None);
                }

                Err(RecvError::Lagged(n)) => {
                    log::error!(
                        "Gear block listener lagged behind {n} blocks, skipping some blocks"
                    );
                    continue;
                }
            };

            if sync_steps == 0 {
                break;
            } else {
                log::info!("Synced {sync_steps} authority sets");
            }
        }

        log::info!("Authority set #{authority_set_id} is in sync");

        if responses
            .send(Response::AuthoritySetSynced(
                authority_set_id,
                initial_block.number(),
            ))
            .is_err()
        {
            return Ok(None);
        }
        Ok(Some(authority_set_id))
    }

    async fn sync_authority_set(
        &mut self,
        block: &GearBlock,
    ) -> anyhow::Result<(SyncStepCount, u64)> {
        let gear_api = self.api_provider.client();

        let finalized_head = block.hash();
        let latest_authority_set_id = gear_api.authority_set_id(finalized_head).await?;

        self.metrics
            .latest_observed_gear_era
            .set(latest_authority_set_id as i64);
        let latest_proven_authority_set_id = self.proof_storage.get_latest_authority_set_id().await;

        if let Some(&latest_proven) = latest_proven_authority_set_id.as_ref() {
            self.metrics.latest_proven_era.set(latest_proven as i64);
        }

        Ok((
            sync_authority_set_id(
                &gear_api,
                &self.proof_storage,
                self.genesis_config,
                latest_authority_set_id,
                latest_proven_authority_set_id,
            )
            .await?,
            latest_authority_set_id,
        ))
    }
}
