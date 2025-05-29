use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use ethereum_client::{EthApi, abi::IRelayer::MerkleRoot};
use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};
use alloy::{providers::{Provider, PendingTransactionBuilder}, sol_types::SolEvent};
use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::{
        common::{AuthoritySetId, GearBlockNumber, RelayedMerkleRoot},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};
use futures::StreamExt;

pub struct MerkleRootExtractor {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,
    confirmations: u64,

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
    pub fn new(eth_api: EthApi, api_provider: ApiProviderConnection, 
        confirmations: u64,) -> Self {
        Self {
            eth_api,
            api_provider,
            confirmations,

            metrics: Metrics::new(),
        }
    }

    pub fn spawn(
        self,
    ) -> UnboundedReceiver<RelayedMerkleRoot> {
        let (sender, receiver) = unbounded_channel();

        tokio::task::spawn(task(self, sender));

        receiver
    }
}

async fn task(
    mut this: MerkleRootExtractor,
    sender: UnboundedSender<RelayedMerkleRoot>,
) {
    let mut attempts = 0;

    loop {
        let res = task_inner(&this, &sender).await;
        if let Err(err) = res {
            attempts += 1;
            log::error!(
                "Merkle root extractor failed (attempt {}/{}): {}. Retrying in {:?}...",
                attempts,
                MAX_RETRIES,
                err,
                BASE_RETRY_DELAY * 2u32.pow(attempts - 1),
            );
            if attempts >= MAX_RETRIES {
                log::error!("Merkle root extractor failed {} times: {}", attempts, err);
                break;
            }

            tokio::time::sleep(BASE_RETRY_DELAY * 2u32.pow(attempts - 1)).await;

            match this.api_provider.reconnect().await {
                Ok(()) => {
                    log::info!("API provider reconnected");
                }

                Err(err) => {
                    log::error!("Merkle root extractor unable to reconnect: {err}");
                    return;
                }
            }

            if common::is_transport_error_recoverable(&err) {
                this.eth_api = match this.eth_api.reconnect().await {
                    Ok(eth_api) => eth_api,
                    Err(err) => {
                        log::error!("Failed to reconnect to Ethereum: {}", err);
                        break;
                    }
                };
            } else {
                log::error!("Merkle root extractor failed: {}", err);
                break;
            }
        } else {
            log::info!("Exiting");
            break;
        }
    }
}

async fn task_inner(
    this: &MerkleRootExtractor,
    sender: &UnboundedSender<RelayedMerkleRoot>,
) -> anyhow::Result<()> {
    let gear_api = this.api_provider.client();
    let subscription = this.eth_api.subscribe_logs().await?;

    let mut stream = subscription.into_result_stream();
    while let Some(Ok(log)) = stream.next().await {
        log::debug!(
            "Get log = {log:?}"
        );

        let (Some(tx_hash), Some(block_number)) = (log.transaction_hash, log.block_number) else {
            log::error!("Unable to get tx_hash and block_number for log = {log:?}. Skipping");
            continue;
        };

        if log.removed {
            log::debug!("Blocks reorganization, log = {log:?}. Skipping");
            continue;
        }

        let root = match MerkleRoot::decode_log_data(log.data(), true) {
            Ok(root) => root,
            Err(e) => {
                log::error!("Failed to decode log = {log:?}: {e:?}. Skipping");
                continue;
            }
        };

        let pending = PendingTransactionBuilder::new(this.eth_api.raw_provider().root().clone(), tx_hash);
        pending
            .with_required_confirmations(this.confirmations)
            .watch()
            .await?;

        log::info!(
            "Found merkle root {:?} at Ethereum block #{block_number} ({} confirmation(s))",
            (root.blockNumber, root.merkleRoot),
            this.confirmations,
        );

        let block_number_gear: u32 = root.blockNumber.to();
        this.metrics
            .latest_merkle_root_for_block
            .set(block_number_gear as i64);

        let block_hash = gear_api
            .block_number_to_hash(block_number_gear)
            .await?;

        let authority_set_id =
            AuthoritySetId(gear_api.signed_by_authority_set_id(block_hash).await?);

        log::info!(
            "Merkle root {:?} is for era #{authority_set_id}",
            (root.blockNumber, root.merkleRoot),
        );

        sender.send(RelayedMerkleRoot {
            block: GearBlockNumber(block_number_gear),
            block_hash,
            authority_set_id,
            merkle_root: root.merkleRoot.0.into(),
        })?;
    }

    Ok(())
}
