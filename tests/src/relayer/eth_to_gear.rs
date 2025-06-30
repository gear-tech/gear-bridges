use alloy::primitives::{fixed_bytes, FixedBytes};
use alloy::rpc::types::TransactionReceipt;

use alloy::signers::k256::elliptic_curve::bigint::const_residue;
use alloy_rlp::Encodable;
use checkpoint_light_client_client::checkpoint_light_client_factory::io::Init;
use checkpoint_light_client_client::traits::CheckpointLightClientFactory;
use checkpoint_light_client_client::{service_checkpoint_for, CheckpointError};
use checkpoint_light_client_io::{FixedArray, Network, Update};
use eth_events_electra_client::traits::EthEventsElectraFactory;
use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use ethereum_common::utils::{BeaconBlockHeaderResponse, MerkleProof, ReceiptEnvelope};
use ethereum_common::{
    beacon::electra::Block,
    utils::{self as eth_utils, BeaconBlockResponse},
};

use gclient::{EventListener, EventProcessor, GearApi, WSAddress};
use gstd::{ActorId, Decode, Encode};
use historical_proxy_client::traits::{HistoricalProxy, HistoricalProxyFactory};
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
use sails_rs::calls::{ActionIo, Activation, Call, Query};
use sails_rs::gclient::calls::GClientRemoting;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use vft_client::traits::{VftAdmin, VftExtension, VftFactory};
use vft_manager_client::traits::{VftManager, VftManagerFactory as _};
use vft_manager_client::{Config, InitConfig, TokenSupply};

use std::sync::{Arc, LazyLock};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::{connect_to_node, MockEndpoint, DEFAULT_BALANCE};

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
    pub block_root: FixedBytes<32>,
    pub checkpoint_root: FixedBytes<32>,

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
                let address = H160::from(log.address.0 .0);
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

static TRANSACTIONS: LazyLock<BTreeMap<FixedBytes<32>, Tx>> = LazyLock::new(|| {
    let mut txs = TRANSACTIONS_BYTES;
    let mut decoder = StreamingDecoder::new(&mut txs).unwrap();
    let mut result = Vec::new();
    decoder.read_to_end(&mut result).unwrap();

    let txs: Vec<Tx> = serde_json::from_slice(&result).unwrap();

    txs.into_iter().map(|tx| (tx.tx_hash, tx)).collect()
});

static ETH_TOKEN_IDS: LazyLock<BTreeSet<H160>> =
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

/* steps to run relayer:
    1) run gear node in dev mode
    2) upload contracts: vft-manager, historical-proxy, vft
    3) create account with enough balance to pay fees
    4) Mock eth transactions manually from TRANSACTIONS
*/
#[tokio::test]
async fn test_relayer() {
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

    let eth_api = EthApi::new(
        "wss://reth-rpc.gear-tech.io/ws",
        "0xE3e5514AC6cAF71560777B9EaD6CaD5f6171D3de",
        "0x521030B5F81aFaaa1267748f6A7eE74735a42fc3",
        None,
    )
    .await
    .unwrap();
    let api_provider = ApiProvider::new("ws://127.0.0.1".to_owned(), 9944, 2)
        .await
        .unwrap();
    let contracts = Contracts::new().await;
    let proof_composer = ProofComposer::new(
        api_provider.connection(),
        BeaconClient::new(
            "http://testing.holesky.beacon-api.nimbus.team".to_string(),
            None,
        )
        .await
        .unwrap(),
        eth_api,
        H256::from(contracts.historical_proxy.into_bytes()),
        contracts.suri.clone(),
    );

    let client = api_provider
        .connection()
        .gclient_client(&contracts.suri)
        .expect("Failed to create GClient client");
    let remoting = GClientRemoting::new(client.clone());

    let vft_manager_admin = vft_manager_client::VftManager::new(remoting.clone())
        .admin()
        .recv(contracts.vft_manager)
        .await
        .expect("Failed to get VFT Manager admin");

    assert_eq!(vft_manager_admin, contracts.admin);

    let historical_proxy_admin = historical_proxy_client::HistoricalProxy::new(remoting.clone())
        .admin()
        .recv(contracts.historical_proxy)
        .await
        .expect("Failed to get Historical Proxy admin");
    assert_eq!(historical_proxy_admin, contracts.admin);

    let route = <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();

    let message_sender = MessageSender::new(
        H256::from(contracts.vft_manager.into_bytes()),
        route,
        H256::from(contracts.historical_proxy.into_bytes()),
        api_provider.connection(),
        contracts.suri.clone(),
    );

    let check = api_provider.connection();

    let message_sender_io = message_sender.run();

    let (mut checkpoints_tx, checkpoints_rx) = unbounded_channel();

    let mut proof_composer_io = proof_composer.run(checkpoints_rx);

    let (tx_hash, tx) = TRANSACTIONS
        .first_key_value()
        .expect("No transactions found");

    checkpoints_tx.send(EthereumSlotNumber(tx.slot_number + 100));

    let trans = Transaction::new(
        TxHashWithSlot {
            slot_number: EthereumSlotNumber(tx.slot_number),
            tx_hash: *tx_hash,
        },
        TxStatus::ComposeProof,
    );

    let listener = client
        .subscribe()
        .await
        .expect("msg failed to subscribe to API");

    proof_composer_io.compose_proof_for(trans.uuid, trans.tx);

    let mocker = CheckpointMocker(contracts.admin);

    let api = client.with(contracts.suri.clone()).unwrap();
    mocker.run(listener, &api).await;

    let proof = proof_composer_io
        .recv()
        .await
        .expect("Failed to receive proof");

    assert_eq!(proof.payload, tx.event());

    drop(checkpoints_tx);
}

struct Contracts {
    admin: ActorId,
    suri: String,
    vft_manager: ActorId,
    historical_proxy: ActorId,
    eth_events: ActorId,
    vft_id: Vec<ActorId>,
}

impl Contracts {
    async fn new() -> Self {
        let balances = vec![DEFAULT_BALANCE; ETH_TOKEN_IDS.len() + 4];
        let mut programs = vec![vft::WASM_BINARY; ETH_TOKEN_IDS.len() + 4];

        programs[1] = vft_manager::WASM_BINARY;
        programs[2] = historical_proxy::WASM_BINARY;
        programs[3] = eth_events_electra::WASM_BINARY;

        let conn = connect_to_node(&balances, "relayer", &programs).await;
        let admin = conn.accounts[0].0;
        let suri = conn.accounts[0].2.clone();
        let salt = conn.salt;
        let api = conn.api.clone().with(suri.clone()).unwrap().clone();

        let code_vft_manager = conn.code_ids[1];
        let code_historical_proxy = conn.code_ids[2];
        let code_eth_events = conn.code_ids[3];

        let code_vft = &conn.code_ids[4..];
        let vft_accounts = &conn.accounts[4..];

        let api = api.with(suri.clone()).unwrap();

        let remoting = GClientRemoting::new(api.clone());

        let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());

        let vft_manager_id = factory
            .new(InitConfig {
                erc20_manager_address: Default::default(),
                gear_bridge_builtin: Default::default(),
                historical_proxy_address: Default::default(),
                config: Config {
                    gas_for_token_ops: 10_000_000_000,
                    gas_for_reply_deposit: 10_000_000_000,
                    gas_to_send_request_to_builtin: 10_000_000_000,
                    gas_for_swap_token_maps: 1_500_000_000,
                    reply_timeout: 100,
                    fee_bridge: 0,
                    fee_incoming: 0,
                },
            })
            .send_recv(code_vft_manager, salt)
            .await
            .expect("Failed to create VFT Manager");

        let factory = vft_client::VftFactory::new(remoting.clone());
        // for every eth token id, create corresponding VFT

        let mut vft_ids = Vec::new();

        for (i, eth_token_id) in ETH_TOKEN_IDS.iter().enumerate() {
            println!(
                "Creating VFT for ETH token ID: {:?}\nCode ID={:?}",
                eth_token_id, code_vft[i]
            );
            let salt = vft_accounts[i].1;
            let name = format!("TEST_TOKEN_{}", eth_token_id);
            let vft_id = factory
                .new(name.into(), "TT".to_owned(), 20)
                .send_recv(code_vft[i], salt)
                .await
                .expect("Failed to create VFT");

            vft_ids.push(vft_id);

            let mut vft = vft_client::VftAdmin::new(remoting.clone());

            vft.set_minter(vft_manager_id)
                .send_recv(vft_id)
                .await
                .expect("Failed to set minter");

            vft.set_burner(vft_manager_id)
                .send_recv(vft_id)
                .await
                .expect("Failed to set burner");

            let mut vft_extension = vft_client::VftExtension::new(remoting.clone());

            while vft_extension
                .allocate_next_balances_shard()
                .send_recv(vft_id)
                .await
                .expect("Failed to allocate next balances shard")
            {}

            while vft_extension
                .allocate_next_allowances_shard()
                .send_recv(vft_id)
                .await
                .expect("Failed to allocate next allowances shard")
            {}

            let mut service = vft_manager_client::VftManager::new(remoting.clone());

            service
                .map_vara_to_eth_address(vft_id, *eth_token_id, TokenSupply::Ethereum)
                .send_recv(vft_manager_id)
                .await
                .expect("Failed to map VFT to ETH address");
            println!("Mapped VFT {:?} to ETH address {:?}", vft_id, eth_token_id);
        }

        let eth_events = eth_events_electra_client::EthEventsElectraFactory::new(remoting.clone())
            .new(admin)
            .send_recv(code_eth_events, salt)
            .await
            .expect("Failed to create Eth Events Electra");

        let historical_proxy =
            historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
                .new()
                .send_recv(code_historical_proxy, salt)
                .await
                .expect("Failed to create Historical Proxy");

        let mut historical_proxy_client = historical_proxy_client::HistoricalProxy::new(remoting);

        historical_proxy_client
            .add_endpoint(
                TRANSACTIONS.first_key_value().unwrap().1.slot_number - 100,
                eth_events,
            )
            .send_recv(historical_proxy)
            .await
            .expect("Failed to add endpoint to Historical Proxy");

        Contracts {
            admin,
            suri,
            vft_manager: vft_manager_id,
            historical_proxy,
            vft_id: vft_ids,
            eth_events,
        }
    }
}

struct CheckpointMocker(ActorId);

impl CheckpointMocker {
    async fn run(&self, mut listener: EventListener, api: &GearApi) {
        let (source, value, message_id) = listener
            .proc(|event| {
                if let gclient::Event::Gear(gclient::GearEvent::UserMessageSent {
                    message, ..
                }) = event
                {
                    if message.destination.0 == self.0.into_bytes()
                        && message
                            .payload
                            .0
                            .starts_with(service_checkpoint_for::io::Get::ROUTE)
                        && message.details.is_none()
                    {
                        let source = message.source;

                        let params_raw =
                            &message.payload.0[service_checkpoint_for::io::Get::ROUTE.len()..];
                        let params: u64 =
                            Decode::decode(&mut &params_raw[..]).expect("Failed to decode params");

                        return Some((source, params, message.id));
                    }
                }

                None
            })
            .await
            .expect("Failed to process event");

        println!("Received checkpoint request for slot #{value}");

        let txs = &*TRANSACTIONS;

        for tx in txs.iter() {
            if tx.1.slot_number == value {
                let checkpoint = tx.1.checkpoint;
                let root = H256::from(tx.1.checkpoint_root.0);
                let gas_limit = api.block_gas_limit().unwrap();
                println!("checkpoint for slot #{value} is {checkpoint}, root={root:?}");

                let reply: <service_checkpoint_for::io::Get as ActionIo>::Reply =
                    Ok((checkpoint, root));

                let mut bytes = Vec::with_capacity(
                    reply.encoded_size() + service_checkpoint_for::io::Get::ROUTE.len(),
                );

                bytes.extend_from_slice(&service_checkpoint_for::io::Get::ROUTE);
                reply.encode_to(&mut bytes);

                api.send_reply_bytes(message_id.into(), bytes, gas_limit, 0)
                    .await
                    .expect("Failed to send message");
                break;
            }
        }
    }
}
