// Incorporate code generated based on the IDL file
#[allow(dead_code)]
mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-manager.rs"));
}

use alloy::rpc::types::TransactionReceipt;
use alloy_rlp::Encodable;
use checkpoint_light_client_io::{Handle, HandleResult};
use erc20_relay_client::{traits::*, BlockInclusionProof, Config, EthToVaraEvent};
use ethereum_common::{
    beacon::light::Block,
    utils::{self as eth_utils, BeaconBlockHeaderResponse, BeaconBlockResponse, MerkleProof},
};

use gtest::System;
use hex_literal::hex;
use historical_proxy_client::traits::*;
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use serde::Deserialize;
use vft::vft_manager;

mod shared;

const ADMIN_ID: u64 = 1000;
const PROXY_ID: u64 = 1001;
const RELAY_ID: u64 = 1002;

async fn setup_for_test() -> (GTestRemoting, CodeId, CodeId) {
    let mut system = System::new();
    system.init_logger();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);
    system.mint_to(PROXY_ID, 100_000_000_000_000);
    system.mint_to(RELAY_ID, 100_000_000_000_000);

    let proxy_code = system.submit_code(historical_proxy::WASM_BINARY);
    let relay_code = system.submit_code(erc20_relay::WASM_BINARY);

    let remoting = GTestRemoting::new(system, ADMIN_ID.into());

    (remoting, proxy_code, relay_code)
}

#[tokio::test]
async fn proxy() {
    let message = shared::event();

    let (mut remoting, proxy_code_id, relay_code_id) = setup_for_test().await;
    println!("node spun up, code uploaded");
    let admin: ActorId = ADMIN_ID.into();
    let gas_limit = 100_000_000_000;
    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());
    let erc20_relay_program_id = factory
        .new(
            admin,
            Config {
                reply_timeout: 1_000,
                reply_deposit: 5_500_000_000,
            },
        )
        .with_gas_limit(gas_limit)
        .send_recv(relay_code_id, b"salt")
        .await
        .unwrap();
    let mut erc20_relay_client = erc20_relay_client::Erc20Relay::new(remoting.clone());
    erc20_relay_client
        .set_vft_manager(admin)
        .with_gas_limit(gas_limit)
        .send_recv(erc20_relay_program_id)
        .await
        .unwrap();

    let proxy_program_id = historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
        .new(historical_proxy_client::Config {
            reply_timeout: 1000,
            reply_deposit: 5_500_000_000,
        })
        .with_gas_limit(gas_limit)
        .send_recv(proxy_code_id, b"salt")
        .await
        .unwrap();
    println!("relay and proxy programs created");
    let mut proxy_client = historical_proxy_client::HistoricalProxy::new(remoting.clone());

    proxy_client
        .add_endpoint(message.proof_block.block.slot, erc20_relay_program_id)
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    let endpoint = proxy_client
        .endpoint_for(message.proof_block.block.slot)
        .send_recv(proxy_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(endpoint, erc20_relay_program_id);
    println!("endpoint {:?}", endpoint);
    let route = <erc20_relay_client::erc_20_relay::io::CheckProofs>::ROUTE;
    let mut call = route.to_vec();
    message.encode_to(&mut call);
    let result = proxy_client
        .redirect(message.proof_block.block.slot, admin, call)
        .send(proxy_program_id)
        .await
        .unwrap();

    let run_result = remoting.system().run_next_block();
    for message in run_result.log {
        if message.destination() == ADMIN_ID.into() {
            
            println!("hello!");
        }
    }
}
