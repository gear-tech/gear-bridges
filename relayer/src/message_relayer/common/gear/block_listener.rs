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
                    "Gear block listener for relayer {relayer_id}: unprocessed blocks found, replaying from #{}",
                    from_block.1
                );
                match self
                    .replay_to_latest(
                        &tx2,
                        from_block.1,
                        &mut last_finalized_block_number,
                        "startup catch-up",
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(err) => {
                        log::error!(
                            "Gear block listener for relayer {relayer_id}: startup replay failed: {err}"
                        );
                        return;
                    }
                }
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
                match self
                    .replay_to_latest(
                        &tx2,
                        from_block,
                        &mut last_finalized_block_number,
                        "reconnect replay",
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(err) => {
                        log::error!(
                            "Gear block listener for relayer {relayer_id}: reconnect replay failed: {err}"
                        );
                        return;
                    }
                }
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

            // GRANDPA justifications commonly skip block numbers. Replay the gap
            // inline so slow archive RPCs cannot create overlapping replay tasks.
            if let Some(last_finalized) = *last_finalized_block_number {
                if block_number <= last_finalized {
                    log::trace!(
                        "Gear block listener for relayer {}: skipping already replayed finalized block #{block_number}",
                        self.relayer_id
                    );
                    continue;
                }

                if last_finalized.saturating_add(1) < block_number {
                    log::info!("Gear block listener for relayer {}: detected gap: last finalized block was #{last_finalized}, current block is #{block_number}", self.relayer_id);

                    if !self
                        .replay_gap(
                            tx,
                            last_finalized.saturating_add(1),
                            block_number.saturating_sub(1),
                            last_finalized_block_number,
                            "live gap replay",
                        )
                        .await?
                    {
                        return Ok(false);
                    }
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

    async fn replay_to_latest(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        from_block: u32,
        last_finalized_block_number: &mut Option<u32>,
        reason: &'static str,
    ) -> anyhow::Result<bool> {
        let latest = rpc::retry_gear(
            &mut self.api_provider,
            "gear latest finalized block for replay",
            |api| async move {
                let hash = api.latest_finalized_block().await?;
                api.block_hash_to_number(hash).await
            },
        )
        .await?;

        if from_block > latest {
            return Ok(true);
        }

        self.replay_gap(tx, from_block, latest, last_finalized_block_number, reason)
            .await
    }

    async fn replay_gap(
        &mut self,
        tx: &broadcast::Sender<GearBlock>,
        from_block: u32,
        to_block: u32,
        last_finalized_block_number: &mut Option<u32>,
        reason: &'static str,
    ) -> anyhow::Result<bool> {
        log::info!(
            "Gear block listener for relayer {} {reason}: replaying blocks #{from_block}..=#{to_block}",
            self.relayer_id
        );
        for block_number in from_block..=to_block {
            log::trace!(
                "Gear block listener for relayer {} {reason}: replaying finalized block #{block_number}",
                self.relayer_id
            );
            if !self.fetch_store_send(tx, block_number, None).await? {
                return Ok(false);
            }
            *last_finalized_block_number = Some(block_number);
            self.metrics.latest_block.set(block_number as i64);
        }
        log::info!(
            "Gear block listener for relayer {} {reason}: replay finished",
            self.relayer_id
        );
        Ok(true)
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
