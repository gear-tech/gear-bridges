use crate::message_relayer::eth_to_gear::api_provider::ApiProviderConnection;

use checkpoint_light_client_client::service_replay_back::events::ServiceReplayBackEvents;
use checkpoint_light_client_client::service_sync_update::events::ServiceSyncUpdateEvents;
use ethereum_common::Hash256;
use futures::StreamExt;
use gsdk::metadata::gear::Event as GearEvent;
use gsdk::metadata::gear_eth_bridge::Event as GearEthBridgeEvent;
use gsdk::metadata::runtime_types::gear_core::message::user::UserMessage;
use gsdk::metadata::runtime_types::gprimitives::ActorId;
use gsdk::{config::Header, subscription::BlockEvents};
use primitive_types::H256;
use prometheus::IntGauge;
use sails_rs::events::EventIo;
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

    pub fn message_queued_events(
        &self,
    ) -> impl Iterator<Item = gear_rpc_client::dto::Message> + use<'_> {
        self.events.iter().filter_map(|event| match event {
            gclient::Event::GearEthBridge(GearEthBridgeEvent::MessageQueued {
                message, ..
            }) => {
                let mut nonce_le = [0; 32];
                primitive_types::U256(message.nonce.0).to_little_endian(&mut nonce_le);

                Some(gear_rpc_client::dto::Message {
                    nonce_le,
                    source: message.source.0,
                    destination: message.destination.0,
                    payload: message.payload.clone(),
                })
            }
            _ => None,
        })
    }

    pub fn user_message_sent_events(
        &self,
        from_program: H256,
        to_user: H256,
    ) -> impl Iterator<Item = gear_rpc_client::dto::UserMessageSent> + use<'_> {
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
                Some(gear_rpc_client::dto::UserMessageSent {
                    payload: payload.0.clone(),
                })
            }
            _ => None,
        })
    }

    pub fn service_events<E: EventIo>(
        &self,
        from_program: H256,
    ) -> impl Iterator<Item = E::Event> + use<'_, E> {
        self.events.iter().filter_map(move |event| match event {
            gclient::Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.source == ActorId(from_program.0)
                    && message.destination.0 == [0; 32] =>
            {
                E::decode_event(&message.payload.0).ok()
            }
            _ => None,
        })
    }

    /// Get all checkpoints added in this block.
    pub fn new_checkpoints(
        &self,
        from_program: H256,
    ) -> impl Iterator<Item = (u64, H256)> + use<'_> {
        self.service_events::<ServiceReplayBackEvents>(from_program)
            .map(
                |ServiceReplayBackEvents::NewCheckpoint {
                     slot,
                     tree_hash_root,
                 }| (slot, tree_hash_root),
            )
            .chain(
                self.service_events::<ServiceSyncUpdateEvents>(from_program)
                    .map(
                        |ServiceSyncUpdateEvents::NewCheckpoint {
                             slot,
                             tree_hash_root,
                         }| (slot, tree_hash_root),
                    ),
            )
    }
}

pub struct BlockListener {
    api_provider: ApiProviderConnection,
    from_block: u32,

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
    pub fn new(api_provider: ApiProviderConnection, from_block: u32) -> Self {
        Self {
            api_provider,
            from_block,

            metrics: Metrics::new(),
        }
    }

    pub async fn run<const RECEIVER_COUNT: usize>(
        mut self,
    ) -> [broadcast::Receiver<GearBlock>; RECEIVER_COUNT] {
        let (tx, _) = broadcast::channel(RECEIVER_COUNT);
        let tx2 = tx.clone();
        tokio::task::spawn(async move {
            let mut current_block = self.from_block;
            loop {
                let res = self.run_inner(&tx2, &mut current_block).await;
                if let Err(err) = res {
                    log::error!("Gear block listener failed: {}", err);

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
        });

        (0..RECEIVER_COUNT)
            .map(|_| tx.subscribe())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    async fn run_inner(
        &self,
        tx: &broadcast::Sender<GearBlock>,
        current_block: &mut u32,
    ) -> anyhow::Result<()> {
        self.metrics.latest_block.set(*current_block as i64);
        let gear_api = self.api_provider.client();

        let mut finalized_blocks = gear_api.api.subscribe_finalized_blocks().await?;
        loop {
            match finalized_blocks.next().await {
                Some(Err(err)) => {
                    log::error!("Error receiving finalized block: {}", err);
                    break Err(err);
                }

                Some(Ok(block)) => {
                    *current_block = block.number() + 1;
                    let header = block.header().clone();
                    let block_events = BlockEvents::new(block).await?;
                    let events = block_events.events()?;

                    match tx.send(GearBlock::new(header, events)) {
                        Ok(_) => (),
                        Err(broadcast::error::SendError(_)) => {
                            log::error!("No active receivers for Gear block listener, stopping");
                            return Ok(());
                        }
                    }
                    self.metrics.latest_block.inc();
                }

                None => break Ok(()),
            }
        }
    }
}
