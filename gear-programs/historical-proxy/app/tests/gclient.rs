use checkpoint_light_client_io::{Handle, HandleResult};
use eth_events_deneb_client::traits::*;
use gclient::{DispatchStatus, Event, EventProcessor, GearApi, GearEvent, WSAddress};
use gear_core::ids::prelude::*;
use hex_literal::hex;
use historical_proxy_client::traits::*;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use sp_core::crypto::DEV_PHRASE;
use tokio::sync::Mutex;
use vft_manager_client::vft_manager;

mod shared;

static LOCK: Mutex<(u32, Option<CodeId>, Option<CodeId>)> = Mutex::const_new((
    42_42_42, /* random number choosen by fair random */
    None, None,
));

async fn connect_to_node() -> (GearApi, ActorId, CodeId, CodeId, GasUnit, [u8; 4]) {
    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();

    let (api, proxy_code_id, ethereum_event_client_code_id, salt) = {
        let mut lock = LOCK.lock().await;
        let proxy_code_id = match lock.1 {
            Some(code_id) => code_id,
            None => {
                let code_id = api
                    .upload_code(historical_proxy::WASM_BINARY)
                    .await
                    .map(|(code_id, ..)| code_id)
                    .unwrap_or_else(|_| CodeId::generate(historical_proxy::WASM_BINARY));
                lock.1 = Some(code_id);

                code_id
            }
        };

        let ethereum_event_client_code_id = match lock.2 {
            Some(code_id) => code_id,
            None => {
                let code_id = api
                    .upload_code(eth_events_deneb::WASM_BINARY)
                    .await
                    .map(|(code_id, ..)| code_id)
                    .unwrap_or_else(|_| CodeId::generate(eth_events_deneb::WASM_BINARY));
                lock.2 = Some(code_id);

                code_id
            }
        };

        let salt = lock.0;
        lock.0 += 1;

        let suri = format!("{DEV_PHRASE}//historical-proxy-{salt}:");
        let api2 = GearApi::init_with(WSAddress::dev(), suri).await.unwrap();

        let account_id: &[u8; 32] = api2.account_id().as_ref();
        api.transfer_keep_alive((*account_id).into(), 100_000_000_000_000)
            .await
            .unwrap();
        (api2, proxy_code_id, ethereum_event_client_code_id, salt)
    };

    let id = api.account_id();
    let admin = <[u8; 32]>::from(id.clone());
    let admin = ActorId::from(admin);

    (
        api,
        admin,
        proxy_code_id,
        ethereum_event_client_code_id,
        gas_limit,
        salt.to_le_bytes(),
    )
}

#[tokio::test]
async fn proxy() {
    let message = shared::event();

    let (api, admin, proxy_code_id, relay_code_id, gas_limit, salt) = connect_to_node().await;
    println!("node spun up, code uploaded, gas_limit={}", gas_limit);
    let factory = eth_events_deneb_client::EthEventsDenebFactory::new(
        GClientRemoting::new(api.clone()),
    );
    let ethereum_event_client_program_id = factory
        .new(admin)
        .with_gas_limit(gas_limit)
        .send_recv(relay_code_id, salt)
        .await
        .unwrap();

    let proxy_program_id =
        historical_proxy_client::HistoricalProxyFactory::new(GClientRemoting::new(api.clone()))
            .new()
            .with_gas_limit(5_500_000_000)
            .send_recv(proxy_code_id, salt)
            .await
            .unwrap();
    println!("relay and proxy programs created");
    let mut proxy_client =
        historical_proxy_client::HistoricalProxy::new(GClientRemoting::new(api.clone()));

    proxy_client
        .add_endpoint(
            message.proof_block.block.slot,
            ethereum_event_client_program_id,
        )
        .send_recv(proxy_program_id)
        .await
        .unwrap();

    let endpoint = proxy_client
        .endpoint_for(message.proof_block.block.slot)
        .recv(proxy_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(endpoint, ethereum_event_client_program_id);
    println!(
        "endpoint {:?}\nproxy: {:?}\nadmin: {:?}",
        endpoint, proxy_program_id, admin
    );

    let gas_limit = api.block_gas_limit().unwrap();
    let mut listener = api.subscribe().await.unwrap();
    let result = proxy_client
        .redirect(
            message.proof_block.block.slot,
            message.encode(),
            admin,
            <vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec(),
        )
        .with_gas_limit(gas_limit / 100 * 95)
        .send(proxy_program_id)
        .await
        .unwrap();
    let message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.source == ethereum_event_client_program_id.into()
                    && message.destination == admin.into()
                    && message.details.is_none() =>
            {
                let request = Handle::decode(&mut &message.payload.0[..]).ok()?;

                match request {
                    Handle::GetCheckpointFor { slot } if slot == 2_498_456 => {
                        println!("get checkpoint for: #{}, messageID={:?}", slot, message.id);
                        Some(message.id)
                    }
                    _ => None,
                }
            }

            _ => None,
        })
        .await
        .unwrap();

    let reply = HandleResult::Checkpoint(Ok((
        2_496_464,
        hex!("b89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1").into(),
    )));

    let mut listener = api.subscribe().await.unwrap();
    let (message_id, _, _) = match api
        .send_reply(message_id.into(), reply, gas_limit / 100 * 95, 0)
        .await
    {
        Ok(reply) => reply,
        Err(err) => {
            let block = api.last_block_number().await.unwrap();
            println!(
                "failed to send reply to {:?}: {:?}, block={}",
                message_id, err, block
            );
            let result = result.recv().await.unwrap().unwrap();
            println!("{:?}", result);
            crate::panic!("{:?}", err);
        }
    };

    println!("Checkpoint reply with ID {:?}", message_id);

    println!("Processed...");
    // wait for SubmitReceipt request and reply to it
    let predicate = |e| match e {
        Event::Gear(GearEvent::UserMessageSent { message, .. })
            if message.destination == admin.into() && message.details.is_none() =>
        {
            message
                .payload
                .0
                .starts_with(vft_manager::io::SubmitReceipt::ROUTE)
                .then_some((Some(message.id), None))
        }

        Event::Gear(GearEvent::MessagesDispatched { statuses, .. }) => {
            statuses.into_iter().find_map(|(mid, status)| {
                (mid.0 == message_id.into_bytes())
                    .then_some((None, Some(DispatchStatus::from(status))))
            })
        }

        _ => None,
    };

    let mut results = listener
        .proc_many(predicate, |pairs| {
            let len = pairs.len();

            (pairs, len > 1)
        })
        .await
        .unwrap();
    let (message_id_1, status_1) = results.pop().unwrap();
    let (message_id_2, status_2) = results.pop().unwrap();

    assert!(status_1.or(status_2).unwrap().succeed());

    let message_id = message_id_1.or(message_id_2).unwrap();

    println!("Submit receipt request");
    let reply: <vft_manager::io::SubmitReceipt as ActionIo>::Reply = Ok(());
    let route = <vft_manager::io::SubmitReceipt as ActionIo>::ROUTE;
    let payload = {
        let mut result = route.to_vec();
        reply.encode_to(&mut result);
        result
    };

    api.send_reply_bytes(message_id.into(), payload, gas_limit / 100 * 95, 0)
        .await
        .unwrap();

    let result = result.recv().await.unwrap().expect("proxy failed");
    assert_eq!(result.0, message.receipt_rlp);

    // returned slot should be correct regardless the input slot
    let slot_expected = message.proof_block.block.slot;
    let result = proxy_client
        .redirect(
            // intentionally submit different slot whithin the same epoch
            1 + slot_expected,
            message.encode(),
            admin,
            <vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec(),
        )
        .with_gas_limit(gas_limit / 100 * 95)
        .send(proxy_program_id)
        .await
        .unwrap();
    let message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.source == ethereum_event_client_program_id.into()
                    && message.destination == admin.into()
                    && message.details.is_none() =>
            {
                let request = Handle::decode(&mut &message.payload.0[..]).ok()?;

                match request {
                    Handle::GetCheckpointFor { slot } if slot == 2_498_456 => {
                        println!("get checkpoint for: #{}, messageID={:?}", slot, message.id);
                        Some(message.id)
                    }
                    _ => None,
                }
            }

            _ => None,
        })
        .await
        .unwrap();

    let reply = HandleResult::Checkpoint(Ok((
        2_496_464,
        hex!("b89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1").into(),
    )));

    let mut listener = api.subscribe().await.unwrap();
    let (message_id, _, _) = match api
        .send_reply(message_id.into(), reply, gas_limit / 100 * 95, 0)
        .await
    {
        Ok(reply) => reply,
        Err(err) => {
            let block = api.last_block_number().await.unwrap();
            println!(
                "failed to send reply to {:?}: {:?}, block={}",
                message_id, err, block
            );
            let result = result.recv().await.unwrap().unwrap();
            println!("{:?}", result);
            crate::panic!("{:?}", err);
        }
    };

    println!("Checkpoint reply with ID {:?}", message_id);

    let _message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.destination == admin.into() && message.details.is_none() =>
            {
                let route = vft_manager::io::SubmitReceipt::ROUTE;
                if message
                    .payload
                    .0
                    .starts_with(route)
                {
                    let slice = &message
                    .payload
                    .0[route.len()..];
                    let (slot, ..) = <vft_manager::io::SubmitReceipt as ActionIo>::Params::decode(&mut &slice[..]).unwrap();

                    assert_eq!(slot, slot_expected);

                    Some(())
                } else {
                    None
                }
            }

            _ => None,
        })
        .await
        .unwrap();
}
