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
use utils::{MerkleRoots, Messages};
use utils_prometheus::{impl_metered_service, MeteredService};

/// Struct accumulates gear-eth messages and required merkle roots.
pub struct Accumulator {
    metrics: Metrics,
    messages: Messages,
    merkle_roots: MerkleRoots,
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
    pub fn new() -> Self {
        Self {
            metrics: Metrics::new(),
            messages: Messages::new(10_000),
            merkle_roots: MerkleRoots::new(100),
        }
    }

    pub async fn run(
        mut self,
        mut messages: UnboundedReceiver<MessageInBlock>,
        mut merkle_roots: UnboundedReceiver<RelayedMerkleRoot>,
    ) -> UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)> {
        let (mut messages_out, receiver) = mpsc::unbounded_channel();
        tokio::task::spawn(async move {
            loop {
                match run_inner(
                    &mut self,
                    &mut messages,
                    &mut merkle_roots,
                    &mut messages_out,
                )
                .await
                {
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
    self_: &mut Accumulator,
    messages: &mut UnboundedReceiver<MessageInBlock>,
    merkle_roots: &mut UnboundedReceiver<RelayedMerkleRoot>,
    messages_out: &mut UnboundedSender<(MessageInBlock, RelayedMerkleRoot)>,
) -> anyhow::Result<()> {
    loop {
        let recv_messages = messages.recv();
        pin_mut!(recv_messages);

        let recv_merkle_roots = merkle_roots.recv();
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
                if let Some(merkle_root) = self_
                    .merkle_roots
                    .find(message.authority_set_id, message.block)
                {
                    messages_out.send((message, *merkle_root))?;
                    continue;
                }

                if self_.messages.add(message.clone()).is_none() {
                    log::error!(
                        "Unable to add the message '{message:?}' since the capacity is full"
                    );
                }
            }

            Either::Right((Some(merkle_root), _)) => {
                match self_.merkle_roots.add(merkle_root) {
                    Ok(None) => {}

                    Ok(Some(merkle_root_old)) => {
                        log::warn!("Removing merkle root = {merkle_root_old:?}");
                        let messages = self_.messages.drain(&merkle_root_old);
                        for message in messages {
                            log::error!("Remove stuck message = {message:?}");
                        }
                    }

                    Err(i) => {
                        log::warn!("There is already a merkle root: root_old = {:?}, merkle_root = {merkle_root:?}", self_.merkle_roots.get(i));

                        continue;
                    }
                }

                for message in self_.messages.drain(&merkle_root) {
                    messages_out.send((message, merkle_root))?;
                }
            }
        }

        self_.metrics.message_count.set(self_.messages.len() as _);
    }
}
