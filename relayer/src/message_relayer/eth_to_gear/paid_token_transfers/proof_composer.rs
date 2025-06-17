use anyhow::Context;
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

#[derive(Clone)]
pub struct Request {
    pub tx: TxHashWithSlot,
    pub tx_uuid: Uuid,
}

#[derive(Clone)]
pub struct Response {
    pub payload: EthToVaraEvent,
    pub tx_uuid: Uuid,
}

pub struct ProofComposerIo {
    requests_channel: UnboundedSender<Request>,
    response_channel: UnboundedReceiver<Response>,
}

impl ProofComposerIo {
    /// Receive composed proof for some transaction.
    ///
    /// In case of `None` indicates closed channel.
    pub async fn recv(&mut self) -> Option<Response> {
        self.response_channel.recv().await
    }

    /// Send request to compose proof for `tx` with uuid `tx_uuid`.
    ///
    /// Returns `false` if send failed which indicates that channel was closed.
    pub fn compose_proof_for(&mut self, tx_uuid: Uuid, tx: TxHashWithSlot) -> bool {
        self.requests_channel.send(Request { tx, tx_uuid }).is_ok()
    }
}

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

    pub fn run(self, checkpoints: UnboundedReceiver<EthereumSlotNumber>) -> ProofComposerIo {
        let (requests_tx, requests_rx) = unbounded_channel();
        let (response_tx, response_rx) = unbounded_channel();

        spawn_blocking(move || {
            block_on(task(self, checkpoints, requests_rx, response_tx));
        });

        ProofComposerIo {
            requests_channel: requests_tx,
            response_channel: response_rx,
        }
    }

    async fn run_inner(
        &mut self,
        checkpoints: &mut UnboundedReceiver<EthereumSlotNumber>,
        requests_rx: &mut UnboundedReceiver<Request>,
        response_tx: &mut UnboundedSender<Response>,
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
                        self.process(response_tx, tx, tx_uuid).await?;
                    }
                }

                Some(Request { tx_uuid, tx }) = requests_rx.recv() => {
                    if self.last_checkpoint.filter(|&last_checkpoint| tx.slot_number <= last_checkpoint)
                        .is_some()
                    {
                        self.process(response_tx, tx, tx_uuid).await?;
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

    async fn process(
        &mut self,
        response_tx: &mut UnboundedSender<Response>,
        tx: TxHashWithSlot,
        tx_uuid: Uuid,
    ) -> anyhow::Result<()> {
        let gear_api = self.api_provider.gclient_client(&self.suri)?;

        match compose_payload::compose(
            &self.beacon_client,
            &gear_api,
            &self.eth_api,
            tx.tx_hash,
            self.historical_proxy_address.into(),
        )
        .await
        {
            Ok(payload) => response_tx
                .send(Response { payload, tx_uuid })
                .context("failed to send response"),
            Err(err) => {
                log::error!(
                    "Failed to compose proof for transaction {}: {:?}",
                    tx.tx_hash,
                    err
                );
                return Err(err);
            }
        }
    }
}

async fn task(
    mut this: ProofComposer,
    mut checkpoints: UnboundedReceiver<EthereumSlotNumber>,
    mut requests: UnboundedReceiver<Request>,
    mut responses: UnboundedSender<Response>,
) {
    loop {
        if let Err(err) = this
            .run_inner(&mut checkpoints, &mut requests, &mut responses)
            .await
        {
            log::error!("Proof composer failed with error: {err:?}");
            if common::is_transport_error_recoverable(&err) {
                match this.api_provider.reconnect().await {
                    Ok(_) => log::info!("Successfully reconnected to Gear API"),
                    Err(err) => {
                        log::error!("Failed to reconnect to Gear API: {err:?}");
                        return;
                    }
                }

                match this.eth_api.reconnect().await {
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
}
