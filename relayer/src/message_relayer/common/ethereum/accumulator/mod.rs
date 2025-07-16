mod utils;

use super::*;
use crate::{
    common::BASE_RETRY_DELAY,
    message_relayer::common::{AuthoritySetId, MessageInBlock},
};
use futures::{
    future::{self, Either},
    pin_mut,
};
use prometheus::IntGauge;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use utils::{AddStatus, MerkleRoots, Messages};
use utils_prometheus::{impl_metered_service, MeteredService};

/// Struct accumulates gear-eth messages and required merkle roots.
pub struct Accumulator {
    metrics: Metrics,
    messages: Messages,
    merkle_roots: MerkleRoots,
    receiver_roots: UnboundedReceiver<RelayedMerkleRoot>,
    receiver_messages: UnboundedReceiver<MessageInBlock>,
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
        receiver_messages: UnboundedReceiver<MessageInBlock>,
    ) -> Self {
        Self {
            metrics: Metrics::new(),
            messages: Messages::new(10_000),
            merkle_roots: MerkleRoots::new(300),
            receiver_roots,
            receiver_messages,
        }
    }

    pub fn spawn(mut self) -> UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)> {
        let (mut messages_out, receiver) = mpsc::unbounded_channel();
        tokio::task::spawn(async move {
            loop {
                match run_inner(&mut self, &mut messages_out).await {
                    Ok(_) => break,
                    Err(e) => {
                        log::error!("{e:?}");

                        tokio::time::sleep(BASE_RETRY_DELAY).await;
                    }
                }
            }
        });

        receiver
    }
}

async fn run_inner(
    this: &mut Accumulator,
    messages_out: &mut UnboundedSender<(MessageInBlock, RelayedMerkleRoot)>,
) -> anyhow::Result<()> {
    loop {
        let recv_messages = this.receiver_messages.recv();
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
                if let Some(merkle_root) = this
                    .merkle_roots
                    .find(message.authority_set_id, message.block)
                {
                    messages_out.send((message, *merkle_root))?;
                    continue;
                }

                if this.messages.add(message.clone()).is_none() {
                    log::error!(
                        "Unable to add the message '{message:?}' since the capacity is full"
                    );
                }
            }

            Either::Right((Some(merkle_root), _)) => {
                match this.merkle_roots.add(merkle_root) {
                    Ok(AddStatus::Ok | AddStatus::Overwritten(_)) => {}

                    Ok(AddStatus::Removed(merkle_root_old)) => {
                        log::warn!("Removing merkle root = {merkle_root_old:?}");
                        let messages = this.messages.drain(&merkle_root_old);
                        for message in messages {
                            log::error!("Remove stuck message = {message:?}");
                        }
                    }

                    Err(_i) => {
                        // log::warn!("There is already a merkle root: root_old = {:?}, merkle_root = {merkle_root:?}", this.merkle_roots.get(i));
                        continue;
                    }
                }

                for message in this.messages.drain(&merkle_root) {
                    messages_out.send((message, merkle_root))?;
                }
            }
        }

        this.metrics.message_count.set(this.messages.len() as _);
    }
}
