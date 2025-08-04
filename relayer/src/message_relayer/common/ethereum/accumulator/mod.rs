pub mod utils;

use super::*;
use crate::{common::BASE_RETRY_DELAY, message_relayer::common::AuthoritySetId};
use futures::{
    future::{self, Either},
    pin_mut,
};
use primitive_types::H256;
use prometheus::IntGauge;
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
    ) -> bool {
        let request = Request {
            authority_set_id,
            block,
            block_hash,
            tx_uuid,
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
    ) -> Self {
        Self {
            metrics: Metrics::new(),
            messages: Messages::new(10_000),
            merkle_roots,
            receiver_roots,
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
    loop {
        let recv_messages = requests.recv();
        pin_mut!(recv_messages);

        let recv_merkle_roots = this.receiver_roots.recv();
        pin_mut!(recv_merkle_roots);

        match future::select(recv_messages, recv_merkle_roots).await {
            Either::Left((None, _)) => {
                log::info!("Channel with messages closed. Exiting");
                return Ok(());
            }

            Either::Right((None, _)) => {
                log::info!("Channel with merkle roots closed. Exiting");
                return Ok(());
            }

            Either::Left((Some(message), _)) => {
                let merkle_roots = this.merkle_roots.read().await;
                if let Some(merkle_root) =
                    merkle_roots.find(message.authority_set_id, message.block)
                {
                    messages_out.send(Response::Success {
                        authority_set_id: message.authority_set_id,
                        block: message.block,
                        tx_uuid: message.tx_uuid,
                        merkle_root: merkle_root.clone(),
                    })?;
                    continue;
                }

                if this.messages.add(message.clone()).is_none() {
                    log::error!(
                        "Unable to add the message '{message:?}' since the capacity is full"
                    );

                    messages_out.send(Response::Overflowed(message))?;
                }
            }

            Either::Right((Some(merkle_root), _)) => {
                let mut merkle_roots = this.merkle_roots.write().await;
                match merkle_roots.add(merkle_root) {
                    Ok(Added::Ok | Added::Overwritten(_)) => {}

                    Ok(Added::Removed(merkle_root_old)) => {
                        log::warn!("Removing merkle root = {merkle_root_old:?}");
                        let messages = this.messages.drain(&merkle_root_old);
                        for message in messages {
                            log::error!("Remove stuck message = {message:?}");
                            messages_out.send(Response::Stuck {
                                authority_set_id: message.authority_set_id,
                                block: message.block,
                                tx_uuid: message.tx_uuid,
                                merkle_root: merkle_root_old.clone(),
                            })?;
                        }
                    }

                    Err(_i) => {
                        // There is already a corresponding merkle root at the position.
                        continue;
                    }
                }

                for message in this.messages.drain(&merkle_root) {
                    messages_out.send(Response::Success {
                        authority_set_id: message.authority_set_id,
                        block: message.block,
                        tx_uuid: message.tx_uuid,
                        merkle_root: merkle_root.clone(),
                    })?;
                }
            }
        }

        this.metrics.message_count.set(this.messages.len() as _);
    }
}
