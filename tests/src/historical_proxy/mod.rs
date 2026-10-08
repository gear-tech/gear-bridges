use crate::{connect_to_node, DEFAULT_BALANCE};
use eth_events_deneb_client::traits::EthEventsDenebFactory;
use gclient::{Event, EventProcessor, GearEvent};
use gstd::ActorId;
use historical_proxy_client::traits::{HistoricalProxy, HistoricalProxyFactory};
use sails_rs::{calls::*, gclient::calls::*, Decode, Encode};
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
    let proof = shared::event();
    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "historical-proxy",
        &[
            historical_proxy::WASM_BINARY,
            eth_events_deneb::WASM_BINARY,
            mock_contract::WASM_BINARY,
        ],
    )
    .await;
    let gas_limit = conn.gas_limit;
    let admin = conn.accounts[0].0;
    let api = conn.api.with(&conn.accounts[0].2).unwrap();
    let (_, checkpoint, _) = api
        .create_program_bytes(conn.code_ids[2], conn.salt, [], gas_limit, 0)
        .await
        .unwrap();
    let remoting = GClientRemoting::new(api.clone());
    let endpoint = eth_events_deneb_client::EthEventsDenebFactory::new(remoting.clone())
        .new(checkpoint)
        .with_gas_limit(gas_limit)
        .send_recv(conn.code_ids[1], conn.salt)
        .await
        .unwrap();
    let proxy = historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
        .new()
        .with_gas_limit(gas_limit)
        .send_recv(conn.code_ids[0], conn.salt)
        .await
        .unwrap();
    let mut client = historical_proxy_client::HistoricalProxy::new(remoting);
    let slot = proof.proof_block.block.slot;
    client
        .add_endpoint(slot, endpoint)
        .send_recv(proxy)
        .await
        .unwrap();
    assert_eq!(
        client.endpoint_for(slot).recv(proxy).await.unwrap(),
        Ok(endpoint)
    );
    let route = vft_manager::io::SubmitReceipt::ROUTE;

    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        // Reject a wrong receipt slot even when it selects the same endpoint.
        for hint in [slot, slot + 1] {
            let mut listener = api.subscribe().await.unwrap();
            let result = client
                .redirect(hint, proof.encode(), admin, route.to_vec())
                .with_gas_limit(gas_limit / 100 * 95)
                .send(proxy)
                .await
                .unwrap();
            if hint != slot {
                assert!(matches!(
                    result.recv().await.unwrap(),
                    Err(historical_proxy_client::ProxyError::DecodeFailure(_))
                ));
                listener.proc(|event| match event {
                    Event::Gear(GearEvent::UserMessageSent { message, .. })
                        if message.source() == proxy && message.destination() == admin => {
                        assert!(message.details().is_some(), "wrong slot reached the consumer");
                        Some(())
                    }
                    _ => None,
                }).await.unwrap();
                continue;
            }
            let result = result.recv();
            tokio::pin!(result);
            let consumer = listener.proc(|event| match event {
                    Event::Gear(GearEvent::UserMessageSent { message, .. })
                        if message.source() == proxy
                            && message.destination() == admin
                            && message.details().is_none()
                            && message.payload_bytes().starts_with(route) =>
                    {
                        let (actual_slot, index, receipt) =
                            <vft_manager::io::SubmitReceipt as ActionIo>::Params::decode(
                                &mut &message.payload_bytes()[route.len()..],
                            )
                            .unwrap();
                        assert_eq!(actual_slot, slot);
                        assert_eq!(index, proof.transaction_index);
                        assert_eq!(receipt, proof.receipt_rlp);
                        Some(message.id())
                    }
                    _ => None,
                });
            let message_id = tokio::select! {
                returned = &mut result => panic!("proxy ended before consumer delivery: {returned:?}"),
                message_id = consumer => message_id.unwrap(),
            };
            let reply: <vft_manager::io::SubmitReceipt as ActionIo>::Reply = Ok(());
            let mut payload = route.to_vec();
            reply.encode_to(&mut payload);
            api.send_reply_bytes(message_id, payload, gas_limit / 100 * 95, 0)
                .await
                .unwrap();
            let returned = result.await.unwrap().expect("proxy failed");
            assert_eq!(returned.0, proof.receipt_rlp);
        }
    })
    .await
    .expect("historical proxy delivery exceeded 120 seconds after deployment");
}
