use eth_events_electra_client::EthToVaraEvent;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use futures::executor::block_on;
use primitive_types::H256;
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::spawn_blocking,
};
use uuid::Uuid;

use crate::{
    common,
    message_relayer::{
        common::{gear::message_sender::compose_payload, EthereumSlotNumber, TxHashWithSlot},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};

pub struct ProofComposer {
    pub api_provider: ApiProviderConnection,
    pub beacon_client: BeaconClient,
    pub eth_api: EthApi,
    pub waiting_for_checkpoints: Vec<(Uuid, TxHashWithSlot)>,
    pub last_checkpoint: Option<EthereumSlotNumber>,
    pub historical_proxy_address: H256,
    pub suri: String,
}

impl ProofComposer {
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
            waiting_for_checkpoints: Vec::new(),
            last_checkpoint: None,
            historical_proxy_address,
            suri,
        }
    }

    pub fn run(
        mut self,
        mut checkpoints: UnboundedReceiver<EthereumSlotNumber>,
    ) -> ProofComposerIo {
        let (requests_tx, mut requests_rx) = unbounded_channel();
        let (mut response_tx, response_rx) = unbounded_channel();

        spawn_blocking(move || {
            block_on(async move {
                loop {
                    if let Err(err) = self
                        .run_inner(&mut checkpoints, &mut requests_rx, &mut response_tx)
                        .await
                    {
                        log::error!("Proof composer failed with error: {err:?}");
                        if common::is_transport_error_recoverable(&err) {
                            match self.api_provider.reconnect().await {
                                Ok(_) => log::info!("Successfully reconnected to Gear API"),
                                Err(err) => {
                                    log::error!("Failed to reconnect to Gear API: {err:?}");
                                    return;
                                }
                            }

                            match self.eth_api.reconnect().await {
                                Ok(_) => log::info!("Successfully reconnected to Ethereum API"),
                                Err(err) => {
                                    log::error!("Failed to reconnect to Ethereum API: {err:?}");
                                    return;
                                }
                            }
                        } else {
                            log::error!("Non recoverable error, exiting: {err:?}");
                            return;
                        }
                    }
                }
            })
        });

        ProofComposerIo {
            requests_channel: requests_tx,
            response_channel: response_rx,
        }
    }

    async fn run_inner(
        &mut self,
        checkpoints: &mut UnboundedReceiver<EthereumSlotNumber>,
        requests_rx: &mut UnboundedReceiver<ComposeProof>,
        response_tx: &mut UnboundedSender<ComposedProof>,
    ) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                Some(checkpoint) = checkpoints.recv() => {
                    log::info!("Received checkpoint: {checkpoint}");
                    self.last_checkpoint = Some(checkpoint);

                    let mut to_process = Vec::new();

                    self.waiting_for_checkpoints.retain(|(tx_uuid, tx)| {
                        if tx.slot_number <= checkpoint {
                            to_process.push((*tx_uuid, tx.clone()));
                            false
                        } else {
                            true
                        }
                    });

                    for (tx_uuid, tx) in to_process {
                        log::info!("Processing waiting transaction {tx_uuid}: {tx:?}");

                        match self.compose_payload(tx).await {
                            Ok(payload) => {
                                response_tx.send(ComposedProof {
                                    payload,
                                    tx_uuid
                                })?;
                            }

                            Err(e) => {
                                log::error!("Failed to compose payload {e:?}");
                                return Err(e);
                            }
                        }
                    }
                }

                Some(ComposeProof { tx_uuid, tx }) = requests_rx.recv() => {
                    if self.last_checkpoint.filter(|&last_checkpoint| tx.slot_number <= last_checkpoint)
                        .is_some() {
                        match self.compose_payload(tx).await {
                            Ok(payload) => {
                                response_tx.send(ComposedProof {
                                    payload,
                                    tx_uuid
                                })?;
                            }
                            Err(e) => {
                                log::error!("Failed to compose payload for {tx_uuid}: {e}");
                                return Err(e);
                            }
                        }
                    } else {
                        log::info!("Transaction {tx_uuid} is waiting for checkpoint, adding to queue");
                        self.waiting_for_checkpoints.push((tx_uuid, tx));
                    }
                }

                else => {
                    log::info!("Channels closed, exiting...");
                    return Ok(());
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

pub struct ProofComposerIo {
    requests_channel: UnboundedSender<ComposeProof>,
    response_channel: UnboundedReceiver<ComposedProof>,
}

impl ProofComposerIo {
    /// Receive composed proof for some transaction.
    ///
    /// In case of `None` indicates closed channel.
    pub async fn recv(&mut self) -> Option<ComposedProof> {
        self.response_channel.recv().await
    }

    /// Send request to compose proof for `tx` with uuid `tx_uuid`.
    ///
    /// Returns `false` if send failed which indicates that channel was closed.
    pub fn compose_proof_for(&mut self, tx_uuid: Uuid, tx: TxHashWithSlot) -> bool {
        self.requests_channel
            .send(ComposeProof { tx, tx_uuid })
            .is_ok()
    }
}

#[derive(Clone)]
pub struct ComposeProof {
    pub tx: TxHashWithSlot,
    pub tx_uuid: Uuid,
}

#[derive(Clone)]
pub struct ComposedProof {
    pub payload: EthToVaraEvent,
    pub tx_uuid: Uuid,
}
