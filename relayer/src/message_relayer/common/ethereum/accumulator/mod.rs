pub mod utils;

use super::*;
use crate::{common::BASE_RETRY_DELAY, message_relayer::common::AuthoritySetId};
use ethereum_client::EthApi;
use primitive_types::H256;
use prometheus::IntGauge;
use sails_rs::ActorId;
use std::sync::Arc;
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    RwLock,
};
use utils::{Added, MerkleRoots, Messages};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub authority_set_id: AuthoritySetId,
    pub block: GearBlockNumber,
    pub block_hash: H256,
    pub tx_uuid: Uuid,
    pub source: ActorId,
}

pub enum Response {
    /// Merkle-root was relayed and the message can be processed further.
    Success {
        authority_set_id: AuthoritySetId,
        block: GearBlockNumber,
        tx_uuid: Uuid,
        merkle_root: RelayedMerkleRoot,
    },

    /// Accumulator buffer is full and the message cannot be processed.
    Overflowed(Request),

    /// Message is stuck and cannot be processed.
    Stuck {
        authority_set_id: AuthoritySetId,
        block: GearBlockNumber,
        tx_uuid: Uuid,
        merkle_root: RelayedMerkleRoot,
    },
}

pub struct AccumulatorIo {
    messages_with_roots: UnboundedReceiver<Response>,
    messages: UnboundedSender<Request>,
}

impl AccumulatorIo {
    pub fn new(
        messages_with_roots: UnboundedReceiver<Response>,
        messages: UnboundedSender<Request>,
    ) -> Self {
        Self {
            messages_with_roots,
            messages,
        }
    }

    pub fn send_message(
        &self,
        tx_uuid: Uuid,
        authority_set_id: AuthoritySetId,
        block: GearBlockNumber,
        block_hash: H256,
        source: ActorId,
    ) -> bool {
        let request = Request {
            authority_set_id,
            block,
            block_hash,
            tx_uuid,
            source,
        };
        self.messages.send(request).is_ok()
    }

    pub async fn recv_message(&mut self) -> Option<Response> {
        self.messages_with_roots.recv().await
    }
}

/// Struct accumulates gear-eth messages and required merkle roots.
pub struct Accumulator {
    metrics: Metrics,
    messages: Messages,
    merkle_roots: Arc<RwLock<MerkleRoots>>,
    receiver_roots: UnboundedReceiver<RelayedMerkleRoot>,
    governance_admin: ActorId,
    governance_pauser: ActorId,
    eth_api: EthApi,
}

impl MeteredService for Accumulator {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        message_count: IntGauge = IntGauge::new(
            "ethereum_accumulator_message_count",
            "Count of waiting messages",
        ),
    }
}

impl Accumulator {
    pub fn new(
        receiver_roots: UnboundedReceiver<RelayedMerkleRoot>,
        merkle_roots: Arc<RwLock<MerkleRoots>>,
        governance_admin: ActorId,
        governance_pauser: ActorId,
        eth_api: EthApi,
    ) -> Self {
        Self {
            metrics: Metrics::new(),
            messages: Messages::new(10_000),
            merkle_roots,
            receiver_roots,
            governance_admin,
            governance_pauser,
            eth_api,
        }
    }

    pub fn spawn(mut self) -> AccumulatorIo {
        let (requests_in, mut requests_out) = mpsc::unbounded_channel();
        let (mut messages_out, receiver) = mpsc::unbounded_channel();
        tokio::task::spawn(async move {
            loop {
                match run_inner(&mut self, &mut messages_out, &mut requests_out).await {
                    Ok(_) => break,
                    Err(e) => {
                        log::error!("{e:?}");

                        tokio::time::sleep(BASE_RETRY_DELAY).await;
                        self.eth_api = match self.eth_api.reconnect().await {
                            Ok(api) => api,
                            Err(e) => {
                                log::error!("Failed to reconnect to Ethereum API: {e:?}");
                                return;
                            }
                        };
                    }
                }
            }
        });

        AccumulatorIo::new(receiver, requests_in)
    }
}

async fn run_inner(
    this: &mut Accumulator,
    messages_out: &mut UnboundedSender<Response>,
    requests: &mut UnboundedReceiver<Request>,
) -> anyhow::Result<()> {
    let mut last_block = this.eth_api.finalized_block_number().await?;
    let mut last_timestamp = this
        .eth_api
        .get_block_timestamp(last_block)
        .await
        .unwrap_or(0);

    let process_message_admin_delay = this.eth_api.process_admin_message_delay().await?;
    let process_message_pauser_delay = this.eth_api.process_pauser_message_delay().await?;
    let process_message_user_delay = this.eth_api.process_user_message_delay().await?;

    let message_delay = |source: ActorId| {
        if source == this.governance_admin {
            process_message_admin_delay
        } else if source == this.governance_pauser {
            process_message_pauser_delay
        } else {
            process_message_user_delay
        }
    };

    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(12));
    poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = poll_interval.tick() => {
                let block = this.eth_api.safe_block_number().await?;
                if block <= last_block {
                    continue;
                }
                last_block = block;

                let timestamp = this.eth_api.get_block_timestamp(block).await?;
                last_timestamp = timestamp;
                log::info!("Found new Ethereum finalized block #{block} at {timestamp}");
                let merkle_roots = this.merkle_roots.read().await;
                for (merkle_root, message) in this.messages.drain_timestamp(last_timestamp, &message_delay, &merkle_roots) {
                    let Ok(_) = messages_out.send(Response::Success {
                        authority_set_id: message.authority_set_id,
                        block: message.block,
                        tx_uuid: message.tx_uuid,
                        merkle_root
                    }) else {
                        log::error!("Messages connection closed, exiting");
                        return Ok(());
                    };
                }
            }
            message = requests.recv() => {
                match message {
                    Some(message) => {
                        let merkle_roots = this.merkle_roots.read().await;
                        let delay = message_delay(message.source);
                        if let Some(merkle_root) =
                            merkle_roots.find(message.authority_set_id, message.block, last_timestamp, delay)
                        {
                            log::trace!("Found merkle root for the message '{message:?}' with delay = {delay:?}");
                            messages_out.send(Response::Success {
                                authority_set_id: message.authority_set_id,
                                block: message.block,
                                tx_uuid: message.tx_uuid,
                                merkle_root: *merkle_root,
                            })?;
                            continue;
                        } else {
                            log::trace!("No merkle root found for the message '{message:?}' with delay = {delay:?}");
                        }

                        if this.messages.add(message.clone()).is_none() {
                            log::error!(
                                "Unable to add the message '{message:?}' since the capacity is full"
                            );

                            messages_out.send(Response::Overflowed(message))?;
                        }
                    }
                    None => {
                        log::error!("Channel with messages closed. Exiting");
                        return Ok(());
                    }
                }
            }

            merkle_root = this.receiver_roots.recv() => {
                let Some(merkle_root) = merkle_root else {
                    log::info!("Channel with merkle roots closed. Exiting");
                    return Ok(());
                };

                let mut merkle_roots = this.merkle_roots.write().await;
                match merkle_roots.add(merkle_root) {
                    Ok(Added::Ok | Added::Overwritten(_)) => {}

                    Ok(Added::Removed(merkle_root_old)) => {
                        log::warn!("Removing merkle root = {merkle_root_old:?}");
                        let messages = this.messages.drain_all(&merkle_root_old);
                        for message in messages {
                            log::error!("Remove stuck message = {message:?}");
                            messages_out.send(Response::Stuck {
                                authority_set_id: message.authority_set_id,
                                block: message.block,
                                tx_uuid: message.tx_uuid,
                                merkle_root: merkle_root_old,
                            })?;
                        }
                    }

                    Err(_i) => {
                        // There is already a corresponding merkle root at the position.
                        continue;
                    }
                }

                log::trace!("Drain messages for merkle root = {merkle_root:?}, current timestamp = {last_timestamp}");
                for message in this.messages.drain(&merkle_root, last_timestamp, &message_delay) {
                    messages_out.send(Response::Success {
                        authority_set_id: message.authority_set_id,
                        block: message.block,
                        tx_uuid: message.tx_uuid,
                        merkle_root,
                    })?;
                }
            }
        }

        this.metrics.message_count.set(this.messages.len() as _);
    }
}
