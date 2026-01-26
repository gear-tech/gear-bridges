use crate::{
    common,
    message_relayer::{
        common::{AuthoritySetId, GearBlockNumber, RelayedMerkleRoot},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};
use alloy::{
    providers::{PendingTransactionBuilder, Provider},
    rpc::types::Log,
    sol_types::SolEvent,
};
use ethereum_client::{abi::IMessageQueue::MerkleRoot, EthApi};
use futures::StreamExt;
use gear_rpc_client::GearApi;
use prometheus::IntGauge;
use tokio::sync::mpsc::UnboundedSender;
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct MerkleRootExtractor {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,
    confirmations: u64,
    sender: UnboundedSender<RelayedMerkleRoot>,

    metrics: Metrics,
}

impl MeteredService for MerkleRootExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        latest_merkle_root_for_block: IntGauge = IntGauge::new(
            "merkle_root_extractor_latest_merkle_root_for_block",
            "Latest gear block present in found merkle roots",
        )
    }
}

impl MerkleRootExtractor {
    pub fn new(
        eth_api: EthApi,
        api_provider: ApiProviderConnection,
        confirmations: u64,
        sender: UnboundedSender<RelayedMerkleRoot>,
    ) -> Self {
        Self {
            eth_api,
            api_provider,
            confirmations,
            sender,

            metrics: Metrics::new(),
        }
    }

    pub fn sender(&self) -> &UnboundedSender<RelayedMerkleRoot> {
        &self.sender
    }

    pub fn spawn(self) {
        tokio::task::spawn(task(self));
    }
}

async fn task(mut this: MerkleRootExtractor) {
    let mut attempts = 0;
    let mut pending_log: Option<Log> = None;

    loop {
        let res = task_inner(&this, &mut pending_log).await;
        if let Err(err) = res {
            attempts += 1;
            common::retry_backoff(attempts, "Merkle root extractor", &err).await;

            // Infinite retry for reconnection
            loop {
                match this.api_provider.reconnect().await {
                    Ok(()) => {
                        log::info!("API provider reconnected");
                    }
                    Err(err) => {
                        log::error!("Merkle root extractor unable to reconnect (1): {err}. Retrying in 5s...");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue;
                    }
                }

                if common::is_transport_error_recoverable(&err) {
                    match this.eth_api.reconnect().await {
                        Ok(eth_api) => {
                            this.eth_api = eth_api;
                            log::info!("Ethereum API reconnected");
                            break;
                        }
                        Err(err) => {
                            log::error!(
                                "Failed to reconnect to Ethereum: {err}. Retrying in 5s..."
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    };
                } else {
                    // Non transport error: Exit the task clearly.
                    return;
                }
            }
        }
    }
}

async fn task_inner(
    this: &MerkleRootExtractor,
    pending_log: &mut Option<alloy::rpc::types::Log>,
) -> anyhow::Result<()> {
    let gear_api = this.api_provider.client();

    // Process pending log first
    if let Some(log) = pending_log.take() {
        match process_log(this, &gear_api, log.clone()).await {
            Ok(_) => {}
            Err(e) => {
                *pending_log = Some(log);
                return Err(e);
            }
        }
    }

    let subscription = this.eth_api.subscribe_logs().await?;
    let mut stream = subscription.into_result_stream();
    // check periodically that the connection to ApiProvider is alive
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !this.api_provider.is_alive() {
                    log::error!("ApiProvider connection is dead, exiting Merkle root extractor task");
                    // Return error to trigger reconnect
                    return Err(anyhow::anyhow!("ApiProvider connection died"));
                }
            }

            log = stream.next() => {
                let log = match log {
                    Some(Ok(log)) => log,
                    Some(Err(e)) => {
                        return Err(anyhow::anyhow!("Failed to get first log from stream: {e:?}"));
                    }
                    None => {
                        log::info!("Log stream closed");
                        return Ok(());
                    }
                };

                match process_log(this, &gear_api, log.clone()).await {
                     Ok(_) => {},
                     Err(e) => {
                         *pending_log = Some(log);
                         return Err(e);
                     }
                 }
            }
        }
    }
}

async fn process_log(
    this: &MerkleRootExtractor,
    gear_api: &GearApi,
    log: Log,
) -> anyhow::Result<()> {
    log::debug!("Get log = {log:?}");

    let (Some(tx_hash), Some(block_number)) = (log.transaction_hash, log.block_number) else {
        log::error!("Unable to get tx_hash and block_number for log = {log:?}. Skipping");
        return Ok(());
    };

    if log.removed {
        log::debug!("Blocks reorganization, log = {log:?}. Skipping");
        return Ok(());
    }

    let root = match MerkleRoot::decode_log_data(log.data()) {
        Ok(root) => root,
        Err(e) => {
            log::error!("Failed to decode log = {log:?}: {e:?}. Skipping");
            return Ok(());
        }
    };

    let pending =
        PendingTransactionBuilder::new(this.eth_api.raw_provider().root().clone(), tx_hash);
    pending
        .with_required_confirmations(this.confirmations)
        .watch()
        .await?;

    let block_timestamp = this.eth_api.get_block_timestamp(block_number).await?;

    log::info!(
        "Found merkle root {:?} at Ethereum block #{block_number} with timestamp {block_timestamp} ({} confirmation(s))",
        (root.blockNumber, root.merkleRoot),
        this.confirmations,
    );

    let block_number_gear: u32 = root.blockNumber.to();
    this.metrics
        .latest_merkle_root_for_block
        .set(block_number_gear as i64);

    let block_hash = gear_api.block_number_to_hash(block_number_gear).await?;

    let authority_set_id = AuthoritySetId(gear_api.signed_by_authority_set_id(block_hash).await?);

    log::info!(
        "Merkle root {:?} is for era #{authority_set_id}",
        (root.blockNumber, root.merkleRoot),
    );

    this.sender.send(RelayedMerkleRoot {
        block: GearBlockNumber(block_number_gear),
        block_hash,
        authority_set_id,
        merkle_root: root.merkleRoot.0.into(),
        timestamp: block_timestamp,
    })?;

    Ok(())
}
