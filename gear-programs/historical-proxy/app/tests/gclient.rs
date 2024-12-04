use checkpoint_light_client_io::{Handle, HandleResult};
use erc20_relay_client::traits::*;
use gclient::{Event, EventProcessor, GearApi, GearEvent, WSAddress};
use hex_literal::hex;
use historical_proxy_client::traits::*;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use sp_core::crypto::DEV_PHRASE;
use tokio::sync::Mutex;
use vft_manager_client::vft_manager;

mod shared;

static LOCK: Mutex<(u32, Option<CodeId>, Option<CodeId>)> = Mutex::const_new((0, None, None));

async fn connect_to_node() -> (GearApi, ActorId, CodeId, CodeId, GasUnit, [u8; 4]) {
    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();

    let (api, proxy_code_id, erc20_relay_code_id, salt) = {
        let mut lock = LOCK.lock().await;
        let proxy_code_id = match lock.1 {
            Some(code_id) => code_id,
            None => {
                let (code_id, _) = api
                    .upload_code(historical_proxy::WASM_BINARY)
                    .await
                    .unwrap();
                lock.1 = Some(code_id);
                code_id
            }
        };

        let erc20_relay_code_id = match lock.2 {
            Some(code_id) => code_id,
            None => {
                let (code_id, _) = api.upload_code(erc20_relay::WASM_BINARY).await.unwrap();
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
        (api2, proxy_code_id, erc20_relay_code_id, salt)
    };

    let id = api.account_id();
    let admin = <[u8; 32]>::from(id.clone());
    let admin = ActorId::from(admin);

    (
        api,
        admin,
        proxy_code_id,
        erc20_relay_code_id,
        gas_limit,
        salt.to_le_bytes(),
    )
}

#[tokio::test]
async fn proxy() {
    let message = shared::event();

    let (api, admin, proxy_code_id, relay_code_id, gas_limit, salt) = connect_to_node().await;
    println!("node spun up, code uploaded, gas_limit={}", gas_limit);
    let factory = erc20_relay_client::Erc20RelayFactory::new(GClientRemoting::new(api.clone()));
    let erc20_relay_program_id = factory
        .new(admin)
        .with_gas_limit(gas_limit)
        .send_recv(relay_code_id, salt)
        .await
        .unwrap();
    let mut erc20_relay_client =
        erc20_relay_client::Erc20Relay::new(GClientRemoting::new(api.clone()));
    erc20_relay_client
        .set_vft_manager(admin)
        .with_gas_limit(5_500_000_000)
        .send_recv(erc20_relay_program_id)
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
        .add_endpoint(message.proof_block.block.slot, erc20_relay_program_id)
        .send_recv(proxy_program_id)
        .await
        .unwrap()
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

    let gas_limit = api.block_gas_limit().unwrap();
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
    let mut listener = api.subscribe().await.unwrap();
    let message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.source == erc20_relay_program_id.into()
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
    println!("Checkpoint reply with ID #{:?}", message_id);
    assert!(listener
        .message_processed(message_id)
        .await
        .unwrap()
        .succeed());
    println!("Processed...");
    let mut listener = api.subscribe().await.unwrap();
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

            _ => None,
        })
        .await
        .unwrap();

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
}
