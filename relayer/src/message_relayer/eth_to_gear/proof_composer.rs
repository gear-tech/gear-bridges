use std::ops::ControlFlow;

use crate::{
    common,
    message_relayer::{
        common::{EthereumSlotNumber, TxHashWithSlot},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};
use alloy::providers::Provider;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_rlp::Encodable;
use anyhow::Context;
use checkpoint_light_client_client::traits::ServiceCheckpointFor as _;
use checkpoint_light_client_client::ServiceCheckpointFor;
use eth_events_electra_client::{
    traits::EthereumEventClient, BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, TxHash};
use ethereum_common::utils as eth_utils;
use ethereum_common::{beacon, tree_hash::TreeHash, utils::MerkleProof};
use futures::executor::block_on;
use historical_proxy_client::traits::HistoricalProxy as _;
use historical_proxy_client::HistoricalProxy;
use primitive_types::H256;
use sails_rs::{calls::Query, gclient::calls::GClientRemoting, ActorId};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::spawn_blocking,
};
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

pub struct ProofComposer {
    pub api_provider: ApiProviderConnection,
    pub beacon_client: BeaconClient,
    pub eth_api: EthApi,
    pub waiting_for_checkpoints: Vec<(Uuid, TxHashWithSlot)>,
    pub last_checkpoint: Option<EthereumSlotNumber>,
    pub historical_proxy_address: H256,
    pub suri: String,
    pub to_process: Vec<(Uuid, TxHashWithSlot)>,
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
            to_process: Vec::with_capacity(100),
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
        let gear_api = self.api_provider.gclient_client(&self.suri)?;

        match compose(
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
        while !this.to_process.is_empty() {
            // safe to use `last` and `pop` since we check that `to_process` is not empty
            let (tx_uuid, tx) = this
                .to_process
                .last()
                .expect("to_process should not be empty");
            log::debug!("Processing transaction #{tx_uuid} (hash: {:?})", tx.tx_hash);
            this.process(responses, tx.clone(), *tx_uuid).await?;
            this.to_process
                .pop()
                .expect("to_process should not be empty");
        }

        tokio::select! {
            value = checkpoints.recv() => {
                if let Some(checkpoint) = value {
                    log::info!("Received checkpoint: {checkpoint}");
                    this.last_checkpoint = Some(checkpoint);

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
    gear_api: &gclient::GearApi,
    eth_client: &EthApi,
    tx_hash: TxHash,
    historical_proxy_id: ActorId,
) -> anyhow::Result<EthToVaraEvent> {
    let provider = eth_client.raw_provider();

    let receipt = provider
        .get_transaction_receipt(tx_hash)
        .await?
        .ok_or(anyhow::anyhow!("Transaction receipt is missing"))?;

    let block = match receipt.block_hash {
        Some(hash) => provider
            .get_block_by_hash(hash)
            .await?
            .ok_or(anyhow::anyhow!("Ethereum block (hash) is missing"))?,
        None => match receipt.block_number {
            Some(number) => provider
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
        gear_api,
        &beacon_root_parent,
        block_number,
        historical_proxy_id,
    )
    .await?;

    // receipt Merkle-proof
    let tx_index = receipt
        .transaction_index
        .ok_or(anyhow::anyhow!("Unable to determine transaction index"))?;
    let receipts = provider
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
    gear_api: &gclient::GearApi,
    beacon_root_parent: &[u8; 32],
    block_number: u64,
    historical_proxy_id: ActorId,
) -> anyhow::Result<BlockInclusionProof> {
    let remoting = GClientRemoting::new(gear_api.clone());

    let historical_proxy = HistoricalProxy::new(remoting.clone());
    let eth_events = eth_events_electra_client::EthereumEventClient::new(remoting.clone());
    let service_checkpoint = ServiceCheckpointFor::new(remoting);

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

    let endpoint = historical_proxy
        .endpoint_for(slot)
        .recv(historical_proxy_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(|e| anyhow::anyhow!("Proxy faield to get endpoint for slot #{}: {:?}", slot, e))?;

    let checkpoint_endpoint = eth_events
        .checkpoint_light_client_address()
        .recv(endpoint)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let (checkpoint_slot, checkpoint) = service_checkpoint
        .get(slot)
        .recv(checkpoint_endpoint)
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(|e| anyhow::anyhow!("Checkpoint error: {:?}", e))?;

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
