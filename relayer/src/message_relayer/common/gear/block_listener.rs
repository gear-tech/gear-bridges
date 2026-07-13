use crate::{
    message_relayer::common::{
        gear::block_storage::{UnprocessedBlocks, UnprocessedBlocksStorage},
        GearBlock,
    },
    rpc,
};
use futures::StreamExt;
use gear_common::api_provider::ApiProviderConnection;
use prometheus::IntGauge;
use std::sync::Arc;
use tokio::sync::broadcast;
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct BlockListener {
    api_provider: ApiProviderConnection,

    block_storage: Arc<dyn UnprocessedBlocksStorage>,
    relayer_id: String,

    metrics: Metrics,
}

impl MeteredService for BlockListener {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_block: IntGauge = IntGauge::new(
            "gear_block_listener_latest_block",
            "Latest gear block discovered by gear block listener",
        )
    }
}

impl BlockListener {
    pub fn new(
        api_provider: ApiProviderConnection,
        block_storage: Arc<dyn UnprocessedBlocksStorage>,
    ) -> Self {
        Self::new_for_relayer(api_provider, block_storage, "unlabeled".to_string())
    }

    pub fn new_for_relayer(
        api_provider: ApiProviderConnection,
        block_storage: Arc<dyn UnprocessedBlocksStorage>,
        relayer_id: String,
    ) -> Self {
        Self {
            api_provider,
            block_storage,
            relayer_id,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<GearBlock>; RECEIVER_COUNT] {
        // Capacity for the channel. At the moment merkle-root relayer might lag behind
        // during proof generation or era sync, so we need to have enough capacity
        // to not drop any blocks. 14400 is how many blocks are produced in 1 era.
        const CAPACITY: usize = 14_400;
        let (tx, _) = broadcast::channel(CAPACITY);
        let tx2 = tx.clone();
        tokio::task::spawn(async move {
            let relayer_id = self.relayer_id.clone();
            let UnprocessedBlocks {
                last_block,
                first_block,
                blocks: _,
            } = self.block_storage.unprocessed_blocks().await;
            let mut last_finalized_block_number = None;
            if let Some(from_block) = first_block.or(last_block) {
                log::info!(
                    "Gear block listener for relayer {relayer_id}: unprocessed blocks found, replaying from #{} in background",
                    from_block.1
                );
                self.spawn_replay_to_latest(
                    tx2.clone(),
                    from_block.1,
                    &mut last_finalized_block_number,
                    "startup catch-up",
                )
                .await;
            }

            loop {
                let res = self.run_inner(&tx2, &mut last_finalized_block_number).await;
                let e = match res {
                    Ok(false) => {
                        log::info!(
                            "Gear block listener for relayer {relayer_id} stopped due to no active receivers"
                        );
                        return;
                    }

                    Ok(true) => {
                        log::info!(
                            "Gear block listener for relayer {relayer_id}: subscription expired, restarting"
                        );
                        continue;
                    }

                    Err(e) => e,
                };

                log::error!(r#"Gear block listener for relayer {relayer_id} failed: "{e:?}""#);

                if let Err(e) = self.api_provider.reconnect().await {
                    log::error!(
                        r#"Gear block listener for relayer {relayer_id}: API provider unable to reconnect: "{e}""#
                    );
                    continue;
                }

                log::debug!(
                    "Gear block listener for relayer {relayer_id}: API provider reconnected"
                );
                let from_block = last_finalized_block_number
                    .map(|block| block.saturating_add(1))
                    .unwrap_or_default();
                self.spawn_replay_to_latest(
                    tx2.clone(),
                    from_block,
                    &mut last_finalized_block_number,
                    "reconnect replay",
                )
                .await;
            }
        });

        (0..RECEIVER_COUNT)
            .map(|_| tx.subscribe())
            .collect::<Vec<_>>()
            .try_into()
            .expect("expected Vec of correct length")
    }

    async fn run_inner(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        last_finalized_block_number: &mut Option<u32>,
    ) -> anyhow::Result<bool> {
        let gear_api = self.api_provider.client();

        let mut subscription = gear_api.subscribe_grandpa_justifications().await?;
        while let Some(justification) = subscription.next().await {
            let justification = justification?;

            let block_hash = justification.commit.target_hash;
            let block_number = justification.commit.target_number;

            // Check if there are missing blocks and fetch them
            if let Some(last_finalized) = *last_finalized_block_number {
                if last_finalized + 1 != block_number {
                    log::info!("Gear block listener for relayer {}: detected gap: last finalized block was #{last_finalized}, current block is #{block_number}", self.relayer_id);

                    self.spawn_replay_range(
                        tx.clone(),
                        last_finalized + 1,
                        block_number.saturating_sub(1),
                        "live gap replay",
                    );
                }
            }

            // Process the current block
            if !self
                .fetch_store_send(tx, block_number, Some(block_hash.0.into()))
                .await?
            {
                return Ok(false);
            }

            // Update the last finalized block number
            *last_finalized_block_number = Some(block_number);
            self.metrics.latest_block.set(block_number as i64);
        }

        Ok(true)
    }

    async fn spawn_replay_to_latest(
        &mut self,
        tx: broadcast::Sender<GearBlock>,
        from_block: u32,
        last_finalized_block_number: &mut Option<u32>,
        reason: &'static str,
    ) {
        let latest = {
            let client = self.api_provider.client();
            match client.latest_finalized_block().await {
                Ok(hash) => match client.block_hash_to_number(hash).await {
                    Ok(number) => Some(number),
                    Err(err) => {
                        log::warn!(
                            "Gear block listener for relayer {} failed to inspect latest finalized block number for {reason}: {err}. Background replay will retry latest lookup",
                            self.relayer_id
                        );
                        None
                    }
                },
                Err(err) => {
                    log::warn!(
                    "Gear block listener for relayer {} failed to inspect latest finalized block for {reason}: {err}. Background replay will retry latest lookup",
                    self.relayer_id
                );
                    None
                }
            }
        };

        if let Some(latest) = latest {
            if from_block > latest {
                return;
            }
            *last_finalized_block_number = Some(
                last_finalized_block_number
                    .map(|current| current.max(latest))
                    .unwrap_or(latest),
            );
            self.spawn_replay_range(tx, from_block, latest, reason);
        } else {
            spawn_replay_to_latest(
                self.api_provider.clone(),
                self.block_storage.clone(),
                self.relayer_id.clone(),
                tx,
                from_block,
                reason,
            );
        }
    }

    fn spawn_replay_range(
        &self,
        tx: broadcast::Sender<GearBlock>,
        from_block: u32,
        to_block: u32,
        reason: &'static str,
    ) {
        if from_block > to_block {
            return;
        }

        spawn_replay_range(
            self.api_provider.clone(),
            self.block_storage.clone(),
            self.relayer_id.clone(),
            tx,
            from_block,
            to_block,
            reason,
        );
    }

    async fn fetch_store_send(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        block_number: u32,
        known_hash: Option<primitive_types::H256>,
    ) -> anyhow::Result<bool> {
        let storage = self.block_storage.clone();
        let gear_block = rpc::retry_gear(
            &mut self.api_provider,
            "gear finalized block replay",
            move |api| {
                let storage = storage.clone();
                async move {
                    let block_hash = match known_hash {
                        Some(hash) => hash,
                        None => api.block_number_to_hash(block_number).await?,
                    };
                    let block = api.api.blocks().at(block_hash).await?;
                    let gear_block = GearBlock::from_subxt_block(&api, block).await?;
                    storage.add_block(&api, &gear_block).await?;
                    Ok(gear_block)
                }
            },
        )
        .await?;
        if tx.send(gear_block).is_err() {
            log::error!(
                "Gear block listener for relayer {}: no active receivers, stopping",
                self.relayer_id
            );
            return Ok(false);
        }
        Ok(true)
    }
}

fn spawn_replay_to_latest(
    mut api_provider: ApiProviderConnection,
    storage: Arc<dyn UnprocessedBlocksStorage>,
    relayer_id: String,
    tx: broadcast::Sender<GearBlock>,
    from_block: u32,
    reason: &'static str,
) {
    tokio::spawn(async move {
        let latest = match rpc::retry_gear(
            &mut api_provider,
            "gear background latest finalized block",
            |api| async move {
                let hash = api.latest_finalized_block().await?;
                api.block_hash_to_number(hash).await
            },
        )
        .await
        {
            Ok(latest) => latest,
            Err(err) => {
                log::error!(
                    "Gear block listener for relayer {relayer_id} {reason} failed to fetch latest block: {err}"
                );
                return;
            }
        };

        replay_range(
            api_provider,
            storage,
            relayer_id,
            tx,
            from_block,
            latest,
            reason,
        )
        .await;
    });
}

fn spawn_replay_range(
    api_provider: ApiProviderConnection,
    storage: Arc<dyn UnprocessedBlocksStorage>,
    relayer_id: String,
    tx: broadcast::Sender<GearBlock>,
    from_block: u32,
    to_block: u32,
    reason: &'static str,
) {
    tokio::spawn(async move {
        replay_range(
            api_provider,
            storage,
            relayer_id,
            tx,
            from_block,
            to_block,
            reason,
        )
        .await;
    });
}

async fn replay_range(
    mut api_provider: ApiProviderConnection,
    storage: Arc<dyn UnprocessedBlocksStorage>,
    relayer_id: String,
    tx: broadcast::Sender<GearBlock>,
    from_block: u32,
    to_block: u32,
    reason: &'static str,
) {
    log::info!(
        "Gear block listener for relayer {relayer_id} {reason}: replaying blocks #{from_block}..=#{to_block}"
    );
    for block_number in from_block..=to_block {
        log::trace!(
            "Gear block listener for relayer {relayer_id} {reason}: replaying finalized block #{block_number}"
        );
        let storage = storage.clone();
        let gear_block = match rpc::retry_gear(
            &mut api_provider,
            "gear background finalized block replay",
            move |api| {
                let storage = storage.clone();
                async move {
                    let block_hash = api.block_number_to_hash(block_number).await?;
                    let block = api.api.blocks().at(block_hash).await?;
                    let gear_block = GearBlock::from_subxt_block(&api, block).await?;
                    storage.add_block(&api, &gear_block).await?;
                    Ok::<_, anyhow::Error>(gear_block)
                }
            },
        )
        .await
        {
            Ok(block) => block,
            Err(err) => {
                log::error!(
                    "Gear block listener for relayer {relayer_id} {reason}: failed to replay block #{block_number}: {err}"
                );
                return;
            }
        };

        if tx.send(gear_block).is_err() {
            log::info!(
                "Gear block listener for relayer {relayer_id} {reason}: no active receivers, stopping replay"
            );
            return;
        }
    }
    log::info!("Gear block listener for relayer {relayer_id} {reason}: replay finished");
}
