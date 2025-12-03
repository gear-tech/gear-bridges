use std::ops::ControlFlow;

use crate::{
    common,
    message_relayer::{
        common::{EthereumSlotNumber, TxHashWithSlot},
        eth_to_gear::api_provider::{ApiProviderConnection, GearApiActor, GetCheckpointSlot},
    },
};
use actix::{Actor, Addr, Context, Handler, Message, Recipient, ResponseFuture};
use alloy::providers::Provider;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_rlp::Encodable;
use anyhow::Context as _;
use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{PollingEthApi, TxHash};
use ethereum_common::{beacon, tree_hash::TreeHash, utils as eth_utils, utils::MerkleProof};
use futures::executor::block_on;

use primitive_types::H256;
use prometheus::IntGauge;
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::spawn_blocking,
};
use utils_prometheus::{impl_metered_service, MeteredService};
use uuid::Uuid;

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
    responses_channel: UnboundedReceiver<Response>,
}

impl ProofComposerIo {
    pub fn new(
        requests_channel: UnboundedSender<Request>,
        responses_channel: UnboundedReceiver<Response>,
    ) -> Self {
        Self {
            requests_channel,
            responses_channel,
        }
    }

    /// Receive composed proof for some transaction.
    ///
    /// In case of `None` indicates closed channel.
    pub async fn recv(&mut self) -> Option<Response> {
        self.responses_channel.recv().await
    }

    /// Send request to compose proof for `tx` with uuid `tx_uuid`.
    ///
    /// Returns `false` if send failed which indicates that channel was closed.
    pub fn compose_proof_for(&mut self, tx_uuid: Uuid, tx: TxHashWithSlot) -> bool {
        self.requests_channel
            .send(Request { tx, tx_uuid })
            .inspect_err(|err| {
                log::error!("proof composer send failure: {err:?}");
            })
            .is_ok()
    }
}

impl_metered_service!(
    struct Metrics {
        messages_waiting_for_checkpoint: IntGauge = IntGauge::new(
            "proof_composer_messages_waiting_for_checkpoint",
            "Number of messages waiting for checkpoint"
        ),
        last_checkpoint: IntGauge = IntGauge::new(
            "proof_composer_last_checkpoint",
            "Last checkpoint slot number"
        )
    }
);

pub struct ProofComposer {
    pub api_provider: ApiProviderConnection,
    pub beacon_client: BeaconClient,
    pub eth_api: PollingEthApi,
    pub waiting_for_checkpoints: Vec<(Uuid, TxHashWithSlot)>,
    pub last_checkpoint: Option<EthereumSlotNumber>,
    pub historical_proxy_address: H256,
    pub suri: String,
    pub to_process: Vec<(Uuid, TxHashWithSlot)>,
    pub gear_actor: Addr<GearApiActor>,

    metrics: Metrics,
}

impl MeteredService for ProofComposer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl ProofComposer {
    pub fn new(
        api_provider: ApiProviderConnection,
        beacon_client: BeaconClient,
        eth_api: PollingEthApi,
        historical_proxy_address: H256,
        suri: String,
        gear_actor: Addr<GearApiActor>,
    ) -> Self {
        Self {
            api_provider,
            beacon_client,
            eth_api,
            waiting_for_checkpoints: Vec::new(),
            last_checkpoint: None,
            historical_proxy_address,
            suri,
            to_process: Vec::with_capacity(100),
            gear_actor,

            metrics: Metrics::new(),
        }
    }

    pub fn run(self, checkpoints: UnboundedReceiver<EthereumSlotNumber>) -> ProofComposerIo {
        let (requests_tx, requests_rx) = unbounded_channel();
        let (response_tx, response_rx) = unbounded_channel();

        spawn_blocking(move || {
            block_on(task(self, checkpoints, requests_rx, response_tx));
        });

        ProofComposerIo::new(requests_tx, response_rx)
    }

    async fn process(
        &mut self,
        response_tx: &UnboundedSender<Response>,
        tx: TxHashWithSlot,
        tx_uuid: Uuid,
    ) -> anyhow::Result<()> {
        match compose(
            &self.beacon_client,
            &self.eth_api,
            tx.tx_hash,
            &self.gear_actor,
            self.historical_proxy_address,
            &self.suri,
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
                Err(err)
            }
        }
    }
}

async fn task(
    mut this: ProofComposer,
    mut checkpoints: UnboundedReceiver<EthereumSlotNumber>,
    mut requests: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
) {
    loop {
        if let Err(err) =
            handle_requests(&mut this, &mut checkpoints, &mut requests, &responses).await
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
        } else {
            return;
        }
    }
}

async fn handle_requests(
    this: &mut ProofComposer,
    checkpoints: &mut UnboundedReceiver<EthereumSlotNumber>,
    requests: &mut UnboundedReceiver<Request>,
    responses: &UnboundedSender<Response>,
) -> anyhow::Result<()> {
    loop {
        this.metrics
            .messages_waiting_for_checkpoint
            .set(this.waiting_for_checkpoints.len() as i64);

        while let Some((tx_uuid, tx)) = this.to_process.pop() {
            log::debug!("Processing transaction #{tx_uuid} (hash: {:?})", tx.tx_hash);
            match this.process(responses, tx.clone(), tx_uuid).await {
                Ok(_) => {}
                Err(err) => {
                    log::error!(
                        "Failed to process transaction {tx_uuid} (hash: {:?}): {err:?}",
                        tx.tx_hash
                    );
                }
            }
        }

        tokio::select! {
            value = checkpoints.recv() => {
                if let Some(checkpoint) = value {
                    log::info!("Received checkpoint: {checkpoint}");
                    this.last_checkpoint = Some(checkpoint);

                    this.metrics.last_checkpoint.set(checkpoint.0 as i64);

                    this.waiting_for_checkpoints.retain(|(tx_uuid, tx)| {
                        if tx.slot_number <= checkpoint {
                            this.to_process.push((*tx_uuid, tx.clone()));
                            false
                        } else {
                            true
                        }
                    });

                    continue;
                } else {
                    log::info!("Checkpoints channel closed, exiting...");
                    return Ok(());
                }
            }

            value = requests.recv() => {
                if let Some(Request { tx_uuid, tx }) = value {
                    if this.last_checkpoint.filter(|&last_checkpoint| tx.slot_number <= last_checkpoint)
                        .is_some()
                    {
                        this.to_process.push((tx_uuid, tx.clone()));
                    } else {
                        log::debug!("Transaction {tx_uuid} is waiting for checkpoint, adding to queue");
                        this.waiting_for_checkpoints.push((tx_uuid, tx));
                    }
                } else {
                    log::info!("Requests channel connection closed, exiting...");
                    return Ok(());
                }
            }
        }
    }
}

pub async fn compose(
    beacon_client: &BeaconClient,

    eth_client: &PollingEthApi,
    tx_hash: TxHash,

    gear_actor: &Addr<GearApiActor>,

    historical_proxy: H256,
    suri: &String,
) -> anyhow::Result<EthToVaraEvent> {
    let receipt = eth_client
        .get_transaction_receipt(tx_hash)
        .await?
        .ok_or(anyhow::anyhow!("Transaction receipt is missing"))?;

    let block = match receipt.block_hash {
        Some(hash) => eth_client
            .get_block_by_hash(hash)
            .await?
            .ok_or(anyhow::anyhow!("Ethereum block (hash) is missing"))?,
        None => match receipt.block_number {
            Some(number) => eth_client
                .get_block_by_number(BlockNumberOrTag::Number(number))
                .await?
                .ok_or(anyhow::anyhow!("Ethereum block (number) is missing"))?,
            None => return Err(anyhow::anyhow!("Unable to get Ethereum block")),
        },
    };

    let beacon_root_parent = block
        .header
        .parent_beacon_block_root
        .ok_or(anyhow::anyhow!(
            "Unable to determine root of parent beacon block"
        ))?;
    let block_number = block.header.number;

    let proof_block = build_inclusion_proof(
        beacon_client,
        &beacon_root_parent,
        block_number,
        gear_actor,
        historical_proxy,
        suri,
    )
    .await?;

    // receipt Merkle-proof
    let tx_index = receipt
        .transaction_index
        .ok_or(anyhow::anyhow!("Unable to determine transaction index"))?;
    let receipts = eth_client
        .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(block_number)))
        .await?
        .unwrap_or_default()
        .iter()
        .map(|tx_receipt| {
            let receipt = tx_receipt.as_ref();

            tx_receipt
                .transaction_index
                .map(|i| (i, eth_utils::map_receipt_envelope(receipt)))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();

    let MerkleProof { proof, receipt } = eth_utils::generate_merkle_proof(tx_index, &receipts[..])?;

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
    Encodable::encode(&receipt, &mut receipt_rlp);

    Ok(EthToVaraEvent {
        proof_block,
        proof,
        transaction_index: tx_index,
        receipt_rlp,
    })
}

async fn build_inclusion_proof(
    beacon_client: &BeaconClient,

    beacon_root_parent: &[u8; 32],
    block_number: u64,

    gear_actor: &Addr<GearApiActor>,
    historical_proxy_id: H256,
    suri: &String,
) -> anyhow::Result<BlockInclusionProof> {
    let beacon_block_parent = beacon_client
        .get_block_by_hash::<beacon::electra::Block>(beacon_root_parent)
        .await?;

    let beacon_block = beacon_client
        .find_beacon_block(block_number, beacon_block_parent)
        .await?;
    let beacon_block = beacon_client
        .get_block::<beacon::electra::Block>(beacon_block.slot)
        .await?;

    let slot = beacon_block.slot;

    let (checkpoint_slot, checkpoint) = gear_actor
        .send(GetCheckpointSlot {
            slot,
            suri: suri.clone(),
            historical_proxy_id: historical_proxy_id.into(),
        })
        .await??;

    let block = BlockGenericForBlockBody {
        slot,
        proposer_index: beacon_block.proposer_index,
        parent_root: beacon_block.parent_root,
        state_root: beacon_block.state_root,
        body: beacon_block.body.into(),
    };
    if slot == checkpoint_slot {
        return Ok(BlockInclusionProof {
            block,
            headers: vec![],
        });
    }

    let headers = {
        let mut headers = beacon_client
            .request_headers(slot + 1, checkpoint_slot + 1)
            .await?;

        headers.sort_unstable_by(|a, b| a.slot.cmp(&b.slot));

        headers
    };
    let ControlFlow::Continue(_) =
        headers
            .iter()
            .rev()
            .try_fold(checkpoint, |block_root_parent, header| {
                let block_root = header.tree_hash_root();
                match block_root == block_root_parent {
                    true => ControlFlow::Continue(header.parent_root),
                    false => ControlFlow::Break(()),
                }
            })
    else {
        return Err(anyhow::anyhow!("Invalid block proof"));
    };

    Ok(BlockInclusionProof { block, headers })
}

pub struct ProofComposerActor {
    pub beacon_client: BeaconClient,
    pub eth_api: PollingEthApi,
    pub waiting_for_checkpoints: Vec<(TxHashWithSlot, Recipient<ComposedProof>)>,
    pub last_checkpoint: Option<EthereumSlotNumber>,
    pub historical_proxy_address: H256,
    pub suri: String,
    pub to_process: Vec<(TxHashWithSlot, Recipient<ComposedProof>)>,
    pub gear_actor: Addr<GearApiActor>,
    composer: Addr<Composer>,
    metrics: Metrics,
}

impl ProofComposerActor {
    pub fn new(
        beacon_client: BeaconClient,
        eth_api: PollingEthApi,
        historical_proxy_address: H256,
        suri: String,
        gear_actor: Addr<GearApiActor>,
    ) -> Self {
        let composer = Composer::new(
            gear_actor.clone(),
            beacon_client.clone(),
            eth_api.clone(),
            historical_proxy_address,
            suri.clone(),
        )
        .start();

        Self {
            beacon_client,
            eth_api,
            waiting_for_checkpoints: Vec::new(),
            last_checkpoint: None,
            historical_proxy_address,
            suri,
            to_process: Vec::with_capacity(100),
            gear_actor,
            composer,

            metrics: Metrics::new(),
        }
    }
}

impl Actor for ProofComposerActor {
    type Context = Context<Self>;
}

/// An actual actor that composes proofs.
struct Composer {
    gear_actor: Addr<GearApiActor>,
    beacon_client: BeaconClient,
    eth_api: PollingEthApi,
    historical_proxy_address: H256,
    suri: String,
}

impl Composer {
    pub fn new(
        gear_actor: Addr<GearApiActor>,
        beacon_client: BeaconClient,
        eth_api: PollingEthApi,
        historical_proxy_address: H256,
        suri: String,
    ) -> Self {
        Self {
            gear_actor,
            beacon_client,
            eth_api,
            historical_proxy_address,
            suri,
        }
    }
}

impl Actor for Composer {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]

pub enum ComposedProof {
    Success {
        tx_hash: TxHashWithSlot,
        payload: EthToVaraEvent,
    },
    Failure {
        tx_hash: TxHashWithSlot,
        error: anyhow::Error,
    },
}

/// Compose proof for the given transaction.
///
/// Instead of returning result directly, it sends it to the specified recipient.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ComposeProof {
    pub tx: TxHashWithSlot,
    pub recipient: Recipient<ComposedProof>,
}

/// New checkpoint discovered on Ethereum network.
#[derive(Message)]
#[rtype(result = "()")]
pub struct NewCheckpoint {
    pub slot: EthereumSlotNumber,
}

impl Handler<NewCheckpoint> for ProofComposerActor {
    type Result = ();

    /// Submit a new checkpoint to the actor.
    ///
    /// Processes all waiting transactions for which the checkpoint is sufficient.
    fn handle(&mut self, msg: NewCheckpoint, _ctx: &mut Self::Context) -> Self::Result {
        log::info!("Received new checkpoint: {}", msg.slot);

        self.last_checkpoint = Some(msg.slot);
        self.metrics.last_checkpoint.set(msg.slot.0 as i64);
        self.waiting_for_checkpoints
            .retain(|(tx_uuid, tx, recipient)| {
                if tx.slot_number <= msg.slot {
                    self.to_process
                        .push((*tx_uuid, tx.clone(), recipient.clone()));
                    false
                } else {
                    true
                }
            });

        // Send all ready transactions to composer. We do not wait for reply
        // but instead let composer handle them asynchronously and send replies to specified recipient.
        while let Some((tx_uuid, tx, recipient)) = self.to_process.pop() {
            self.composer.do_send(ComposeProof { tx, recipient });
        }
    }
}

impl Handler<ComposeProof> for ProofComposerActor {
    type Result = ();

    /// Handle request to compose proof for the given transaction.
    ///
    /// If checkpoint for the transaction slot is available, forwards the request to the [composer](Composer) actor
    /// immediately. Otherwise, adds the request to the waiting queue.
    fn handle(&mut self, msg: ComposeProof, _ctx: &mut Self::Context) -> Self::Result {
        if self
            .last_checkpoint
            .filter(|&last_checkpoint| msg.tx.slot_number <= last_checkpoint)
            .is_some()
        {
            self.composer.do_send(msg);
        } else {
            log::debug!(
                "Transaction {} is waiting for checkpoint, adding to queue",
                msg.tx.tx_hash
            );
            self.waiting_for_checkpoints.push((msg.tx, msg.recipient));
        }
    }
}

impl Handler<ComposeProof> for Composer {
    type Result = ResponseFuture<()>;

    /// Compose proof for the given transaction and send it to the recipient.
    ///
    /// Does not block the actor, instead returns a future which is executed asynchronously.
    fn handle(&mut self, msg: ComposeProof, _ctx: &mut Self::Context) -> Self::Result {
        let gear_actor = self.gear_actor.clone();
        let beacon_client = self.beacon_client.clone();
        let eth_api = self.eth_api.clone();
        let historical_proxy_address = self.historical_proxy_address;
        let suri = self.suri.clone();

        // Create a future that composes the proof and sends it to the recipient.
        // Does not block the actor and allows it to process other messages.
        Box::pin(async move {
            match compose(
                &beacon_client,
                &eth_api,
                msg.tx.tx_hash,
                &gear_actor,
                historical_proxy_address,
                &suri,
            )
            .await
            {
                Ok(payload) => {
                    let _ = msg
                        .recipient
                        .send(ComposedProof::Success {
                            payload,
                            tx_hash: msg.tx,
                        })
                        .await;
                }
                Err(err) => {
                    log::error!(
                        "Failed to compose proof for transaction {}: {:?}",
                        msg.tx.tx_hash,
                        err
                    );

                    let _ = msg
                        .recipient
                        .send(ComposedProof::Failure {
                            tx_hash: msg.tx,
                            error: err,
                        })
                        .await;
                }
            }
        })
    }
}
