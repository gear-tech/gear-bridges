// Incorporate code generated based on the IDL file
#[allow(dead_code)]
mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-manager.rs"));
}

use alloy::rpc::types::TransactionReceipt;
use alloy_rlp::Encodable;
use checkpoint_light_client_io::{Handle, HandleResult};
use ethereum_common::{
    beacon::light::Block,
    utils::{self as eth_utils, BeaconBlockHeaderResponse, BeaconBlockResponse, MerkleProof},
};
use futures::StreamExt;
use gclient::{Event, EventListener, EventProcessor, GearApi, GearEvent};
use hex_literal::hex;

use erc20_relay_client::{
    traits::*, BlockInclusionProof, Config as Erc20RelayConfig, Erc20Relay as Erc20RelayC,
    Erc20RelayFactory as Erc20RelayFactoryC, Error as Erc20Error, EthToVaraEvent,
};
use historical_proxy_client::{
    traits::*, Config, HistoricalProxy as HistoricalProxyC,
    HistoricalProxyFactory as HistoricalProxyFactoryC, ProxyError,
};
use sails_rs::{calls::*, events::Listener, gtest::calls::*, prelude::*};
use serde::Deserialize;
use vft::vft_manager;

const HOLESKY_RECEIPTS_2_498_456: &[u8; 160_144] =
    include_bytes!("./holesky-receipts-2_498_456.json");
const HOLESKY_BLOCK_2_498_456: &[u8; 235_397] = include_bytes!("./holesky-block-2_498_456.json");
const HOLESKY_HEADER_2_498_457: &[u8; 670] = include_bytes!("./holesky-header-2_498_457.json");
const HOLESKY_HEADER_2_498_458: &[u8; 669] = include_bytes!("./holesky-header-2_498_458.json");
const HOLESKY_HEADER_2_498_459: &[u8; 670] = include_bytes!("./holesky-header-2_498_459.json");
const HOLESKY_HEADER_2_498_460: &[u8; 670] = include_bytes!("./holesky-header-2_498_460.json");
const HOLESKY_HEADER_2_498_461: &[u8; 670] = include_bytes!("./holesky-header-2_498_461.json");
const HOLESKY_HEADER_2_498_462: &[u8; 669] = include_bytes!("./holesky-header-2_498_462.json");
const HOLESKY_HEADER_2_498_463: &[u8; 670] = include_bytes!("./holesky-header-2_498_463.json");
const HOLESKY_HEADER_2_498_464: &[u8; 669] = include_bytes!("./holesky-header-2_498_464.json");

#[derive(Deserialize)]
pub struct Receipts {
    result: Vec<TransactionReceipt>,
}

use gtest::System;

const ADMIN_ID: u64 = 1_000;
const PROXY_ID: u64 = 1_001;
const ERC20_RELAY_ID: u64 = 1_002;

async fn setup_for_test() -> (GTestRemoting, ActorId, ActorId) {
    println!("init gtest");
    let system = System::new();
    system.init_logger();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);
    system.mint_to(PROXY_ID, 100_000_000_000_000);
    system.mint_to(ERC20_RELAY_ID, 100_000_000_000_000);

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
            ADMIN_ID.into(),
            Erc20RelayConfig {
                reply_timeout: 10_000,
                reply_deposit: 0,
            },
        )
        .send_recv(erc20_id, b"salt")
        .await
        .unwrap();

    (remoting, erc20_relay, proxy)
}

#[test]
fn proxy_gtest() {
    println!("start proxy test");

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // tx 0x180cd2328df9c4356adc77e19e33c5aa2d5395f1b52e70d22c25070a04f16691
            let tx_index = 15;

            let receipts: Receipts =
                serde_json::from_slice(HOLESKY_RECEIPTS_2_498_456.as_ref()).unwrap();
            let receipts = receipts
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

            let block: Block = {
                let response: BeaconBlockResponse =
                    serde_json::from_slice(HOLESKY_BLOCK_2_498_456.as_ref()).unwrap();

                response.data.message.into()
            };
            let headers = vec![
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_457.as_ref()).unwrap();

                    response.data.header.message
                },
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_458.as_ref()).unwrap();

                    response.data.header.message
                },
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_459.as_ref()).unwrap();

                    response.data.header.message
                },
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_460.as_ref()).unwrap();

                    response.data.header.message
                },
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_461.as_ref()).unwrap();

                    response.data.header.message
                },
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_462.as_ref()).unwrap();

                    response.data.header.message
                },
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_463.as_ref()).unwrap();

                    response.data.header.message
                },
                {
                    let response: BeaconBlockHeaderResponse =
                        serde_json::from_slice(HOLESKY_HEADER_2_498_464.as_ref()).unwrap();

                    response.data.header.message
                },
            ];
            println!("ALLOOO");

            let MerkleProof { proof, receipt } =
                eth_utils::generate_merkle_proof(tx_index, &receipts[..]).unwrap();

            let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
            Encodable::encode(&receipt, &mut receipt_rlp);
            let message = EthToVaraEvent {
                proof_block: BlockInclusionProof {
                    block: block.clone(),
                    headers: headers.clone(),
                },
                proof: proof.clone(),
                transaction_index: tx_index,
                receipt_rlp,
            };
            let (mut remoting, erc20_relay, proxy) = setup_for_test().await;
            println!("gtest initialized");
            Erc20RelayC::new(remoting.clone())
                .set_vft_manager(ADMIN_ID.into())
                .send_recv(erc20_relay)
                .await
                .unwrap();

            HistoricalProxyC::new(remoting.clone())
                .add_endpoint(message.proof_block.block.slot, ADMIN_ID.into())
                .send_recv(proxy)
                .await
                .unwrap();
            println!("proxy initialized");

            let mut payload = {
                let mut call =
                    <erc20_relay_client::erc_20_relay::io::CheckProofs as ActionIo>::ROUTE.to_vec();
                message.encode_to(&mut call);
                call
            };
            println!("hello");
            let mut factory = HistoricalProxyC::new(remoting.clone());
            let reply = factory
                .redirect(message.proof_block.block.slot, ADMIN_ID.into(), payload)
                .send(proxy)
                .await
                .unwrap();

            println!("waiting for message");
            let mut stream = remoting.listen().await.unwrap();

            while let Some(message) = stream.next().await {
                println!("{:?}", message);
            }
        });
}
