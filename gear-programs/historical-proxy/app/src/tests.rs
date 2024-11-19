use erc20_relay_client::{
    traits::*, BlockInclusionProof, Config as Erc20RelayConfig, Erc20Relay as Erc20RelayC, Erc20RelayFactory as Erc20RelayFactoryC, Error as Erc20Error, EthToVaraEvent
};
use ethereum_common::beacon::light::{Block, BlockBody};
use historical_proxy_client::{
    traits::*, Config, HistoricalProxy as HistoricalProxyC,
    HistoricalProxyFactory as HistoricalProxyFactoryC, ProxyError,
};
use vft_manager_client::{
    traits::*, Config as VftManagerConfig, VftManager as VftManagerC, VftManagerError,
    VftManagerFactory as VftManagerFactoryC,
};

use gtest::System;
use sails_rs::{calls::*, gtest::calls::*, prelude::*};

struct Fixture {
    remoting: GTestRemoting,
    proxy: ActorId,
    erc20_relay: ActorId,
    vft_manager: ActorId,
}

const ADMIN_ID: u64 = 1_000;
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

    let erc20_id = remoting.system().submit_code(erc20_relay::WASM_BINARY);
    let erc20_relay = Erc20RelayFactoryC::new(remoting.clone())
        .new(
            Default::default(),
            Erc20Config {
                reply_timeout: 10_000,
                reply_deposit: 0,
            },
        )
        .send_recv(erc20_id, b"salt")
        .await
        .unwrap();

    let vft_manager_id = remoting.system().submit_code(vft_manager::WASM_BINARY);
    let vft_manager = VftManagerFactoryC::new(remoting.clone())
        .new(VftManagerConfig {
            gas_for_token_ops: 15_000_000_000,
            gas_for_reply_deposit: 15_000_000_000,
            gas_for_submit_receipt: 20_000_000_000,
            gas_to_send_request_to_builtin: 15_000_000_000,
            reply_timeout: 100,
            gas_for_request_bridging: 20_000_000_000,
        })
        .send_recv(vft_manager_id, b"salt")
        .await
        .unwrap();

    Fixture {
        remoting,
        proxy,
        erc20_relay,
        vft_manager,
    }
}

#[test]
fn test_utility_functions() {
    /*  #[tokio::test] proc macro panics in rust-analyzer, use this locally for ease of use.
       will update once ready for merge
    */
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let Fixture {
                remoting,
                proxy: proxy_progam_rid,
                erc20_relay,
                vft_manager,
            } = setup_for_test().await;

            let admin_id = HistoricalProxyC::new(remoting.clone())
                .admin()
                .recv(proxy_program_id)
                .await
                .unwrap();

            assert_eq!(admin_id, ActorId::from(ADMIN_ID));

            let endpoint1 = (0, ActorId::from(0x42));

            HistoricalProxyC::new(remoting.clone())
                .add_endpoint(0, ActorId::from(0x42))
                .send(proxy_program_id)
                .await
                .unwrap();

            let recv_endpoint = HistoricalProxyC::new(remoting.clone())
                .endpoint_for(0)
                .send_recv(proxy_program_id)
                .await
                .unwrap();

            assert_eq!(recv_endpoint, Ok(endpoint1.1));

            let recv_endpoint = HistoricalProxyC::new(remoting.clone())
                .endpoint_for(42)
                .send_recv(proxy_program_id)
                .await
                .unwrap();

            assert_eq!(recv_endpoint, Err(ProxyError::NoEndpointForSlot(42)));

            let endpoints = HistoricalProxyC::new(remoting.clone())
                .endpoints()
                .recv(proxy_program_id)
                .await
                .unwrap();

            assert!(!endpoints.is_empty());
            assert_eq!(endpoints[0], endpoint1);

            let _endpoint2 = (10, ActorId::from(0x800));

            HistoricalProxyC::new(remoting.clone())
                .add_endpoint(10, ActorId::from(0x800))
                .send_recv(proxy_program_id)
                .await
                .unwrap();

            let endpoint_for_slot_0 = HistoricalProxyC::new(remoting.clone())
                .endpoint_for(0)
                .send_recv(proxy_program_id)
                .await
                .unwrap();
            assert_eq!(endpoint_for_slot_0, Ok(ActorId::from(0x42)));

            let endpoint_for_slot_1 = HistoricalProxyC::new(remoting.clone())
                .endpoint_for(1)
                .send_recv(proxy_program_id)
                .await
                .unwrap();

            assert_eq!(endpoint_for_slot_1, Ok(ActorId::from(0x800)));
        });
}

#[test]
fn test_proxy() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let Fixture {
                remoting,
                proxy: proxy_progam_rid,
                erc20_relay,
                vft_manager,
            } = setup_for_test().await;

            let admin_id = HistoricalProxyC::new(remoting.clone())
                .admin()
                .recv(proxy_program_id)
                .await
                .unwrap();

            assert_eq!(admin_id, ActorId::from(ADMIN_ID));

            HistoricalProxyC::new(remoting.clone())
                .add_endpoint(0, erc20_relay.clone())
                .send_recv(proxy_progam_rid)
                .await
                .unwrap();

            // TODO
        });
}
