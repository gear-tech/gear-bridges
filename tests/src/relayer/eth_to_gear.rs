use alloy::primitives::{fixed_bytes, FixedBytes};
use alloy::rpc::types::TransactionReceipt;
use alloy_rlp::Encodable;
use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent};
use ethereum_common::utils::{BeaconBlockHeaderResponse, MerkleProof, ReceiptEnvelope};
use ethereum_common::{
    beacon::electra::Block,
    utils::{self as eth_utils, BeaconBlockResponse},
};
use primitive_types::H256;
use relayer::message_relayer::common::{EthereumSlotNumber, TxHashWithSlot};
use relayer::message_relayer::eth_to_gear::api_provider::ApiProvider;
use relayer::message_relayer::eth_to_gear::message_sender::MessageSender;
use relayer::message_relayer::eth_to_gear::proof_composer::ProofComposer;
use relayer::message_relayer::eth_to_gear::storage::NoStorage;
use relayer::message_relayer::eth_to_gear::{
    message_sender::{self, MessageSenderIo},
    proof_composer::{self, ProofComposerIo},
    tx_manager::*,
};
use ruzstd::{self, StreamingDecoder};
use sails_rs::calls::{ActionIo, Call};
use sails_rs::gclient::calls::GClientRemoting;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use vft_manager_client::traits::VftManager;

use std::sync::{Arc, LazyLock};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Deserialize, Debug)]
pub struct Receipts {
    pub result: Vec<TransactionReceipt>,
}

#[derive(Deserialize, Debug)]
pub struct Tx {
    pub tx_hash: FixedBytes<32>,
    pub tx_index: u64,

    pub slot_number: u64,
    pub checkpoint: u64,

    pub receipts: Receipts,
    pub block: BeaconBlockResponse<Block>,
    pub headers: Vec<BeaconBlockHeaderResponse>,
}
use primitive_types::H160;

impl Tx {
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

pub static TRANSACTIONS: LazyLock<BTreeMap<FixedBytes<32>, Tx>> = LazyLock::new(|| {
    let mut txs = TRANSACTIONS_BYTES;
    let mut decoder = StreamingDecoder::new(&mut txs).unwrap();
    let mut result = Vec::new();
    decoder.read_to_end(&mut result).unwrap();

    let txs: Vec<Tx> = serde_json::from_slice(&result).unwrap();

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

struct MockMessageSender;

impl MockMessageSender {
    async fn run(
        mut requests: UnboundedReceiver<message_sender::Request>,
        responses: UnboundedSender<message_sender::Response>,
    ) {
        tokio::task::spawn(async move {
            loop {
                if requests.is_closed() || responses.is_closed() {
                    return;
                }
                let req = requests.recv().await.unwrap();

                let tx = TRANSACTIONS.get(&req.tx_hash).unwrap();
                assert_eq!(tx.event(), req.payload);
                println!("send message for #{}: {:?}", req.tx_uuid, req.tx_hash);
                if req.tx_hash == TX_TO_FAIL {
                    responses
                        .send(message_sender::Response {
                            tx_uuid: req.tx_uuid,
                            status: message_sender::MessageStatus::Failure(
                                "Mock failure for testing".to_string(),
                            ),
                        })
                        .unwrap();
                    continue;
                }
                responses
                    .send(message_sender::Response {
                        tx_uuid: req.tx_uuid,
                        status: message_sender::MessageStatus::Success,
                    })
                    .unwrap();
            }
        });
    }
}

#[tokio::test]
async fn test_relayer_mock() {
    let txs = &*TRANSACTIONS;

    for (_, tx) in txs.iter() {
        println!(
            "Eth token ID for transaction {:?}: {:?}",
            tx.tx_hash,
            tx.eth_token_id()
        );
    }

    let (events_tx, mut events_rx) = unbounded_channel();

    for (_, tx) in txs.iter() {
        let tx_event = TxHashWithSlot {
            tx_hash: tx.tx_hash,
            slot_number: EthereumSlotNumber(tx.slot_number),
        };

        events_tx.send(tx_event).unwrap();
    }

    let (proof_req_tx, proof_req_rx) = unbounded_channel();
    let (proof_res_tx, proof_res_rx) = unbounded_channel();

    let mut proof_composer = ProofComposerIo::new(proof_req_tx, proof_res_rx);

    let (message_req_tx, message_req_rx) = unbounded_channel();
    let (message_res_tx, message_res_rx) = unbounded_channel();

    let mut message_sender = MessageSenderIo::new(message_req_tx, message_res_rx);

    let tx_manager = TransactionManager::new(Arc::new(NoStorage::new()));
    MockProofComposer::run(proof_req_rx, proof_res_tx).await;
    MockMessageSender::run(message_req_rx, message_res_tx).await;
    loop {
        let res = tx_manager
            .process(&mut events_rx, &mut proof_composer, &mut message_sender)
            .await;
        assert!(matches!(res, Ok(true)));

        if tx_manager.completed.read().await.len() == txs.len() - 1
            && tx_manager.failed.read().await.len() == 1
        {
            break;
        }
    }

    for (_, tx) in tx_manager.completed.read().await.iter() {
        assert!(
            txs.contains_key(&tx.tx.tx_hash),
            "Transaction {:?} not found in test data",
            tx.tx.tx_hash
        );
    }

    let failed = tx_manager.failed.read().await;

    let (failed_tx, msg) = failed
        .iter()
        .next()
        .expect("Expected one failed transaction");

    assert_eq!(
        tx_manager
            .transactions
            .read()
            .await
            .get(failed_tx)
            .unwrap()
            .tx
            .tx_hash,
        TX_TO_FAIL,
        "Failed transaction does not match expected hash"
    );
    assert_eq!(
        msg, "Mock failure for testing",
        "Failed transaction does not match expected failure message"
    );
    println!("Failed transaction: {:?}, reason: {}", failed_tx, msg);

    drop(events_tx);
}

#[tokio::test]
async fn test_api_provider() {
    let api_provider = ApiProvider::new("ws://127.0.0.1".to_owned(), 9944, 1)
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

#[tokio::test]
async fn test_tx_manager() {
    
    pretty_env_logger::formatted_timed_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .filter(Some("prover"), log::LevelFilter::Info)
        .filter(Some("relayer"), log::LevelFilter::Debug)
        .filter(Some("ethereum-client"), log::LevelFilter::Info)
        .filter(Some("metrics"), log::LevelFilter::Info)
        .format_timestamp_secs()
        .parse_default_env()
        .init();
    let contracts = super::upload::EthContracts::new().await;
    let (eth, beacon, api_provider) = super::upload::connections().await;

    let mut conn = api_provider.connection();

    let client = conn
        .gclient_client(&contracts.suri)
        .expect("Failed to create GClient client");

    let (checkpoints_tx, checkpoints_rx) = unbounded_channel();

    let proof_composer = ProofComposer::new(
        conn.clone(),
        beacon.clone(),
        eth.clone(),
        H256(contracts.historical_proxy.into_bytes()),
        contracts.suri.clone(),
    );
    let mut proof_composer_io = proof_composer.run(checkpoints_rx);

    let message_sender = MessageSender::new(
        contracts.vft_manager.into(),
        <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec(),
        contracts.historical_proxy.into(),
        conn.clone(),
        contracts.suri.clone(),
    );

    let mut message_sender_io = message_sender.run();

    let tx_manager = TransactionManager::new(Arc::new(NoStorage::new()));

    let (events_tx, mut events_rx) = unbounded_channel();

    for (_, tx_data) in TRANSACTIONS.iter().filter(|(hash, _)| **hash != TX_TO_FAIL) {
        let tx_event = TxHashWithSlot {
            tx_hash: tx_data.tx_hash,
            slot_number: EthereumSlotNumber(tx_data.slot_number),
        };

        events_tx.send(tx_event).unwrap();
        checkpoints_tx
            .send(EthereumSlotNumber(tx_data.checkpoint))
            .unwrap();
    }

    while let Ok(true) = tx_manager
        .process(
            &mut events_rx,
            &mut proof_composer_io,
            &mut message_sender_io,
        )
        .await
    {
        if tx_manager.completed.read().await.len() == TRANSACTIONS.len() - 1 {
            break;
        }
    }

    let remoting = GClientRemoting::new(client.clone());

    vft_manager_client::VftManager::new(remoting)
        .pause()
        .send_recv(contracts.vft_manager.into())
        .await
        .expect("Failed to pause vft-manager");

    events_tx
        .send(TxHashWithSlot {
            tx_hash: TX_TO_FAIL,
            slot_number: EthereumSlotNumber(TRANSACTIONS.get(&TX_TO_FAIL).unwrap().slot_number),
        })
        .unwrap();

    checkpoints_tx.send(EthereumSlotNumber(
        TRANSACTIONS.get(&TX_TO_FAIL).unwrap().checkpoint,
    )).unwrap();

    while let Ok(true) = tx_manager
        .process(
            &mut events_rx,
            &mut proof_composer_io,
            &mut message_sender_io,
        )
        .await
    {
        if tx_manager.failed.read().await.len() == 1 {
            break;
        }
    }

    let failed = tx_manager.failed.read().await;
    let msg = failed.first_key_value().unwrap().1;

    assert!(msg.contains("Paused"));

    drop(events_tx);
    // now let's upgrade vft-manager and then check that tx manager fails the transaction
}
