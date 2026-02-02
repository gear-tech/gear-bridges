use crate::message_relayer::{
    common::{AuthoritySetId, GearBlockNumber, RelayedMerkleRoot},
    eth_to_gear::api_provider::ApiProviderConnection,
};
use alloy::{
    providers::{PendingTransactionBuilder, Provider},
    sol_types::SolEvent,
};
use ethereum_client::{abi::IMessageQueue::MerkleRoot, EthApi};
use futures::StreamExt;
use gear_rpc_client::GearApi;
use primitive_types::H256;
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

    async fn fetch_hash_auth_id(
        &mut self,
        block_number_gear: u32,
    ) -> Option<(H256, AuthoritySetId)> {
        loop {
            let gear_api = self.api_provider.client();

            match self::fetch_hash_auth_id(&gear_api, block_number_gear).await {
                Ok(result) => return Some(result),

                Err(e) => {
                    log::error!(r#"Merkle root extractor failed to fetch block_hash: "{e:?}""#);
                    log::trace!(
                        r#"e.downcast_ref::<gsdk::Error>(): "{:?}""#,
                        e.downcast_ref::<gsdk::Error>()
                    );
                    log::trace!(
                        r#"e.downcast_ref::<subxt::Error>(): "{:?}""#,
                        e.downcast_ref::<subxt::Error>()
                    );
                    for cause in e.chain() {
                        log::trace!(r#"cause: "{cause:?}""#);
                    }
                }
            }

            if let Err(e) = self.api_provider.reconnect().await {
                log::error!(r#"Merkle root extractor unable to reconnect: "{e}""#);
                return None;
            }

            log::debug!("API provider reconnected");
        }
    }
}

async fn fetch_hash_auth_id(
    gear_api: &GearApi,
    block_number_gear: u32,
) -> anyhow::Result<(H256, AuthoritySetId)> {
    let block_hash = gear_api.block_number_to_hash(block_number_gear).await?;

    let authority_set_id = AuthoritySetId(gear_api.signed_by_authority_set_id(block_hash).await?);

    Ok((block_hash, authority_set_id))
}

async fn task(mut this: MerkleRootExtractor) {
    loop {
        let Err(err) = task_inner(&mut this).await else {
            log::info!("Exiting");
            break;
        };

        log::error!(r#"Merkle root extractor failed: "{err:?}""#);

        this.eth_api = match this.eth_api.reconnect().await {
            Ok(eth_api) => eth_api,
            Err(err) => {
                log::error!(r#"Failed to reconnect to Ethereum: "{err}""#);
                break;
            }
        };
    }
}

async fn task_inner(this: &mut MerkleRootExtractor) -> anyhow::Result<()> {
    let subscription = this.eth_api.subscribe_logs().await?;

    let mut stream = subscription.into_result_stream();
    // check periodically that the connection to ApiProvider is alive
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !this.api_provider.is_alive() {
                    log::error!("ApiProvider connection is dead, exiting Merkle root extractor task");
                    return Ok(());
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

                log::debug!("Get log = {log:?}");

                let (Some(tx_hash), Some(block_number)) = (log.transaction_hash, log.block_number) else {
                    log::error!("Unable to get tx_hash and block_number for log = {log:?}. Skipping");
                    continue;
                };

                if log.removed {
                    log::debug!("Blocks reorganization, log = {log:?}. Skipping");
                    continue;
                }

                let root = match MerkleRoot::decode_log_data(log.data()) {
                    Ok(root) => root,
                    Err(e) => {
                        log::error!("Failed to decode log = {log:?}: {e:?}. Skipping");
                        continue;
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

                let Some((block_hash, authority_set_id)) = this.fetch_hash_auth_id(block_number_gear).await else {
                    return Ok(());
                };

                log::info!(
                    "Merkle root {:?} is for era #{authority_set_id}",
                    (root.blockNumber, root.merkleRoot),
                );

                if let Err(e) = this.sender.send(RelayedMerkleRoot {
                    block: GearBlockNumber(block_number_gear),
                    block_hash,
                    authority_set_id,
                    merkle_root: root.merkleRoot.0.into(),
                    timestamp: block_timestamp,
                }) {
                    log::error!(r#"Sender channel closed: "{e:?}"."#);
                    return Ok(());
                }
            }
        }
    }
}
