use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;

use ethereum_common::Hash256;
use futures::StreamExt;
use gsdk::{
    config::Header,
    metadata::{
        gear::Event as GearEvent,
        runtime_types::{gear_core::message::user::UserMessage, gprimitives::ActorId},
    },
    subscription::BlockEvents,
};
use primitive_types::H256;
use prometheus::IntGauge;
use subxt::config::Header as _;
use tokio::sync::broadcast;
use utils_prometheus::{impl_metered_service, MeteredService};

#[derive(Clone)]
pub struct GearBlock {
    pub header: Header,
    pub events: Vec<gsdk::Event>,
}

impl GearBlock {
    pub fn new(header: Header, events: Vec<gsdk::Event>) -> Self {
        Self { header, events }
    }

    pub fn number(&self) -> u32 {
        self.header.number()
    }

    pub fn hash(&self) -> Hash256 {
        self.header.hash()
    }

    pub fn events(&self) -> &[gsdk::Event] {
        &self.events
    }

    pub fn user_message_sent_events(
        &self,
        from_program: H256,
        to_user: H256,
    ) -> impl Iterator<Item = &[u8]> + use<'_> {
        self.events.iter().filter_map(move |event| match event {
            gclient::Event::Gear(GearEvent::UserMessageSent {
                message:
                    UserMessage {
                        source,
                        destination,
                        payload,
                        ..
                    },
                ..
            }) if source == &ActorId(from_program.0) && destination == &ActorId(to_user.0) => {
                Some(payload.0.as_ref())
            }
            _ => None,
        })
    }
}

pub struct BlockListener {
    api_provider: ApiProviderConnection,

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
    pub fn new(api_provider: ApiProviderConnection) -> Self {
        Self {
            api_provider,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<GearBlock>; RECEIVER_COUNT] {
        let (tx, _) = broadcast::channel(RECEIVER_COUNT);
        let tx2 = tx.clone();
        tokio::task::spawn(async move {
            loop {
                let res = self.run_inner(&tx2).await;
                match res {
                    Ok(false) => {
                        log::info!("Gear block listener stopped due to no active receivers");
                        return;
                    }

                    Ok(true) => {
                        log::info!("Gear block listener: subscription expired, restarting");
                        continue;
                    }

                    Err(err) => {
                        log::error!("Gear block listener failed: {err}");

                        match self.api_provider.reconnect().await {
                            Ok(()) => {
                                log::info!("Gear block listener reconnected");
                            }
                            Err(err) => {
                                log::error!("Gear block listener unable to reconnect: {err}");
                                return;
                            }
                        };
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

    async fn run_inner(&self, tx: &broadcast::Sender<GearBlock>) -> anyhow::Result<bool> {
        let gear_api = self.api_provider.client();

        let mut finalized_blocks = gear_api.api.subscribe_finalized_blocks().await?;
        loop {
            match finalized_blocks.next().await {
                Some(Err(err)) => {
                    log::error!("Error receiving finalized block: {err}");
                    break Err(err);
                }

                Some(Ok(block)) => {
                    self.metrics.latest_block.set(block.number() as i64);
                    let header = block.header().clone();
                    let block_events = BlockEvents::new(block).await?;
                    let events = block_events.events()?;

                    match tx.send(GearBlock::new(header, events)) {
                        Ok(_) => (),
                        Err(broadcast::error::SendError(_)) => {
                            log::error!("No active receivers for Gear block listener, stopping");
                            return Ok(false);
                        }
                    }
                    self.metrics.latest_block.inc();
                }

                None => break Ok(true),
            }
        }
    }
}
