use crate::{
    common,
    message_relayer::{
        common::{gear::message_sender::compose_payload, EthereumSlotNumber, TxHashWithSlot},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};
use eth_events_electra_client::EthToVaraEvent;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use futures::executor::block_on;

use primitive_types::H256;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

pub struct ProofComposerTask {
    pub api_provider: ApiProviderConnection,
    pub beacon_client: BeaconClient,
    pub eth_api: EthApi,
    pub waiting_checkpoint: Vec<(Uuid, TxHashWithSlot)>,
    pub last_checkpoint: Option<EthereumSlotNumber>,
    pub historical_proxy_address: H256,
    pub suri: String,
}

impl ProofComposerTask {
    pub fn new(
        api_provider: ApiProviderConnection,
        beacon_client: BeaconClient,
        eth_api: EthApi,

        historical_proxy_address: H256,
        suri: String,
    ) -> Self {
        Self {
            api_provider,
            beacon_client,
            eth_api,
            last_checkpoint: None,
            waiting_checkpoint: Vec::new(),
            historical_proxy_address,
            suri,
        }
    }

    pub fn run(
        mut self,
        checkpoints: UnboundedReceiver<EthereumSlotNumber>,
    ) -> (
        UnboundedSender<ProofComposerRequest>,
        UnboundedReceiver<ProofComposerResponse>,
    ) {
        let (requests_tx, mut requests_rx) = unbounded_channel();
        let (mut responses_tx, responses_rx) = unbounded_channel();
        let mut checkpoints = checkpoints;

        tokio::task::spawn_blocking(|| {
            block_on(async move {
                loop {
                    if let Err(e) = self
                        .run_inner(&mut checkpoints, &mut requests_rx, &mut responses_tx)
                        .await
                    {
                        log::error!("ProofComposerTask encountered an error: {e}");

                        if common::is_transport_error_recoverable(&e) {
                            match self.api_provider.reconnect().await {
                                Ok(_) => log::info!("Reconnected successfully"),
                                Err(err) => {
                                    log::error!("Failed to reconnect to Gear API: {err}");
                                    return;
                                }
                            }

                            match self.eth_api.reconnect().await {
                                Ok(_) => log::info!("Reconnected successfully to Ethereum API"),
                                Err(err) => {
                                    log::error!("Failed to reconnect to Ethereum API: {err}");
                                    return;
                                }
                            }
                        } else {
                            log::error!("Non-recoverable error encountered, stopping task: {e}");
                            return;
                        }
                    }
                }
            })
        });
        (requests_tx, responses_rx)
    }

    async fn run_inner(
        &mut self,
        checkpoints: &mut UnboundedReceiver<EthereumSlotNumber>,
        requests_rx: &mut UnboundedReceiver<ProofComposerRequest>,
        responses_tx: &mut UnboundedSender<ProofComposerResponse>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                Some(checkpoint) = checkpoints.recv() => {
                    log::info!("Received checkpoint: {checkpoint}");
                    self.last_checkpoint = Some(checkpoint);
                    let mut to_process = Vec::new();

                    self.waiting_checkpoint.retain(|(task_uuid, tx)| {
                        if tx.slot_number <= checkpoint {
                            to_process.push((*task_uuid, tx.clone()));
                            false
                        } else {
                            true
                        }
                    });

                    for (task_uuid, tx) in to_process {
                        log::info!("Processing waiting transaction for task {task_uuid}: {tx:?}");
                        match self.compose_payload(tx).await {
                            Ok(payload) => {
                                responses_tx.send(ProofComposerResponse { payload, task_uuid }).unwrap();
                            }
                            Err(e) => {
                                log::error!("Failed to compose payload: {e}");
                                return Err(e);
                            }
                        }
                    }
                }

                Some(req) = requests_rx.recv() => {
                    let ProofComposerRequest { tx, task_uuid } = req;
                    if let Some(last_checkpoint) = self.last_checkpoint {
                        log::info!("Received request for task {task_uuid}: {tx:?}");
                        if tx.slot_number <= last_checkpoint {
                            match self.compose_payload(tx).await {
                                Ok(payload) => {
                                    responses_tx.send(ProofComposerResponse { payload, task_uuid }).unwrap();
                                }
                                Err(e) => {
                                    log::error!("Failed to compose payload: {e}");
                                    return Err(e);
                                }
                            }
                        } else {
                            log::info!("Task {task_uuid} is waiting for checkpoint, adding to queue.");
                            self.waiting_checkpoint.push((task_uuid, tx));
                        }
                    } else {
                        self.waiting_checkpoint.push((task_uuid, tx));
                    }
                }
            }
        }
    }

    async fn compose_payload(&mut self, tx: TxHashWithSlot) -> anyhow::Result<EthToVaraEvent> {
        let gear_api = self.api_provider.gclient_client(&self.suri)?;

        compose_payload::compose(
            &self.beacon_client,
            &gear_api,
            &self.eth_api,
            tx.tx_hash,
            self.historical_proxy_address.into(),
        )
        .await
    }
}

pub struct ProofComposerRequest {
    pub tx: TxHashWithSlot,
    pub task_uuid: Uuid,
}

pub struct ProofComposerResponse {
    pub payload: EthToVaraEvent,
    pub task_uuid: Uuid,
}
