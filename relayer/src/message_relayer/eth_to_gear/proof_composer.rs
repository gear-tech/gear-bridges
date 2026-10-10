use std::ops::ControlFlow;

use crate::{
    message_relayer::common::{EthereumSlotNumber, TxHashWithSlot},
    rpc,
};
use alloy::{providers::Provider, rpc::types::TransactionReceipt};
use alloy_eips::BlockId;
use alloy_rlp::Encodable;
use anyhow::Context;
use checkpoint_light_client_client::{traits::ServiceCheckpointFor as _, ServiceCheckpointFor};
use eth_events_electra_client::{
    traits::EthereumEventClient, BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{PollingEthApi, TxHash};
use ethereum_common::{
    beacon, hash_db, memory_db,
    patricia_trie::TrieDB,
    tree_hash::TreeHash,
    trie_db::{HashDB, Trie},
    utils as eth_utils,
    utils::MerkleProof,
    Hash256,
};
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

    anyhow::ensure!(
        receipt.transaction_hash == tx_hash,
        "Receipt does not identify the requested transaction"
    );
    let block_hash = receipt
        .block_hash
        .context("Original receipt block hash is missing")?;
    let block_number = receipt
        .block_number
        .context("Original receipt block number is missing")?;
    let block = eth_client
        .get_block_by_hash(block_hash)
        .await?
        .context("Ethereum block (hash) is missing")?;
    anyhow::ensure!(
        block.header.hash == block_hash
            && block.header.number == block_number
            && block.header.inner.hash_slow() == block_hash,
        "Ethereum header does not match original receipt block identity"
    );
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

    log::info!("compose: building inclusion proof tx_hash={tx_hash:?} block_number={block_number} beacon_root_parent={beacon_root_parent:?}");

    let inclusion = build_inclusion_proof(
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
        inclusion.block.slot,
        inclusion.headers.len()
    );

    let tx_index = receipt
        .transaction_index
        .ok_or(anyhow::anyhow!("Unable to determine transaction index"))?;
    log::info!("compose: fetching block receipts tx_hash={tx_hash:?} block_number={block_number} tx_index={tx_index}");
    let receipts = eth_client
        .get_block_receipts(BlockId::hash(block_hash))
        .await
        .inspect_err(|e| log::info!("compose: get_block_receipts failed block_hash={block_hash:?} tx_hash={tx_hash:?} err={e:?}"))?
        .context("Ethereum block receipts are missing")?;
    compose_event(tx_hash, &receipt, inclusion, receipts)
}

struct FullBlockInclusionProof {
    block: beacon::electra::Block,
    headers: Vec<beacon::BlockHeader>,
}

fn compose_event(
    tx_hash: TxHash,
    original_receipt: &TransactionReceipt,
    inclusion: FullBlockInclusionProof,
    mut receipts: Vec<TransactionReceipt>,
) -> anyhow::Result<EthToVaraEvent> {
    let payload = &inclusion.block.body.execution_payload;
    let block_hash = TxHash::from(payload.block_hash.0 .0);
    anyhow::ensure!(
        original_receipt.transaction_hash == tx_hash,
        "Receipt does not identify the requested transaction"
    );
    anyhow::ensure!(
        original_receipt.block_hash == Some(block_hash)
            && original_receipt.block_number == Some(payload.block_number),
        "Original receipt block metadata differs from authenticated execution payload"
    );
    let tx_index = original_receipt
        .transaction_index
        .context("Unable to determine transaction index")?;
    let selected_index = usize::try_from(tx_index).context("Transaction index is too large")?;
    let transaction = payload
        .transactions
        .get(selected_index)
        .context("Transaction index is outside authenticated execution payload")?;
    anyhow::ensure!(
        alloy_primitives::keccak256(transaction.0.as_ref()) == tx_hash,
        "Requested hash differs from authenticated transaction at receipt index"
    );

    anyhow::ensure!(
        receipts.len() == payload.transactions.len(),
        "Receipt count differs from authenticated transaction count"
    );
    receipts.sort_unstable_by_key(|receipt| receipt.transaction_index);
    let receipts = receipts
        .iter()
        .enumerate()
        .map(|(index, receipt)| {
            anyhow::ensure!(
                receipt.transaction_index == Some(index as u64),
                "Receipt indices must be complete, contiguous and unique"
            );
            anyhow::ensure!(
                receipt.block_hash == Some(block_hash)
                    && receipt.block_number == Some(payload.block_number),
                "Receipt block metadata differs from authenticated execution payload"
            );
            let expected_hash = if index == selected_index {
                tx_hash
            } else {
                alloy_primitives::keccak256(payload.transactions[index].0.as_ref())
            };
            anyhow::ensure!(
                receipt.transaction_hash == expected_hash,
                "Receipt hash differs from authenticated transaction at its index"
            );
            Ok((
                index as u64,
                eth_utils::map_receipt_envelope(receipt.as_ref()),
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    anyhow::ensure!(
        receipts[selected_index].1 == eth_utils::map_receipt_envelope(original_receipt.as_ref()),
        "Original receipt differs from hash-pinned block receipt"
    );

    let MerkleProof { proof, receipt } = eth_utils::generate_merkle_proof(tx_index, &receipts)
        .context("Unable to generate receipt proof")?;
    let receipts_root = Hash256::from(payload.receipts_root.0 .0);
    let mut memory_db = memory_db::new();
    for node in &proof {
        memory_db.insert(hash_db::EMPTY_PREFIX, node);
    }
    let trie = TrieDB::new(&memory_db, &receipts_root)
        .map_err(|_| anyhow::anyhow!("Receipt proof does not match authenticated receipts root"))?;
    let (key, value) = eth_utils::rlp_encode_index_and_receipt(&tx_index, &receipt);
    anyhow::ensure!(
        matches!(trie.get(&key), Ok(Some(found)) if found == value),
        "Receipt proof value does not match authenticated receipts root"
    );

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
    Encodable::encode(&receipt, &mut receipt_rlp);
    Ok(EthToVaraEvent {
        proof_block: BlockInclusionProof {
            block: BlockGenericForBlockBody {
                slot: inclusion.block.slot,
                proposer_index: inclusion.block.proposer_index,
                parent_root: inclusion.block.parent_root,
                state_root: inclusion.block.state_root,
                body: inclusion.block.body.into(),
            },
            headers: inclusion.headers,
        },
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
) -> anyhow::Result<FullBlockInclusionProof> {
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
    anyhow::ensure!(
        beacon_block_parent.tree_hash_root() == Hash256::from(*beacon_root_parent),
        "Parent Beacon block differs from requested root"
    );
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
    let selected_slot = beacon_block.slot;
    let beacon_block = beacon_client
        .get_block::<beacon::electra::Block>(selected_slot)
        .await
        .inspect_err(|e| {
            log::info!("build_inclusion_proof: get_block failed slot={selected_slot} err={e:?}")
        })?;
    anyhow::ensure!(
        beacon_block.slot == selected_slot
            && beacon_block.body.execution_payload.block_number == block_number,
        "Selected Beacon block slot or execution number differs from request"
    );
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

    anyhow::ensure!(
        slot <= checkpoint_slot,
        "Checkpoint is behind selected Beacon block"
    );
    let headers = if slot == checkpoint_slot {
        vec![]
    } else {
        beacon_client
            .request_headers(
                slot.checked_add(1).context("Beacon slot overflow")?,
                checkpoint_slot
                    .checked_add(1)
                    .context("Checkpoint slot overflow")?,
            )
            .await?
    };
    authenticate_beacon_block(beacon_block, checkpoint_slot, checkpoint, headers)
}

fn authenticate_beacon_block(
    block: beacon::electra::Block,
    checkpoint_slot: u64,
    checkpoint: Hash256,
    headers: Vec<beacon::BlockHeader>,
) -> anyhow::Result<FullBlockInclusionProof> {
    anyhow::ensure!(
        block.slot <= checkpoint_slot,
        "Checkpoint is behind selected Beacon block"
    );
    let mut previous_slot = block.slot;
    for header in &headers {
        anyhow::ensure!(
            previous_slot < header.slot && header.slot <= checkpoint_slot,
            "Invalid Beacon header slot sequence"
        );
        previous_slot = header.slot;
    }
    anyhow::ensure!(
        previous_slot == checkpoint_slot,
        "Missing checkpoint Beacon header"
    );
    let ControlFlow::Continue(selected_root) =
        headers
            .iter()
            .rev()
            .try_fold(checkpoint, |block_root_parent, header| {
                if header.tree_hash_root() == block_root_parent {
                    ControlFlow::Continue(header.parent_root)
                } else {
                    ControlFlow::Break(())
                }
            })
    else {
        return Err(anyhow::anyhow!("Invalid block proof"));
    };
    // The full body (including raw transactions) is still available at this trust boundary.
    anyhow::ensure!(
        block.tree_hash_root() == selected_root,
        "Selected Beacon block does not match authenticated checkpoint chain"
    );
    Ok(FullBlockInclusionProof { block, headers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_rlp::Decodable;
    use ethereum_common::utils::{BeaconBlockHeaderResponse, BeaconBlockResponse};
    use serde::Deserialize;
    use std::{io::Read, sync::LazyLock};

    #[derive(Deserialize)]
    struct Receipts {
        result: Vec<TransactionReceipt>,
    }

    #[derive(Deserialize)]
    struct Fixture {
        tx_hash: TxHash,
        tx_index: u64,
        receipts: Receipts,
        block: BeaconBlockResponse<beacon::electra::Block>,
        headers: Vec<BeaconBlockHeaderResponse>,
    }

    fn fixture() -> &'static Fixture {
        static FIXTURE: LazyLock<Fixture> = LazyLock::new(|| {
            let bytes: &[u8] =
                include_bytes!("../../../../tests/src/relayer/transactions.json.zst");
            let mut decoder = ruzstd::StreamingDecoder::new(bytes).unwrap();
            let mut decoded = Vec::new();
            decoder.read_to_end(&mut decoded).unwrap();
            let fixtures: Vec<Fixture> = serde_json::from_slice(&decoded).unwrap();
            fixtures
                .into_iter()
                .min_by_key(|fixture| fixture.tx_hash)
                .unwrap()
        });
        &FIXTURE
    }

    fn headers() -> Vec<beacon::BlockHeader> {
        let mut headers: Vec<_> = fixture()
            .headers
            .iter()
            .map(|header| header.data.header.message.clone())
            .collect();
        headers.sort_unstable_by_key(|header| header.slot);
        headers
    }

    fn inclusion(
        block: beacon::electra::Block,
        equal_slot: bool,
    ) -> anyhow::Result<FullBlockInclusionProof> {
        let mut headers = headers();
        let (slot, root) = if equal_slot {
            (
                fixture().block.data.message.slot,
                headers.first().unwrap().parent_root,
            )
        } else {
            let checkpoint = headers.last().unwrap();
            (checkpoint.slot, checkpoint.tree_hash_root())
        };
        if equal_slot {
            headers.clear();
        }
        authenticate_beacon_block(block, slot, root, headers)
    }

    fn original_receipt() -> &'static TransactionReceipt {
        let fixture = fixture();
        fixture
            .receipts
            .result
            .iter()
            .find(|receipt| receipt.transaction_index == Some(fixture.tx_index))
            .unwrap()
    }

    fn alter_receipt(receipt: &mut TransactionReceipt) {
        let mut json = serde_json::to_value(&*receipt).unwrap();
        assert_ne!(json["cumulativeGasUsed"], serde_json::json!("0x1"));
        json["cumulativeGasUsed"] = serde_json::json!("0x1");
        *receipt = serde_json::from_value(json).unwrap();
    }

    #[test]
    fn rejects_misrouted_and_relabeled_same_block_transaction() {
        let fixture = fixture();
        let receipts = &fixture.receipts.result;
        let block = &fixture.block.data.message;
        let sibling = receipts
            .iter()
            .find(|receipt| receipt.transaction_index != Some(fixture.tx_index) && receipt.status())
            .unwrap();
        // T2 itself has a genuine proof; only substituting it for the queued T1 must fail.
        let event = compose_event(
            sibling.transaction_hash,
            sibling,
            inclusion(block.clone(), false).unwrap(),
            receipts.clone(),
        )
        .unwrap();
        assert_eq!(event.transaction_index, sibling.transaction_index.unwrap());
        compose_event(
            fixture.tx_hash,
            sibling,
            inclusion(block.clone(), false).unwrap(),
            receipts.clone(),
        )
        .unwrap_err();
        let mut relabeled = sibling.clone();
        relabeled.transaction_hash = fixture.tx_hash;
        let mut relabeled_receipts = receipts.clone();
        relabeled_receipts
            .iter_mut()
            .find(|receipt| receipt.transaction_index == sibling.transaction_index)
            .unwrap()
            .transaction_hash = fixture.tx_hash;
        compose_event(
            fixture.tx_hash,
            &relabeled,
            inclusion(block.clone(), false).unwrap(),
            relabeled_receipts,
        )
        .unwrap_err();
    }

    #[test]
    fn authenticates_selected_block_and_receipt_root() {
        let fixture = fixture();
        let block = &fixture.block.data.message;
        let receipts = &fixture.receipts.result;
        let original = original_receipt();
        for equal_slot in [true, false] {
            let event = compose_event(
                fixture.tx_hash,
                original,
                inclusion(block.clone(), equal_slot).unwrap(),
                receipts.clone(),
            )
            .unwrap();
            assert_eq!(event.transaction_index, fixture.tx_index);
            let light = &event.proof_block.block;
            let header = beacon::BlockHeader {
                slot: light.slot,
                proposer_index: light.proposer_index,
                parent_root: light.parent_root,
                state_root: light.state_root,
                body_root: light.body.tree_hash_root(),
            };
            assert_eq!(header.tree_hash_root(), block.tree_hash_root());
            let decoded = eth_utils::ReceiptEnvelope::decode(&mut &event.receipt_rlp[..]).unwrap();
            assert_eq!(decoded, eth_utils::map_receipt_envelope(original.as_ref()));
            let mut changed_block = block.clone();
            changed_block.state_root = Hash256::repeat_byte(7);
            assert!(inclusion(changed_block, equal_slot).is_err());
        }
        let canonical = headers();
        let checkpoint = canonical.last().unwrap();
        let mut reversed = canonical.clone();
        reversed.reverse();
        assert!(authenticate_beacon_block(
            block.clone(),
            checkpoint.slot,
            checkpoint.tree_hash_root(),
            reversed
        )
        .is_err());
        let mut duplicate = canonical.clone();
        duplicate.insert(1, duplicate[0].clone());
        assert!(authenticate_beacon_block(
            block.clone(),
            checkpoint.slot,
            checkpoint.tree_hash_root(),
            duplicate
        )
        .is_err());
        let mut incomplete_headers = headers();
        let checkpoint = incomplete_headers.last().unwrap().clone();
        incomplete_headers.remove(0);
        assert!(authenticate_beacon_block(
            block.clone(),
            checkpoint.slot,
            checkpoint.tree_hash_root(),
            incomplete_headers
        )
        .is_err());

        let mut unordered = receipts.clone();
        unordered.reverse();
        let event = compose_event(
            fixture.tx_hash,
            original,
            inclusion(block.clone(), false).unwrap(),
            unordered,
        )
        .unwrap();
        assert_eq!(event.transaction_index, fixture.tx_index);
        for mutation in [
            "receipt trie",
            "count",
            "duplicate index",
            "index gap",
            "missing index",
            "block hash",
            "block number",
            "transaction hash",
            "original receipt",
        ] {
            let mut altered = receipts.clone();
            let mut original = original.clone();
            match mutation {
                "receipt trie" => {
                    let sibling = altered
                        .iter_mut()
                        .find(|receipt| receipt.transaction_index != Some(fixture.tx_index))
                        .unwrap();
                    alter_receipt(sibling);
                }
                "count" => {
                    altered.pop();
                }
                "duplicate index" => {
                    altered[1].transaction_index = altered[0].transaction_index;
                }
                "index gap" => {
                    altered[0].transaction_index = Some(u64::MAX);
                }
                "missing index" => {
                    altered[0].transaction_index = None;
                }
                "block hash" => {
                    altered[0].block_hash = Some(TxHash::ZERO);
                }
                "block number" => {
                    altered[0].block_number = Some(0);
                }
                "transaction hash" => {
                    altered[0].transaction_hash = TxHash::ZERO;
                }
                "original receipt" => alter_receipt(&mut original),
                _ => unreachable!(),
            }
            assert!(
                compose_event(
                    fixture.tx_hash,
                    &original,
                    inclusion(block.clone(), false).unwrap(),
                    altered
                )
                .is_err(),
                "{mutation}"
            );
        }
    }
}
