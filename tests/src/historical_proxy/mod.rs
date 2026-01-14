use crate::{connect_to_node, DEFAULT_BALANCE};
use checkpoint_light_client_client::service_checkpoint_for::io as checkpoint_for_io;
use eth_events_deneb_client::traits::EthEventsDenebFactory;
use gclient::{DispatchStatus, Event, EventProcessor, GearEvent};
use gstd::ActorId;
use hex_literal::hex;
use historical_proxy_client::traits::{HistoricalProxy, HistoricalProxyFactory};
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vft_manager_client::vft_manager;

mod shared;

#[tokio::test]
async fn update_admin() {
    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "historical_proxy",
        &[historical_proxy::WASM_BINARY],
    )
    .await;
    let gas_limit = conn.gas_limit;
    let api = conn.api.with(&conn.accounts[0].2).unwrap();
    let admin = conn.accounts[0].0;
    let salt = conn.salt;
    println!("admin: {admin:?}");
    let proxy_program_id =
        historical_proxy_client::HistoricalProxyFactory::new(GClientRemoting::new(api.clone()))
            .new()
            .with_gas_limit(gas_limit)
            .send_recv(conn.code_ids[0], salt)
            .await
            .unwrap();

    let api_unathorized = api.clone().with("//Bob").unwrap();
    let admin_new = api_unathorized.account_id();
    let admin_new = <[u8; 32]>::from(admin_new.clone());
    let admin_new = ActorId::from(admin_new);

    let mut proxy_client = historical_proxy_client::HistoricalProxy::new(GClientRemoting::new(
        api_unathorized.clone(),
    ));

    let result = proxy_client
        .update_admin(admin_new)
        .with_gas_limit(gas_limit)
        .send_recv(proxy_program_id)
        .await;
    assert!(result.is_err());

    let admin_current = proxy_client
        .admin()
        .with_gas_limit(gas_limit)
        .recv(proxy_program_id)
        .await
        .unwrap();
    assert_eq!(admin_current, admin);

    // The authorized user changes the admin
    let mut proxy_client =
        historical_proxy_client::HistoricalProxy::new(GClientRemoting::new(api.clone()));
    let result = proxy_client
        .update_admin(admin_new)
        .with_gas_limit(gas_limit)
        .send_recv(proxy_program_id)
        .await;
    assert!(result.is_ok());

    let admin_current = proxy_client
        .admin()
        .with_gas_limit(gas_limit)
        .recv(proxy_program_id)
        .await
        .unwrap();
    assert_eq!(admin_current, admin_new);
}

#[tokio::test]
async fn proxy() {
    let message = shared::event();

    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "historical-proxy",
        &[historical_proxy::WASM_BINARY, eth_events_deneb::WASM_BINARY],
    )
    .await;

    let gas_limit = conn.gas_limit;
    let admin = conn.accounts[0].0;
    let api = conn.api.with(&conn.accounts[0].2).unwrap();
    let salt = conn.salt;
    println!("admin: {admin:?}");

    let factory =
        eth_events_deneb_client::EthEventsDenebFactory::new(GClientRemoting::new(api.clone()));
    let ethereum_event_client_program_id = factory
        .new(admin)
        .with_gas_limit(gas_limit)
        .send_recv(conn.code_ids[1], salt)
        .await
        .unwrap();

    let proxy_program_id =
        historical_proxy_client::HistoricalProxyFactory::new(GClientRemoting::new(api.clone()))
            .new()
            .with_gas_limit(5_500_000_000)
            .send_recv(conn.code_ids[0], salt)
            .await
            .unwrap();

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
    println!("endpoint {endpoint:?}\nproxy: {proxy_program_id:?}\nadmin: {admin:?}");

    let gas_limit = api.block_gas_limit().unwrap();
    let mut listener = api.subscribe().await.unwrap();
    let result = proxy_client
        .redirect(
            message.proof_block.block.slot,
            message.encode(),
            admin,
            vft_manager::io::SubmitReceipt::ROUTE.to_vec(),
        )
        .with_gas_limit(gas_limit / 100 * 95)
        .send(proxy_program_id)
        .await
        .unwrap();
    let message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.source().into_bytes()
                    == ethereum_event_client_program_id.into_bytes()
                    && message.destination().into_bytes() == admin.into_bytes()
                    && message.details().is_none()
                    && message
                        .payload_bytes()
                        .starts_with(checkpoint_for_io::Get::ROUTE) =>
            {
                let encoded = &message.payload_bytes()[checkpoint_for_io::Get::ROUTE.len()..];
                let slot: <checkpoint_for_io::Get as ActionIo>::Params =
                    Decode::decode(&mut &encoded[..]).ok()?;

                if slot == 2_498_456 {
                    println!(
                        "get checkpoint for: #{}, messageID={:?}",
                        slot,
                        message.id()
                    );
                    Some(message.id())
                } else {
                    None
                }
            }

            _ => None,
        })
        .await
        .unwrap();

    let reply: <checkpoint_for_io::Get as ActionIo>::Reply = Ok((
        2_496_464,
        hex!("b89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1").into(),
    ));
    let payload = {
        let mut result = checkpoint_for_io::Get::ROUTE.to_vec();
        reply.encode_to(&mut result);

        result
    };

    let mut listener = api.subscribe().await.unwrap();
    let (message_id, _, _) = match api
        .send_reply_bytes(message_id.into(), payload, gas_limit / 100 * 95, 0)
        .await
    {
        Ok(reply) => reply,
        Err(err) => {
            let block = api.last_block_number().await.unwrap();
            println!("failed to send reply to {message_id:?}: {err:?}, block={block}");
            let result = result.recv().await.unwrap().unwrap();
            println!("{result:?}");
            crate::panic!("{:?}", err);
        }
    };

    println!("Checkpoint reply with ID {message_id:?}");

    println!("Processed...");
    // wait for SubmitReceipt request and reply to it
    let predicate = |e| match e {
        Event::Gear(GearEvent::UserMessageSent { message, .. })
            if message.destination().into_bytes() == admin.into_bytes()
                && message.details().is_none() =>
        {
            message
                .payload_bytes()
                .starts_with(vft_manager::io::SubmitReceipt::ROUTE)
                .then_some((Some(message.id()), None))
        }

        Event::Gear(GearEvent::MessagesDispatched { statuses, .. }) => {
            statuses.into_iter().find_map(|(mid, status)| {
                (mid.into_bytes() == message_id.into_bytes())
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
    let payload = {
        let mut result = vft_manager::io::SubmitReceipt::ROUTE.to_vec();
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
                if message.source().into_bytes()
                    == ethereum_event_client_program_id.into_bytes()
                    && message.destination().into_bytes() == admin.into_bytes()
                    && message.details().is_none()
                    && message
                        .payload_bytes()
                        .starts_with(checkpoint_for_io::Get::ROUTE) =>
            {
                let encoded = &message.payload_bytes()[checkpoint_for_io::Get::ROUTE.len()..];
                let slot: <checkpoint_for_io::Get as ActionIo>::Params =
                    Decode::decode(&mut &encoded[..]).ok()?;

                if slot == 2_498_456 {
                    println!(
                        "get checkpoint for: #{}, messageID={:?}",
                        slot,
                        message.id()
                    );
                    Some(message.id())
                } else {
                    None
                }
            }

            _ => None,
        })
        .await
        .unwrap();

    let reply: <checkpoint_for_io::Get as ActionIo>::Reply = Ok((
        2_496_464,
        hex!("b89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1").into(),
    ));
    let payload = {
        let mut result = checkpoint_for_io::Get::ROUTE.to_vec();
        reply.encode_to(&mut result);

        result
    };

    let mut listener = api.subscribe().await.unwrap();
    let (message_id, _, _) = match api
        .send_reply_bytes(message_id.into(), payload, gas_limit / 100 * 95, 0)
        .await
    {
        Ok(reply) => reply,
        Err(err) => {
            let block = api.last_block_number().await.unwrap();
            println!("failed to send reply to {message_id:?}: {err:?}, block={block}");
            let result = result.recv().await.unwrap().unwrap();
            println!("{result:?}");
            crate::panic!("{:?}", err);
        }
    };

    println!("Checkpoint reply with ID {message_id:?}");

    listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.destination().into_bytes() == admin.into_bytes()
                    && message.details().is_none() =>
            {
                let route = vft_manager::io::SubmitReceipt::ROUTE;
                if message.payload_bytes().starts_with(route) {
                    let slice = &message.payload_bytes()[route.len()..];
                    let (slot, ..) = <vft_manager::io::SubmitReceipt as ActionIo>::Params::decode(
                        &mut &slice[..],
                    )
                    .unwrap();

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
