// Incorporate code generated based on the IDL file
#[allow(dead_code)]
mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-manager.rs"));
}

use std::time::Duration;

use alloy::rpc::types::TransactionReceipt;
use alloy_rlp::Encodable;
use checkpoint_light_client_io::{Handle, HandleResult};
use erc20_relay_client::{traits::*, BlockInclusionProof, Config, EthToVaraEvent};
use ethereum_common::{
    beacon::light::Block,
    utils::{self as eth_utils, BeaconBlockHeaderResponse, BeaconBlockResponse, MerkleProof},
};
use gclient::{Event, EventListener, EventProcessor, GearApi, GearEvent};
use hex_literal::hex;
use historical_proxy_client::traits::*;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use serde::Deserialize;
use vft::vft_manager;

mod shared;
use shared::*;

async fn spin_up_node() -> (
    GearApi,
    ActorId,
    GClientRemoting,
    CodeId,
    CodeId,
    GasUnit,
    EventListener,
) {
    let api = if std::env::var("VARA_TESTNET").is_ok() {
        GearApi::vara_testnet().await.unwrap()
    } else {
        GearApi::dev().await.unwrap()
    };
    let id = api.account_id();
    let admin = <[u8; 32]>::from(id.clone());
    let admin = ActorId::from(admin);
    let gas_limit = api.block_gas_limit().unwrap();
    let mut listener = api.subscribe().await.unwrap();
    assert!(listener.blocks_running().await.unwrap());

    let (relay_code_id, _) = api.upload_code(erc20_relay::WASM_BINARY).await.unwrap();

    let (proxy_code_id, _) = api
        .upload_code(historical_proxy::WASM_BINARY)
        .await
        .unwrap();
    let api_ = api.clone();
    let remoting = GClientRemoting::new(api);

    (
        api_,
        admin,
        remoting,
        relay_code_id,
        proxy_code_id,
        gas_limit,
        listener,
    )
}
#[test]
#[ignore = "Requires running node"]
fn proxy() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let message = shared::event();

            let (api, admin, remoting, relay_code_id, proxy_code_id, gas_limit, mut listener) =
                spin_up_node().await;
            println!("node spun up, code uploaded, gas_limit={}", gas_limit);
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
                .send_recv(relay_code_id, gclient::now_micros().to_le_bytes())
                .await
                .unwrap();
            let mut erc20_relay_client = erc20_relay_client::Erc20Relay::new(remoting.clone());
            erc20_relay_client
                .set_vft_manager(admin)
                .with_gas_limit(5_500_000_000)
                .send_recv(erc20_relay_program_id)
                .await
                .unwrap();

            let proxy_program_id =
                historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
                    .new(historical_proxy_client::Config {
                        reply_timeout: 1000,
                        reply_deposit: gas_limit / 100 * 95,
                    })
                    .with_gas_limit(5_500_000_000)
                    .send_recv(proxy_code_id, gclient::now_micros().to_le_bytes())
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
            println!(
                "endpoint {:?}\nproxy: {:?}\nadmin: {:?}",
                endpoint, proxy_program_id, admin
            );
            let route = <erc20_relay_client::erc_20_relay::io::CheckProofs>::ROUTE;
            let mut call = route.to_vec();
            call.extend_from_slice(&message.encode());
            let result = proxy_client
                .redirect(message.proof_block.block.slot, admin, call)
                .with_gas_limit(gas_limit / 100 * 95)
                .send(proxy_program_id)
                .await
                .unwrap();
            let message_id = listener
                .proc(|e| match e {
                    Event::Gear(GearEvent::UserMessageSent { message, .. })
                        if message.destination == admin.into() && message.details.is_none() =>
                    {
                        let request = Handle::decode(&mut &message.payload.0[..]).ok()?;

                        match request {
                            Handle::GetCheckpointFor { slot } if slot == 2_498_456 => {
                                println!("get checkpoint for: #{}", slot);
                                Some(message.id)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                })
                .await
                .unwrap();
            println!("CHECKPOINT!");
            let reply = HandleResult::Checkpoint(Ok((
                2_496_464,
                hex!("b89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1").into(),
            )));
            let block_gas_limit = api.block_gas_limit().unwrap();
            let (message_id, _, _) = api.send_reply(message_id.into(), reply, block_gas_limit / 100 * 95, 0)
                .await
                .unwrap();
            println!("CHECKPOINT REPLY WITH ID {:?} SENT", message_id);
            assert!(listener.message_processed(message_id).await.unwrap().succeed());
            
            // wait for SubmitReceipt request and reply to it
            let message_id = listener
                .proc(|e| match e {
                    Event::Gear(GearEvent::UserMessageSent { message, .. })
                        if message.destination == admin.into() && message.details.is_none() =>
                    {
                        message
                            .payload
                            .0
                            .starts_with(vft_manager::io::SubmitReceipt::ROUTE)
                            .then_some(message.id)
                            .or_else(|| crate::panic!())
                    }
                    Event::Gear(GearEvent::UserMessageSent { message, .. }) => {
                        println!(
                            "{:?} -> {:?}: received {:?}",
                            message.source,
                            message.destination,
                            std::string::String::from_utf8_lossy(&message.payload.0)
                        );

                        None
                    }
                    _ => None,
                })
                .await
                .unwrap();
            
            println!("Submit receipt request");
            let reply: <vft_manager::io::SubmitReceipt as ActionIo>::Reply = Ok(());
            let payload = {
                let mut result = route.to_vec();
                reply.encode_to(&mut result);
                result
            };

            api.send_reply_bytes(message_id.into(), payload, 0, 0)
                .await
                .unwrap();

            let _result = result.recv().await.unwrap();
            println!("wohoo");
        });
}
