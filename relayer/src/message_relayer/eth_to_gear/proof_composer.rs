use std::ops::ControlFlow;

use crate::{
    message_relayer::common::{EthereumSlotNumber, TxHashWithSlot},
    rpc,
};
use alloy::providers::Provider;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_rlp::Encodable;
use anyhow::Context;
use checkpoint_light_client_client::{traits::ServiceCheckpointFor as _, ServiceCheckpointFor};
use eth_events_electra_client::{
    traits::EthereumEventClient, BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{PollingEthApi, TxHash};
use ethereum_common::{beacon, tree_hash::TreeHash, utils as eth_utils, utils::MerkleProof};
use futures::executor::block_on;
use gear_common::api_provider::ApiProviderConnection;
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::H256;
use prometheus::IntGauge;
use sails_rs::{
    calls::{Action, Query},
    gclient::calls::GClientRemoting,
    ActorId,
};
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
        let gear_api = self.api_provider.gclient_client(&self.suri)?;

        match compose(
            &self.beacon_client,
            &gear_api,
            &self.eth_api,
            tx.tx_hash,
            self.historical_proxy_address.0.into(),
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

            match this.api_provider.reconnect().await {
                Ok(_) => log::info!("Successfully reconnected to Gear API"),
                Err(err) => {
                    log::error!("Failed to reconnect to Gear API: {err:?}");
                    return;
                }
            }

            match rpc::reconnect_polling_eth(&mut this.eth_api).await {
                Ok(()) => log::info!("Successfully reconnected to Ethereum API"),
                Err(err) => {
                    log::error!("Failed to reconnect to Ethereum API: {err:?}");
                    return;
                }
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
                    this.to_process.push((tx_uuid, tx));
                    return Err(err); // force reconnect
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
    gear_api: &gclient::GearApi,
    eth_client: &PollingEthApi,
    tx_hash: TxHash,
    historical_proxy_id: ActorId,
) -> anyhow::Result<EthToVaraEvent> {
    log::info!("compose: start tx_hash={tx_hash:?} historical_proxy={historical_proxy_id:?}");
    let receipt = eth_client
        .get_transaction_receipt(tx_hash)
        .await
        .inspect_err(|e| {
            log::info!("compose: get_transaction_receipt failed tx_hash={tx_hash:?} err={e:?}")
        })?
        .ok_or(anyhow::anyhow!("Transaction receipt is missing"))?;
    log::info!(
        "compose: receipt ok tx_hash={tx_hash:?} block_number={:?} block_hash={:?} tx_index={:?} status={:?}",
        receipt.block_number,
        receipt.block_hash,
        receipt.transaction_index,
        receipt.status()
    );

    let block = match receipt.block_hash {
        Some(hash) => {
            log::info!("compose: fetching block by hash {hash:?} tx_hash={tx_hash:?}");
            eth_client
                .get_block_by_hash(hash)
                .await
                .inspect_err(|e| log::info!("compose: get_block_by_hash failed hash={hash:?} tx_hash={tx_hash:?} err={e:?}"))?
                .ok_or(anyhow::anyhow!("Ethereum block (hash) is missing"))?
        }
        None => match receipt.block_number {
            Some(number) => {
                log::info!("compose: fetching block by number {number} tx_hash={tx_hash:?}");
                eth_client
                    .get_block_by_number(BlockNumberOrTag::Number(number))
                    .await
                    .inspect_err(|e| log::info!("compose: get_block_by_number failed number={number} tx_hash={tx_hash:?} err={e:?}"))?
                    .ok_or(anyhow::anyhow!("Ethereum block (number) is missing"))?
            }
            None => return Err(anyhow::anyhow!("Unable to get Ethereum block")),
        },
    };
    log::info!(
        "compose: block ok tx_hash={tx_hash:?} block_number={} parent_beacon_root={:?}",
        block.header.number,
        block.header.parent_beacon_block_root
    );

    let beacon_root_parent = block
        .header
        .parent_beacon_block_root
        .ok_or(anyhow::anyhow!(
            "Unable to determine root of parent beacon block"
        ))?;
    let block_number = block.header.number;
    log::info!("compose: building inclusion proof tx_hash={tx_hash:?} block_number={block_number} beacon_root_parent={beacon_root_parent:?}");

    let proof_block = build_inclusion_proof(
        beacon_client,
        gear_api,
        &beacon_root_parent,
        block_number,
        historical_proxy_id,
    )
    .await
    .inspect_err(|e| log::info!("compose: build_inclusion_proof failed tx_hash={tx_hash:?} block_number={block_number} err={e:?}"))?;
    log::info!(
        "compose: inclusion proof ok tx_hash={tx_hash:?} slot={} headers={} block_number={block_number}",
        proof_block.block.slot,
        proof_block.headers.len()
    );

    // receipt Merkle-proof
    let tx_index = receipt
        .transaction_index
        .ok_or(anyhow::anyhow!("Unable to determine transaction index"))?;
    log::info!("compose: fetching block receipts tx_hash={tx_hash:?} block_number={block_number} tx_index={tx_index}");
    let receipts = eth_client
        .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(block_number)))
        .await
        .inspect_err(|e| log::info!("compose: get_block_receipts failed block_number={block_number} tx_hash={tx_hash:?} err={e:?}"))?
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
    log::info!(
        "compose: receipts ok tx_hash={tx_hash:?} count={} tx_index={tx_index}",
        receipts.len()
    );

    let MerkleProof { proof, receipt } = eth_utils::generate_merkle_proof(tx_index, &receipts[..])
        .inspect_err(|e| log::info!("compose: generate_merkle_proof failed tx_hash={tx_hash:?} tx_index={tx_index} err={e:?}"))?;
    log::info!(
        "compose: merkle proof ok tx_hash={tx_hash:?} proof_len={}",
        proof.len()
    );

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
    Encodable::encode(&receipt, &mut receipt_rlp);
    log::info!(
        "compose: success tx_hash={tx_hash:?} tx_index={tx_index} receipt_rlp_len={} slot={}",
        receipt_rlp.len(),
        proof_block.block.slot
    );

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
    log::info!("build_inclusion_proof: start block_number={block_number} beacon_root_parent={beacon_root_parent:?} historical_proxy={historical_proxy_id:?}");
    let remoting = GClientRemoting::new(gear_api.clone());

    let historical_proxy = HistoricalProxy::new(remoting.clone());
    let eth_events = eth_events_electra_client::EthereumEventClient::new(remoting.clone());
    let service_checkpoint = ServiceCheckpointFor::new(remoting);

    log::info!("build_inclusion_proof: get_block_by_hash parent_root={beacon_root_parent:?} block_number={block_number}");
    let beacon_block_parent = beacon_client
        .get_block_by_hash::<beacon::electra::Block>(beacon_root_parent)
        .await
        .inspect_err(|e| log::info!("build_inclusion_proof: get_block_by_hash failed parent_root={beacon_root_parent:?} err={e:?}"))?;
    log::info!(
        "build_inclusion_proof: parent ok slot={} block_number={block_number}",
        beacon_block_parent.slot
    );

    log::info!(
        "build_inclusion_proof: find_beacon_block block_number={block_number} parent_slot={}",
        beacon_block_parent.slot
    );
    let beacon_block = beacon_client
        .find_beacon_block(block_number, beacon_block_parent)
        .await
        .inspect_err(|e| log::info!("build_inclusion_proof: find_beacon_block failed block_number={block_number} err={e:?}"))?;
    log::info!(
        "build_inclusion_proof: find ok slot={} block_number={block_number}",
        beacon_block.slot
    );

    log::info!(
        "build_inclusion_proof: get_block slot={} block_number={block_number}",
        beacon_block.slot
    );
    let beacon_block = beacon_client
        .get_block::<beacon::electra::Block>(beacon_block.slot)
        .await
        .inspect_err(|e| {
            log::info!(
                "build_inclusion_proof: get_block failed slot={} err={e:?}",
                beacon_block.slot,
            )
        })?;
    log::info!(
        "build_inclusion_proof: get_block ok slot={} proposer={}",
        beacon_block.slot,
        beacon_block.proposer_index
    );

    let slot = beacon_block.slot;
    let gas_limit = gear_api
        .block_gas_limit()
        .inspect_err(|e| log::info!("build_inclusion_proof: block_gas_limit failed err={e:?}"))?;
    log::info!("build_inclusion_proof: historical_proxy.endpoint_for slot={slot} proxy={historical_proxy_id:?} gas_limit={gas_limit}");
    let endpoint = historical_proxy
        .endpoint_for(slot)
        .recv(historical_proxy_id)
        .await
        .inspect_err(|e| {
            log::info!("build_inclusion_proof: endpoint_for recv failed slot={slot} err={e:?}")
        })
        .map_err(|e| anyhow::anyhow!("Failed to receive endpoint: {e:?}"))?
        .inspect_err(|e| {
            log::info!("build_inclusion_proof: endpoint_for Proxy error slot={slot} err={e:?}")
        })
        .map_err(|e| anyhow::anyhow!("Proxy failed to get endpoint for slot #{slot}: {e:?}"))?;
    log::info!("build_inclusion_proof: endpoint ok slot={slot} endpoint={endpoint:?}");

    log::info!(
        "build_inclusion_proof: checkpoint_light_client_address endpoint={endpoint:?} slot={slot}"
    );
    let checkpoint_endpoint = eth_events
        .checkpoint_light_client_address()
        .recv(endpoint)
        .await
        .inspect_err(|e| log::info!("build_inclusion_proof: checkpoint_light_client_address failed endpoint={endpoint:?} err={e:?}"))
        .map_err(|e| anyhow::anyhow!("Failed to receive checkpoint endpoint: {e:?}"))?;
    log::info!("build_inclusion_proof: checkpoint_endpoint ok {checkpoint_endpoint:?} slot={slot}");

    log::info!("build_inclusion_proof: service_checkpoint.get slot={slot} checkpoint_endpoint={checkpoint_endpoint:?}");
    let (checkpoint_slot, checkpoint) = service_checkpoint
        .get(slot)
        .with_gas_limit(gas_limit)
        .recv(checkpoint_endpoint)
        .await
        .inspect_err(|e| {
            log::info!(
                "build_inclusion_proof: service_checkpoint.get recv failed slot={slot} err={e:?}"
            )
        })
        .map_err(|e| anyhow::anyhow!("Failed to receive checkpoint: {e:?}"))?
        .inspect_err(|e| {
            log::info!(
                "build_inclusion_proof: service_checkpoint.get Proxy error slot={slot} err={e:?}"
            )
        })
        .map_err(|e| anyhow::anyhow!("Checkpoint error: {e:?}"))?;
    log::info!(
        "build_inclusion_proof: checkpoint ok slot={slot} checkpoint_slot={checkpoint_slot}"
    );

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
