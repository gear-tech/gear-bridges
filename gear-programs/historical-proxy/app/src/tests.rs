extern crate alloc;

use alloc::rc::Rc;
use core::cell::Cell;

use historical_proxy_client::{
    traits::*, HistoricalProxy as HistoricalProxyC,
    HistoricalProxyFactory as HistoricalProxyFactoryC, ProxyError,
};

use gtest::{Program, System, WasmProgram};
use sails_rs::{calls::*, gtest::calls::*, prelude::*};

#[derive(Clone, Debug)]
struct EthereumEventClientMock {
    slot: u64,
}

impl WasmProgram for EthereumEventClientMock {
    fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        let reply: Result<_, eth_events_common::Error> = Ok(eth_events_common::CheckedProofs {
            receipt_rlp: Vec::new(),
            transaction_index: 0,
            block_number: 0,
            slot: self.slot,
        });
        let mut payload =
            eth_events_electra_client::ethereum_event_client::io::CheckProofs::ROUTE.to_vec();
        reply.encode_to(&mut payload);
        Ok(Some(payload))
    }

    fn clone_boxed(&self) -> Box<dyn WasmProgram> {
        Box::new(self.clone())
    }

    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Debug)]
struct VftManagerMock(Rc<Cell<u32>>);

impl WasmProgram for VftManagerMock {
    fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        self.0.set(self.0.get() + 1);
        Err("unexpected VFT-manager dispatch")
    }

    fn clone_boxed(&self) -> Box<dyn WasmProgram> {
        Box::new(self.clone())
    }

    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        Ok(Vec::new())
    }
}
struct Fixture {
    remoting: GTestRemoting,
    proxy: ActorId,
}

const ADMIN_ID: u64 = 1_000;
const USER_ID: u64 = 500;
const PROXY_ID: u64 = 1_001;
const ETHEREUM_EVENT_CLIENT_ID: u64 = 1_002;
const VFT_MANAGER_ID: u64 = 1_003;

async fn setup_for_test() -> Fixture {
    let system = System::new();
    system.init_logger();
    system.mint_to(ADMIN_ID, 100_000_000_000_000_000);
    system.mint_to(PROXY_ID, 100_000_000_000_000_000);
    system.mint_to(ETHEREUM_EVENT_CLIENT_ID, 100_000_000_000_000_000);
    system.mint_to(VFT_MANAGER_ID, 100_000_000_000_000_000);
    system.mint_to(USER_ID, 100_000_000_000_000_000);

    let remoting = GTestRemoting::new(system, ADMIN_ID.into());

    let proxy_id = remoting.system().submit_code(historical_proxy::WASM_BINARY);
    let proxy = HistoricalProxyFactoryC::new(remoting.clone())
        .new()
        .send_recv(proxy_id, b"salt")
        .await
        .unwrap();

    Fixture { remoting, proxy }
}

#[tokio::test]
async fn test_utility_functions() {
    let Fixture {
        remoting,
        proxy: proxy_program_id,
    } = setup_for_test().await;

    let admin_id = HistoricalProxyC::new(remoting.clone())
        .admin()
        .recv(proxy_program_id)
        .await
        .unwrap();

    assert_eq!(admin_id, ActorId::from(ADMIN_ID));

    let endpoint1 = (42, ActorId::from(0x42));

    HistoricalProxyC::new(remoting.clone())
        .add_endpoint(42, ActorId::from(0x42))
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    let recv_endpoint = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(43)
        .recv(proxy_program_id)
        .await
        .unwrap();

    assert_eq!(recv_endpoint, Ok(endpoint1.1));

    let recv_endpoint = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(41)
        .recv(proxy_program_id)
        .await
        .unwrap();

    assert_eq!(recv_endpoint, Err(ProxyError::NoEndpointForSlot(41)));

    let endpoints = HistoricalProxyC::new(remoting.clone())
        .endpoints()
        .recv(proxy_program_id)
        .await
        .unwrap();

    assert!(!endpoints.is_empty());
    assert_eq!(endpoints[0], endpoint1);

    let _endpoint2 = (10, ActorId::from(0x800));

    HistoricalProxyC::new(remoting.clone())
        .add_endpoint(84, ActorId::from(0x800))
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    let endpoint_for_slot_0 = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(43)
        .recv(proxy_program_id)
        .await
        .unwrap();
    assert_eq!(endpoint_for_slot_0, Ok(ActorId::from(0x42)));

    let endpoint_for_slot_1 = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(85)
        .recv(proxy_program_id)
        .await
        .unwrap();

    assert_eq!(endpoint_for_slot_1, Ok(ActorId::from(0x800)));
}

#[tokio::test]
async fn test_client_route_validation() {
    let Fixture {
        remoting,
        proxy: proxy_program_id,
    } = setup_for_test().await;

    let route = ("VftManager".to_owned(), "SubmitReceipt".to_owned()).encode();
    let result = HistoricalProxyC::new(remoting.clone())
        .redirect(42, Vec::new(), VFT_MANAGER_ID.into(), route.clone())
        .send_recv(proxy_program_id)
        .await
        .unwrap();
    assert_eq!(result, Err(ProxyError::NoEndpointForSlot(42)));
    let endpoint = Program::mock_with_id(
        remoting.system(),
        ETHEREUM_EVENT_CLIENT_ID,
        EthereumEventClientMock { slot: 42 },
    );
    let _ = endpoint.send_bytes(ADMIN_ID, b"INIT");

    let vft_manager_calls = Rc::new(Cell::new(0));
    let vft_manager = Program::mock_with_id(
        remoting.system(),
        VFT_MANAGER_ID,
        VftManagerMock(vft_manager_calls.clone()),
    );
    let _ = vft_manager.send_bytes(ADMIN_ID, b"INIT");

    HistoricalProxyC::new(remoting.clone())
        .add_endpoint(42, ETHEREUM_EVENT_CLIENT_ID.into())
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    let mut smuggled_route = route;
    (1u64, 2u64, vec![3u8]).encode_to(&mut smuggled_route);
    let result = HistoricalProxyC::new(remoting.clone())
        .redirect(42, Vec::new(), VFT_MANAGER_ID.into(), smuggled_route)
        .send_recv(proxy_program_id)
        .await
        .unwrap();
    assert!(matches!(
        result,
        Err(ProxyError::DecodeFailure(message))
            if message.contains("trailing bytes")
    ));
    assert_eq!(vft_manager_calls.get(), 0);

    let result = HistoricalProxyC::new(remoting)
        .redirect(42, Vec::new(), VFT_MANAGER_ID.into(), vec![0xff])
        .send_recv(proxy_program_id)
        .await
        .unwrap();
    assert!(matches!(result, Err(ProxyError::DecodeFailure(_))));
    assert_eq!(vft_manager_calls.get(), 0);
}

#[tokio::test]
async fn test_rejects_verified_slot_for_another_endpoint() {
    let Fixture {
        remoting,
        proxy: proxy_program_id,
    } = setup_for_test().await;
    let endpoint = Program::mock_with_id(
        remoting.system(),
        ETHEREUM_EVENT_CLIENT_ID,
        EthereumEventClientMock { slot: 84 },
    );
    let _ = endpoint.send_bytes(ADMIN_ID, b"INIT");

    HistoricalProxyC::new(remoting.clone())
        .add_endpoint(42, ETHEREUM_EVENT_CLIENT_ID.into())
        .send_recv(proxy_program_id)
        .await
        .unwrap();
    HistoricalProxyC::new(remoting.clone())
        .add_endpoint(84, VFT_MANAGER_ID.into())
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    let result = HistoricalProxyC::new(remoting)
        .redirect(
            42,
            Vec::new(),
            USER_ID.into(),
            ("Client".to_owned(), "Method".to_owned()).encode(),
        )
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    assert!(
        matches!(
            &result,
            Err(ProxyError::DecodeFailure(message))
                if message.contains("outside the selected endpoint range")
        ),
        "unexpected result: {result:?}"
    );
}

#[test]
fn test_routes_eq() {
    assert_eq!(
        eth_events_deneb_client::ethereum_event_client::io::CheckpointLightClientAddress::ROUTE,
        eth_events_electra_client::ethereum_event_client::io::CheckpointLightClientAddress::ROUTE
    );
    assert_eq!(
        eth_events_deneb_client::ethereum_event_client::io::CheckProofs::ROUTE,
        eth_events_electra_client::ethereum_event_client::io::CheckProofs::ROUTE
    );
}

#[path = "../../../../tests/src/historical_proxy/shared.rs"]
mod receipt_fixture;

#[derive(Clone, Debug)]
struct AuthenticatedCheckpointFixture {
    slot: u64,
    root: H256,
}

impl WasmProgram for AuthenticatedCheckpointFixture {
    fn init(&mut self, _: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }
    fn handle(&mut self, payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        let (service, method) = <(String, String)>::decode(&mut &payload[..]).unwrap();
        let mut reply = (service.clone(), method.clone()).encode();
        match (service.as_str(), method.as_str()) {
            ("ServiceState", "Network") => {
                ethereum_common::network::Network::Holesky.encode_to(&mut reply)
            }
            ("ServiceCheckpointFor", "Get") => {
                let result: Result<_, checkpoint_light_client_client::CheckpointError> =
                    Ok((self.slot, self.root));
                result.encode_to(&mut reply);
            }
            _ => panic!("unexpected checkpoint route"),
        }
        Ok(Some(reply))
    }
    fn clone_boxed(&self) -> Box<dyn WasmProgram> {
        Box::new(self.clone())
    }
    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn receipt_verifier_wasm_rejects_mutated_historical_proofs() {
    use alloy_rlp::{Decodable, Encodable};
    use eth_events_deneb_client::{
        traits::{EthEventsDenebFactory as _, EthereumEventClient as _},
        EthEventsDenebFactory, EthereumEventClient,
    };
    use ethereum_common::tree_hash::TreeHash;
    let Fixture { remoting, proxy: _ } = setup_for_test().await;
    let original = receipt_fixture::event();
    let checkpoint = original.proof_block.headers.last().unwrap();
    let checkpoint_program = Program::mock_with_id(
        remoting.system(),
        ETHEREUM_EVENT_CLIENT_ID,
        AuthenticatedCheckpointFixture {
            slot: checkpoint.slot,
            root: checkpoint.tree_hash_root(),
        },
    );
    checkpoint_program.send_bytes(ADMIN_ID, b"INIT");
    let code = remoting.system().submit_code(eth_events_deneb::WASM_BINARY);
    let endpoint = EthEventsDenebFactory::new(remoting.clone())
        .new(ETHEREUM_EVENT_CLIENT_ID.into())
        .send_recv(code, b"receipt-verifier")
        .await
        .unwrap();
    let checked = EthereumEventClient::new(remoting.clone())
        .check_proofs(original.clone())
        .send_recv(endpoint)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checked.slot, original.proof_block.block.slot);
    assert_eq!(checked.receipt_rlp, original.receipt_rlp);
    for mutation in [
        "reverse",
        "duplicate",
        "before receipt",
        "after checkpoint",
        "missing endpoint",
        "empty ancestry",
        "bad parent",
        "bad receipt proof",
        "RLP trailing",
        "failed status",
        "post-state status",
    ] {
        let mut event = original.clone();
        match mutation {
            "reverse" => event.proof_block.headers.reverse(),
            "duplicate" => event
                .proof_block
                .headers
                .insert(1, event.proof_block.headers[0].clone()),
            "before receipt" => event.proof_block.headers[0].slot = event.proof_block.block.slot,
            "after checkpoint" => event.proof_block.headers.last_mut().unwrap().slot += 1,
            "missing endpoint" => {
                event.proof_block.headers.pop();
            }
            "empty ancestry" => event.proof_block.headers.clear(),
            "bad parent" => event.proof_block.headers[0].parent_root = H256::repeat_byte(9),
            "bad receipt proof" => {
                event.proof[0][0] ^= 1;
            }
            "RLP trailing" => event.receipt_rlp.push(0),
            "failed status" | "post-state status" => {
                let mut receipt =
                    ethereum_common::utils::ReceiptEnvelope::decode(&mut &event.receipt_rlp[..])
                        .unwrap();
                receipt.as_receipt_with_bloom_mut().unwrap().receipt.status =
                    if mutation == "failed status" {
                        false.into()
                    } else {
                        alloy::primitives::B256::ZERO.into()
                    };
                event.receipt_rlp.clear();
                receipt.encode(&mut event.receipt_rlp);
            }
            _ => unreachable!(),
        }
        let result = EthereumEventClient::new(remoting.clone())
            .check_proofs(event)
            .send_recv(endpoint)
            .await
            .unwrap();
        assert!(result.is_err(), "accepted mutation {mutation}");
    }
    let mut payload =
        eth_events_deneb_client::ethereum_event_client::io::CheckProofs::ROUTE.to_vec();
    original.encode_to(&mut payload);
    payload.push(0);
    let reply = remoting
        .clone()
        .message(endpoint, payload, None, 0, GTestArgs::default())
        .await
        .unwrap()
        .await;
    assert!(reply.is_err(), "accepted trailing SCALE proof bytes");

    let code = remoting.system().submit_code(eth_events_deneb::WASM_BINARY);
    let receipt_slot = original.proof_block.block.slot;
    let receipt_root = ethereum_common::beacon::BlockHeader {
        slot: receipt_slot,
        proposer_index: original.proof_block.block.proposer_index,
        parent_root: original.proof_block.block.parent_root,
        state_root: original.proof_block.block.state_root,
        body_root: original.proof_block.block.body.tree_hash_root(),
    }
    .tree_hash_root();
    let same_checkpoint = Program::mock_with_id(
        remoting.system(),
        2002,
        AuthenticatedCheckpointFixture {
            slot: receipt_slot,
            root: receipt_root,
        },
    );
    same_checkpoint.send_bytes(ADMIN_ID, b"INIT");
    let same_endpoint = EthEventsDenebFactory::new(remoting.clone())
        .new(2002.into())
        .send_recv(code, b"same-slot")
        .await
        .unwrap();
    let mut same_event = original;
    same_event.proof_block.headers.clear();
    assert!(EthereumEventClient::new(remoting)
        .check_proofs(same_event)
        .send_recv(same_endpoint)
        .await
        .unwrap()
        .is_ok());
}

#[tokio::test]
async fn electra_and_fulu_receipt_wasm_bind_network_schedule() {
    extern crate std;
    use alloy_rlp::Encodable;
    use eth_events_electra_client::{
        traits::{EthEventsElectraFactory as _, EthereumEventClient as _},
        BlockGenericForBlockBody, BlockInclusionProof, EthEventsElectraFactory, EthToVaraEvent,
        EthereumEventClient,
    };
    use ethereum_common::{beacon, tree_hash::TreeHash, utils};
    use std::io::Read;
    #[derive(serde::Deserialize)]
    struct Receipts {
        result: Vec<alloy::rpc::types::TransactionReceipt>,
    }
    #[derive(serde::Deserialize)]
    struct ReceiptFixture {
        tx_index: u64,
        receipts: Receipts,
        block: utils::BeaconBlockResponse<beacon::electra::Block>,
    }
    let mut bytes = Vec::new();
    ruzstd::StreamingDecoder::new(
        &include_bytes!("../../../../tests/src/relayer/transactions.json.zst")[..],
    )
    .unwrap()
    .read_to_end(&mut bytes)
    .unwrap();
    let fixture: ReceiptFixture = serde_json::from_slice::<Vec<ReceiptFixture>>(&bytes)
        .unwrap()
        .remove(0);
    let receipts: Vec<_> = fixture
        .receipts
        .result
        .iter()
        .map(|receipt| {
            (
                receipt.transaction_index.unwrap(),
                utils::map_receipt_envelope(receipt.as_ref()),
            )
        })
        .collect();
    let proof = utils::generate_merkle_proof(fixture.tx_index, &receipts).unwrap();
    let mut receipt_rlp = Vec::new();
    proof.receipt.encode(&mut receipt_rlp);
    let Fixture { remoting, proxy: _ } = setup_for_test().await;
    let code = remoting
        .system()
        .submit_code(eth_events_electra::WASM_BINARY);
    let network = ethereum_common::network::Network::Holesky;
    let actual_slot = fixture.block.data.message.slot;
    assert!(actual_slot / 32 >= network.epoch_electra());
    for (index, (slot, accepted)) in [
        (actual_slot, true),
        (network.epoch_fulu() * 32 - 1, true),
        (network.epoch_fulu() * 32, true),
        (network.epoch_electra() * 32 - 1, false),
        (network.epoch_deneb() * 32 - 1, false),
    ]
    .into_iter()
    .enumerate()
    {
        // Changed slots are explicit synthetic fork-boundary cases, not historical finality evidence.
        let mut block = fixture.block.data.message.clone();
        block.slot = slot;
        let checkpoint_id = 3000u64 + index as u64;
        let checkpoint = Program::mock_with_id(
            remoting.system(),
            checkpoint_id,
            AuthenticatedCheckpointFixture {
                slot,
                root: block.tree_hash_root(),
            },
        );
        checkpoint.send_bytes(ADMIN_ID, b"INIT");
        let endpoint = EthEventsElectraFactory::new(remoting.clone())
            .new(checkpoint_id.into())
            .send_recv(code, slot.to_le_bytes())
            .await
            .unwrap();
        let event = EthToVaraEvent {
            proof_block: BlockInclusionProof {
                block: BlockGenericForBlockBody {
                    slot,
                    proposer_index: block.proposer_index,
                    parent_root: block.parent_root,
                    state_root: block.state_root,
                    body: block.body.into(),
                },
                headers: Vec::new(),
            },
            proof: proof.proof.clone(),
            transaction_index: fixture.tx_index,
            receipt_rlp: receipt_rlp.clone(),
        };
        let result = EthereumEventClient::new(remoting.clone())
            .check_proofs(event)
            .send_recv(endpoint)
            .await
            .unwrap();
        if accepted {
            assert!(result.is_ok(), "rejected supported slot {slot}: {result:?}");
        } else {
            assert!(matches!(
                result,
                Err(eth_events_electra_client::Error::UnsupportedFork)
            ));
        }
    }
}

#[derive(Clone, Debug)]
struct RawReplyFixture(Vec<u8>);
impl WasmProgram for RawReplyFixture {
    fn init(&mut self, _: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }
    fn handle(&mut self, _: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(Some(self.0.clone()))
    }
    fn clone_boxed(&self) -> Box<dyn WasmProgram> {
        Box::new(self.clone())
    }
    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn proxy_preserves_inner_error_and_rejects_trailing_verifier_reply() {
    let Fixture { remoting, proxy } = setup_for_test().await;
    let endpoint = Program::mock_with_id(
        remoting.system(),
        ETHEREUM_EVENT_CLIENT_ID,
        EthereumEventClientMock { slot: 42 },
    );
    endpoint.send_bytes(ADMIN_ID, b"INIT");
    let route = ("Application".to_owned(), "SubmitReceipt".to_owned());
    let mut consumer_reply = route.encode();
    Result::<(), u8>::Err(7).encode_to(&mut consumer_reply);
    let consumer = Program::mock_with_id(
        remoting.system(),
        VFT_MANAGER_ID,
        RawReplyFixture(consumer_reply.clone()),
    );
    consumer.send_bytes(ADMIN_ID, b"INIT");
    HistoricalProxyC::new(remoting.clone())
        .add_endpoint(42, ETHEREUM_EVENT_CLIENT_ID.into())
        .send_recv(proxy)
        .await
        .unwrap();
    let (_, reply) = HistoricalProxyC::new(remoting.clone())
        .redirect(42, Vec::new(), VFT_MANAGER_ID.into(), route.encode())
        .send_recv(proxy)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, consumer_reply);
    let mut input = &reply[..];
    assert_eq!(<(String, String)>::decode(&mut input).unwrap(), route);
    assert_eq!(Result::<(), u8>::decode(&mut input).unwrap(), Err(7));
    assert!(input.is_empty());
    // A different slot in the same endpoint interval must not relabel the authenticated receipt.
    let result = HistoricalProxyC::new(remoting.clone())
        .redirect(43, Vec::new(), VFT_MANAGER_ID.into(), route.encode())
        .send_recv(proxy)
        .await
        .unwrap();
    assert!(matches!(result, Err(ProxyError::DecodeFailure(_))));
    let mut invalid_reply = EthereumEventClientMock { slot: 84 }
        .handle(Vec::new())
        .unwrap()
        .unwrap();
    invalid_reply.push(0);
    let malformed = Program::mock_with_id(remoting.system(), 4002, RawReplyFixture(invalid_reply));
    malformed.send_bytes(ADMIN_ID, b"INIT");
    HistoricalProxyC::new(remoting.clone())
        .add_endpoint(84, 4002.into())
        .send_recv(proxy)
        .await
        .unwrap();
    let result = HistoricalProxyC::new(remoting)
        .redirect(84, Vec::new(), VFT_MANAGER_ID.into(), route.encode())
        .send_recv(proxy)
        .await
        .unwrap();
    assert!(
        matches!(result, Err(ProxyError::DecodeFailure(message)) if message.contains("trailing bytes"))
    );
}
