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
