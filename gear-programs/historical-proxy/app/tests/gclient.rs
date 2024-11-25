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
use gclient::{Event, EventListener, EventProcessor, GearApi, GearEvent};
use hex_literal::hex;
use historical_proxy_client::traits::*;
use sails_rs::{calls::*, events::Listener, gclient::calls::*, prelude::*};
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

async fn spin_up_node() -> (
    GearApi,
    ActorId,
    GClientRemoting,
    CodeId,
    CodeId,
    GasUnit,
    EventListener,
) {
    let api = GearApi::dev().await.unwrap();
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

            let (api, admin, remoting, relay_code_id, proxy_code_id, gas_limit, mut listener) =
                spin_up_node().await;

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
                .with_gas_limit(gas_limit)
                .send_recv(erc20_relay_program_id)
                .await
                .unwrap();

            let proxy_program_id =
                historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
                    .new(historical_proxy_client::Config {
                        reply_timeout: 1000,
                        reply_deposit: 5_500_000_000,
                    })
                    .with_gas_limit(gas_limit)
                    .send_recv(proxy_code_id, gclient::now_micros().to_le_bytes())
                    .await
                    .unwrap();
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
            call.extend_from_slice(&message.encode());
            let result = proxy_client
                .redirect(message.proof_block.block.slot, admin, call)
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

            api.send_reply(message_id.into(), reply, gas_limit, 0)
                .await
                .unwrap();

            // wait for SubmitReceipt request and reply to it
            let (source, message_id) = listener
                .proc(|e| match e {
                    Event::Gear(GearEvent::UserMessageSent { message, .. })
                        if message.destination == admin.into() && message.details.is_none() =>
                    {
                        message
                            .payload
                            .0
                            .starts_with(vft_manager::io::SubmitReceipt::ROUTE)
                            .then_some((message.source, message.id))
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

            remoting
                .clone()
                .message(
                    source.into(),
                    payload,
                    Some(gas_limit),
                    0,
                    GClientArgs::default(),
                )
                .await
                .unwrap()
                .await
                .unwrap();

            let result = result.recv().await.unwrap();
            println!("wohoo");
        });
}

/*
#[tokio::test]
#[ignore = "Requires running node"]
async fn gas_for_reply() {
    use erc20_relay_client::{traits::Erc20Relay as _, Erc20Relay, Erc20RelayFactory};

    let route = <vft_gateway::io::MintTokens as ActionIo>::ROUTE;

    let (remoting, code_id, gas_limit) = spin_up_node().await;
    let account_id: ActorId = <[u8; 32]>::from(remoting.api().account_id().clone()).into();

    let factory = Erc20RelayFactory::new(remoting.clone());

    let program_id = factory
        .gas_calculation(1_000, 5_500_000_000)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .unwrap();

    let mut client = Erc20Relay::new(remoting.clone());
    while client
        .fill_transactions()
        .send_recv(program_id)
        .await
        .unwrap()
    {}

    println!("prepared");

    for i in 5..10 {
        let mut listener = remoting.api().subscribe().await.unwrap();

        client
            .calculate_gas_for_reply(i, i)
            .with_gas_limit(10_000_000_000)
            .send(program_id)
            .await
            .unwrap();

        let message_id = listener
            .proc(|e| match e {
                Event::Gear(GearEvent::UserMessageSent { message, .. })
                    if message.destination == account_id.into() && message.details.is_none() =>
                {
                    message.payload.0.starts_with(route).then_some(message.id)
                }
                _ => None,
            })
            .await
            .unwrap();

        println!("message_id = {}", hex::encode(message_id.0.as_ref()));

        let reply: <vft_gateway::io::MintTokens as ActionIo>::Reply = Ok(());
        let payload = {
            let mut result = Vec::with_capacity(route.len() + reply.encoded_size());
            result.extend_from_slice(route);
            reply.encode_to(&mut result);

            result
        };
        let gas_info = remoting
            .api()
            .calculate_reply_gas(None, message_id.into(), payload, 0, true)
            .await
            .unwrap();

        println!("gas_info = {gas_info:?}");
    }
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn set_vft_gateway() {
    let (remoting, code_id, gas_limit) = spin_up_node().await;

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());

    let program_id = factory
        .new(
            Default::default(),
            Default::default(),
            Config {
                reply_timeout: 10_000,
                reply_deposit: 1_000_000_000,
            },
        )
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .unwrap();

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());

    // by default address of the VFT gateway is not set
    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, Default::default());

    let vft_gateway_new = ActorId::from([1u8; 32]);

    // admin should be able to set the VFT gateway address
    client
        .set_vft_gateway(vft_gateway_new)
        .send_recv(program_id)
        .await
        .unwrap();

    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, vft_gateway_new);

    // and reset it
    client
        .set_vft_gateway(Default::default())
        .send_recv(program_id)
        .await
        .unwrap();

    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, Default::default());

    // another account isn't permitted to change the VFT gateway address
    let api = GearApi::dev().await.unwrap().with("//Bob").unwrap();
    let remoting = GClientRemoting::new(api);

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());
    let result = client
        .set_vft_gateway(Default::default())
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    let result = client
        .set_vft_gateway(vft_gateway_new)
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    // anyone should be able to read the address
    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, Default::default());
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn update_config() {
    let (remoting, code_id, gas_limit) = spin_up_node().await;

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());

    let checkpoints = ActorId::from([1u8; 32]);
    let address = H160::from([2u8; 20]);
    let reply_timeout = 10_000;
    let reply_deposit = 1_000_000_000;
    let program_id = factory
        .new(
            address,
            Config {
                reply_timeout,
                reply_deposit,
            },
        )
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .unwrap();

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());

    assert_eq!(
        Config {
            reply_timeout,
            reply_deposit,
        },
        client.config().recv(program_id).await.unwrap()
    );

    let reply_timeout = 4_000;
    let reply_deposit = 1_222_000_000;
    client
        .update_config(Config {
            reply_timeout,
            reply_deposit,
        })
        .send_recv(program_id)
        .await
        .unwrap();

    assert_eq!(
        Config {
            reply_timeout,
            reply_deposit,
        },
        client.config().recv(program_id).await.unwrap()
    );

    // another account isn't permitted to update the config
    let api = GearApi::dev().await.unwrap().with("//Bob").unwrap();
    let remoting = GClientRemoting::new(api);

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());
    let result = client
        .update_config(Config {
            reply_timeout: 111,
            reply_deposit: 222,
        })
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    // anyone is able to get the config
    assert_eq!(
        Config {
            reply_timeout,
            reply_deposit,
        },
        client.config().recv(program_id).await.unwrap()
    );
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn test_relay_erc20() {
    // tx 0x180cd2328df9c4356adc77e19e33c5aa2d5395f1b52e70d22c25070a04f16691
    let tx_index = 15;

    let route = <vft_gateway::io::MintTokens as ActionIo>::ROUTE;

    let receipts: Receipts = serde_json::from_slice(HOLESKY_RECEIPTS_2_498_456.as_ref()).unwrap();
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

    let Proof { proof, receipt } = eth_utils::generate_proof(tx_index, &receipts[..]).unwrap();

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

    let (remoting, code_id, gas_limit) = spin_up_node().await;
    let admin = <[u8; 32]>::from(remoting.api().account_id().clone());
    let admin = ActorId::from(admin);

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());
    let program_id = factory
        .new(
            admin,
            H160::from(hex!("33B53f4E8bA2B127712af3C9723626cf98091D87")),
            Config {
                reply_timeout: 1_000,
                reply_deposit: 5_500_000_000,
            },
        )
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .unwrap();
    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());
    client
        .set_vft_gateway(admin)
        .with_gas_limit(gas_limit)
        .send_recv(program_id)
        .await
        .unwrap();

    let mut listener = remoting.api().subscribe().await.unwrap();

    let result = client
        .relay(message)
        .with_gas_limit(gas_limit)
        .send(program_id)
        .await
        .unwrap();

    // wait for Handle::GetCheckpointFor request and reply to it
    let message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.destination == admin.into() && message.details.is_none() =>
            {
                let request = Handle::decode(&mut &message.payload.0[..]).ok()?;
                match request {
                    Handle::GetCheckpointFor { slot } if slot == 2_498_456 => Some(message.id),
                    _ => None,
                }
            }
            _ => None,
        })
        .await
        .unwrap();

    let reply = HandleResult::Checkpoint(Ok((
        2_498_464,
        hex!("b89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1").into(),
    )));
    remoting
        .api()
        .send_reply(message_id.into(), reply, gas_limit, 0)
        .await
        .unwrap();

    // wait for MintTokens request and reply to it
    let message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.destination == admin.into() && message.details.is_none() =>
            {
                message.payload.0.starts_with(route).then_some(message.id)
            }
            _ => None,
        })
        .await
        .unwrap();

    let reply: <vft_gateway::io::MintTokens as ActionIo>::Reply = Ok(());
    let payload = {
        let mut result = Vec::with_capacity(route.len() + reply.encoded_size());
        result.extend_from_slice(route);
        reply.encode_to(&mut result);

        result
    };
    remoting
        .api()
        .send_reply_bytes(message_id.into(), payload, gas_limit, 0)
        .await
        .unwrap();

    let result = result.recv().await.unwrap();
    assert!(result.is_ok());
}
*/
