use alloy::{
    primitives::{fixed_bytes, FixedBytes},
    rpc::types::TransactionReceipt,
};
use alloy_rlp::Encodable;
use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent};
use ethereum_common::{
    beacon::electra::Block,
    utils::{
        self as eth_utils, BeaconBlockHeaderResponse, BeaconBlockResponse, MerkleProof,
        ReceiptEnvelope,
    },
};
use gear_common::api_provider::ApiProvider;
use relayer::message_relayer::{
    common::{EthereumSlotNumber, TxHashWithSlot},
    eth_to_gear::{
        message_sender::{MessageSender, MessageStatus},
        proof_composer::{self, ProofComposerIo},
        storage::{BlockStorage, Storage},
        tx_manager::*,
    },
};
use ruzstd::{self, StreamingDecoder};
use sails_rs::{
    calls::{Call, Query},
    gclient::calls::GClientRemoting,
    Encode,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    sync::{Arc, LazyLock},
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use vft_manager_client::traits::VftManager;

#[derive(Deserialize, Debug)]
pub struct Receipts {
    pub result: Vec<TransactionReceipt>,
}

/// Test transaction stored in the `TRANSACTIONS` map.
///
/// Loaded from the `transactions.json.zst` file, which is a compressed JSON file containing
/// a list of transactions with their details fetched from Holesky testnet. You can find
/// original transactions on `holesky.etherscan.io` or similar block explorers.
#[derive(Deserialize, Debug)]
pub struct TestTx {
    pub tx_hash: FixedBytes<32>,
    pub tx_index: u64,

    pub slot_number: u64,

    pub receipts: Receipts,
    pub block: BeaconBlockResponse<Block>,
    pub headers: Vec<BeaconBlockHeaderResponse>,
}
use primitive_types::H160;

impl TestTx {
    pub fn eth_token_id(&self) -> H160 {
        use alloy_rlp::Decodable;
        use alloy_sol_types::SolEvent;
        let event = self.event();

        let receipt =
            ReceiptEnvelope::decode(&mut &event.receipt_rlp[..]).expect("Failed to decode receipt");

        if !receipt.is_success() {
            panic!("Receipt is not successful");
        }

        let event = receipt
            .logs()
            .iter()
            .find_map(|log| {

                let event = ethereum_client::abi::IERC20Manager::BridgingRequested::decode_raw_log_validate(
                    log.topics(),
                    &log.data.data,
                )
                .ok()?;
                let eth_token_id = H160::from(event.token.0 .0);
                Some(eth_token_id)
            }).unwrap();

        event
    }

    pub fn event(&self) -> EthToVaraEvent {
        let receipts = self
            .receipts
            .result
            .iter()
            .map(|tx_receipt| {
                let receipt = tx_receipt.as_ref();
                tx_receipt
                    .transaction_index
                    .map(|i| (i, eth_utils::map_receipt_envelope(receipt)))
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        let headers = self.headers.clone();

        let MerkleProof { proof, receipt } =
            eth_utils::generate_merkle_proof(self.tx_index, &receipts).unwrap();

        let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
        Encodable::encode(&receipt, &mut receipt_rlp);

        let block = BlockGenericForBlockBody {
            slot: self.block.data.message.slot,
            proposer_index: self.block.data.message.proposer_index,
            parent_root: self.block.data.message.parent_root,
            state_root: self.block.data.message.state_root,
            body: self.block.data.message.body.clone().into(),
        };

        EthToVaraEvent {
            proof_block: BlockInclusionProof {
                block,
                headers: headers
                    .into_iter()
                    .map(|header| header.data.header.message)
                    .collect(),
            },
            proof: proof.clone(),
            transaction_index: self.tx_index,
            receipt_rlp,
        }
    }
}

static TRANSACTIONS_BYTES: &[u8] = include_bytes!("./transactions.json.zst");

/* use btreemap and btreeset to make tests behaviour predictable */

pub static TRANSACTIONS: LazyLock<BTreeMap<FixedBytes<32>, TestTx>> = LazyLock::new(|| {
    let mut txs = TRANSACTIONS_BYTES;
    let mut decoder = StreamingDecoder::new(&mut txs).unwrap();
    let mut result = Vec::new();
    decoder.read_to_end(&mut result).unwrap();

    let txs: Vec<TestTx> = serde_json::from_slice(&result).unwrap();

    txs.into_iter().map(|tx| (tx.tx_hash, tx)).collect()
});

pub static ETH_TOKEN_IDS: LazyLock<BTreeSet<H160>> =
    LazyLock::new(|| TRANSACTIONS.values().map(|tx| tx.eth_token_id()).collect());
static TX_TO_FAIL: FixedBytes<32> =
    fixed_bytes!("0xe2a0d9a04a9ce1328a79096a9df1f5f16f9c227169e9fb1b3e43a2370b54b592");

struct MockProofComposer;

impl MockProofComposer {
    async fn run(
        mut requests: UnboundedReceiver<proof_composer::Request>,
        response: UnboundedSender<proof_composer::Response>,
    ) {
        tokio::task::spawn(async move {
            loop {
                if requests.is_closed() || response.is_closed() {
                    return;
                }

                let req = requests.recv().await.unwrap();

                let tx = TRANSACTIONS.get(&req.tx.tx_hash).unwrap();

                let event = tx.event();
                println!("compose proof #{}: {:?}", req.tx_uuid, tx.tx_hash);
                response
                    .send(proof_composer::Response {
                        payload: event,
                        tx_uuid: req.tx_uuid,
                    })
                    .unwrap();
            }
        });
    }
}

#[tokio::test]
async fn test_api_provider() {
    let api_provider = ApiProvider::new("ws://127.0.0.1:9944".to_owned(), 1)
        .await
        .expect("failed to create API provider");

    let mut conn = api_provider.connection();
    let client = conn
        .gclient_client("//Alice")
        .expect("failed to create GClient client");

    assert!(
        client.block_gas_limit().is_ok(),
        "Failed to get block gas limit"
    );
    assert!(
        client.last_block_number().await.is_ok(),
        "Failed to get block number"
    );
}

#[cfg(test)]
struct TestStorage(BlockStorage);

#[cfg(test)]
#[async_trait::async_trait]
impl Storage for TestStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.0
    }
    async fn save(&self, _manager: &TransactionManager) -> anyhow::Result<()> {
        Ok(())
    }
    async fn load(&self, _manager: &TransactionManager) -> anyhow::Result<()> {
        Ok(())
    }
    async fn save_blocks(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_tx_manager() {
    let _ = pretty_env_logger::formatted_timed_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .filter(Some("prover"), log::LevelFilter::Info)
        .filter(Some("relayer"), log::LevelFilter::Debug)
        .filter(Some("ethereum-client"), log::LevelFilter::Info)
        .filter(Some("metrics"), log::LevelFilter::Info)
        .format_timestamp_secs()
        .parse_default_env()
        .try_init();
    let contracts = super::upload::EthContracts::new().await;

    let api_provider = ApiProvider::new("ws://127.0.0.1:9944".to_owned(), 2)
        .await
        .unwrap();

    let mut conn = api_provider.connection();

    let client = conn
        .gclient_client(&contracts.suri)
        .expect("Failed to create GClient client");

    let (proof_req_tx, proof_req_rx) = unbounded_channel();
    let (proof_res_tx, proof_res_rx) = unbounded_channel();

    let mut proof_composer_io = ProofComposerIo::new(proof_req_tx, proof_res_rx);

    MockProofComposer::run(proof_req_rx, proof_res_tx).await;

    let message_sender = MessageSender::new(
        contracts.vft_manager.into_bytes().into(),
        ("VftManager".to_owned(), "SubmitReceipt".to_owned()).encode(),
        contracts.historical_proxy.into_bytes().into(),
        conn.clone(),
        contracts.suri2.clone(),
        None,
    );

    let mut message_sender_io = message_sender.run();

    let tx_manager = TransactionManager::new(Arc::new(TestStorage(BlockStorage::new())));

    let (events_tx, mut events_rx) = unbounded_channel();

    // Bound the delivery batch independently of deployment and the paused-receipt scenario.
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        for (_, tx_data) in TRANSACTIONS.iter().filter(|(hash, _)| **hash != TX_TO_FAIL) {
            let tx_event = TxHashWithSlot {
                tx_hash: tx_data.tx_hash,
                slot_number: EthereumSlotNumber(tx_data.slot_number),
            };

            events_tx.send(tx_event).unwrap();
        }

        loop {
            assert!(
                tx_manager
                    .process(
                        &mut events_rx,
                        &mut proof_composer_io,
                        &mut message_sender_io,
                    )
                    .await
                    .expect("transaction manager failed while delivering receipts"),
                "transaction manager channel closed before delivery completed"
            );
            if tx_manager.completed.read().await.len() == TRANSACTIONS.len() - 1 {
                break;
            }
        }
    })
    .await
    .expect("native receipt delivery batch exceeded 120 seconds after deployment");

    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let delivered = tx_manager
            .completed
            .read()
            .await
            .values()
            .next()
            .unwrap()
            .clone();
        let mut signed = delivered
            .receipt
            .as_ref()
            .unwrap()
            .signed_submission
            .as_ref()
            .unwrap()
            .clone();
        signed.chain_genesis_hash = format!("{:#x}", primitive_types::H256::zero());
        assert!(message_sender_io.submit_prepared(
            delivered.uuid,
            delivered.tx.tx_hash,
            TRANSACTIONS.get(&delivered.tx.tx_hash).unwrap().event(),
            signed,
        ));
        let rejected = message_sender_io
            .recv()
            .await
            .expect("message sender disconnected");
        assert!(matches!(
            rejected.status,
            MessageStatus::NeedsReconciliation { .. }
        ));

        let remoting = GClientRemoting::new(client.clone());

        let mut manager = vft_manager_client::VftManager::new(remoting);
        manager
            .pause()
            .send_recv(contracts.vft_manager)
            .await
            .expect("Failed to pause vft-manager");

        events_tx
            .send(TxHashWithSlot {
                tx_hash: TX_TO_FAIL,
                slot_number: EthereumSlotNumber(TRANSACTIONS.get(&TX_TO_FAIL).unwrap().slot_number),
            })
            .unwrap();

        loop {
            assert!(
                tx_manager
                    .process(
                        &mut events_rx,
                        &mut proof_composer_io,
                        &mut message_sender_io,
                    )
                    .await
                    .expect("transaction manager failed while retaining a paused receipt"),
                "transaction manager channel closed before reconciliation evidence arrived"
            );
            if tx_manager.transactions.read().await.values().any(|tx| {
                tx.tx.tx_hash == TX_TO_FAIL
                    && matches!(tx.status, TxStatus::NeedsReconciliation { .. })
                    && tx
                        .receipt
                        .as_ref()
                        .and_then(|receipt| receipt.initial_response.as_ref())
                        .is_some()
            }) {
                break;
            }
        }

        let receipt_key = {
            let pending = tx_manager.transactions.read().await;
            let held = pending
                .values()
                .find(|tx| tx.tx.tx_hash == TX_TO_FAIL)
                .unwrap();
            assert!(matches!(held.status, TxStatus::NeedsReconciliation { .. }));
            let receipt = held.receipt.as_ref().unwrap();
            let signed = receipt.signed_submission.as_ref().unwrap();
            assert!(signed.message_id.is_some());
            assert!(signed.inclusion_block_hash.is_some());
            receipt.receipt_key
        };
        assert_eq!(
            manager
                .receipt_status(receipt_key.0, receipt_key.1)
                .recv(contracts.vft_manager)
                .await
                .unwrap(),
            vft_manager_client::ReceiptStatus::Unknown,
        );

        // drop here so that channel is not closed before the tasks finish
        drop(events_tx);
    })
    .await
    .expect("native paused-receipt scenario exceeded 120 seconds");
}
