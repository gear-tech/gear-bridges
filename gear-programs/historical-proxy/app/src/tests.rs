use historical_proxy_client::{
    traits::*, Config, HistoricalProxy as HistoricalProxyC,
    HistoricalProxyFactory as HistoricalProxyFactoryC, ProxyError,
};

use gtest::System;
use sails_rs::{calls::*, gtest::calls::*, prelude::*};

struct Fixture {
    remoting: GTestRemoting,
    proxy: ActorId,
}

const ADMIN_ID: u64 = 1_000;
const USER_ID: u64 = 500;
const PROXY_ID: u64 = 1_001;
const ERC20_RELAY_ID: u64 = 1_002;
const VFT_MANAGER_ID: u64 = 1_003;

async fn setup_for_test() -> Fixture {
    let system = System::new();
    system.init_logger();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);
    system.mint_to(PROXY_ID, 100_000_000_000_000);
    system.mint_to(ERC20_RELAY_ID, 100_000_000_000_000);
    system.mint_to(VFT_MANAGER_ID, 100_000_000_000_000);
    system.mint_to(USER_ID, 100_000_000_000_000);

    let remoting = GTestRemoting::new(system, ADMIN_ID.into());

    let proxy_id = remoting.system().submit_code(historical_proxy::WASM_BINARY);
    let proxy = HistoricalProxyFactoryC::new(remoting.clone())
        .new(Config {
            reply_timeout: 100,
            reply_deposit: 0,
        })
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
        .unwrap()
        .unwrap();

    let recv_endpoint = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(43)
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    assert_eq!(recv_endpoint, Ok(endpoint1.1));

    let recv_endpoint = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(41)
        .send_recv(proxy_program_id)
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
        .unwrap()
        .unwrap();

    let endpoint_for_slot_0 = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(43)
        .send_recv(proxy_program_id)
        .await
        .unwrap();
    assert_eq!(endpoint_for_slot_0, Ok(ActorId::from(0x42)));

    let endpoint_for_slot_1 = HistoricalProxyC::new(remoting.clone())
        .endpoint_for(85)
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    assert_eq!(endpoint_for_slot_1, Ok(ActorId::from(0x800)));
}
