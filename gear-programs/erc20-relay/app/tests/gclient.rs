use erc20_relay_client::{
    ft_manage::events::FtManageEvents,
    traits::{Erc20RelayFactory, FtManage},
};
use futures::StreamExt;
use gclient::GearApi;
use hex_literal::hex;
use sails_rs::{calls::*, events::*, gclient::calls::*, prelude::*};

const PATH_WASM: &str = match cfg!(debug_assertions) {
    true => "../../../target/wasm32-unknown-unknown/debug/erc20_relay.opt.wasm",
    false => "../../../target/wasm32-unknown-unknown/release/erc20_relay.opt.wasm",
};

async fn spin_up_node() -> (GClientRemoting, CodeId, GasUnit) {
    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();
    let remoting = GClientRemoting::new(api);
    let code_id = remoting.upload_code_by_path(PATH_WASM).await.unwrap();

    (remoting, code_id, gas_limit)
}

#[tokio::test]
async fn tokens_map() {
    let (remoting, code_id, gas_limit) = spin_up_node().await;

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());

    let program_id = factory
        .new(Default::default(), Default::default())
        .with_gas_limit(gas_limit)
        .send_recv(code_id, "")
        .await
        .unwrap();

    let mut ft_client = erc20_relay_client::FtManage::new(remoting.clone());

    let tokens = ft_client.tokens().recv(program_id).await.unwrap();
    assert!(tokens.is_empty());

    let mut listener = erc20_relay_client::ft_manage::events::listener(remoting.clone());
    let mut events = listener.listen().await.unwrap();

    let eth_address = H160::from(hex!("52c953cac2269c599b075359EdA11E738a75c6F6"));
    let result = ft_client
        .add_fungible_token(eth_address, [0u8; 32].into())
        .send_recv(program_id)
        .await
        .unwrap();
    assert!(result.is_some());

    let event = events.next().await.unwrap();
    assert_eq!(
        (
            program_id,
            FtManageEvents::Added {
                eth_address,
                fungible_token: [0u8; 32].into()
            }
        ),
        event
    );

    let tokens = ft_client.tokens().recv(program_id).await.unwrap();
    assert_eq!(&tokens[..], &[(eth_address, [0u8; 32].into())]);

    // duplicate Eth address
    let result = ft_client
        .add_fungible_token(eth_address, [0u8; 32].into())
        .send_recv(program_id)
        .await
        .unwrap();
    assert!(result.is_none());

    // another mapping
    let eth_address2 = H160::from(hex!("52c953cac2269c599b075359EdA11E738a75c6F7"));
    let result = ft_client
        .add_fungible_token(eth_address2, [0u8; 32].into())
        .send_recv(program_id)
        .await
        .unwrap();
    assert!(result.is_some());

    let event = events.next().await.unwrap();
    assert_eq!(
        (
            program_id,
            FtManageEvents::Added {
                eth_address: eth_address2,
                fungible_token: [0u8; 32].into()
            }
        ),
        event
    );

    let tokens = ft_client.tokens().recv(program_id).await.unwrap();
    assert_eq!(
        &tokens[..],
        &[
            (eth_address, [0u8; 32].into()),
            (eth_address2, [0u8; 32].into())
        ]
    );

    let result = ft_client
        .remove_fungible_token(eth_address2)
        .send_recv(program_id)
        .await
        .unwrap();
    assert!(result.is_some());

    let event = events.next().await.unwrap();
    assert_eq!(
        (
            program_id,
            FtManageEvents::Removed {
                eth_address: eth_address2
            }
        ),
        event
    );

    let tokens = ft_client.tokens().recv(program_id).await.unwrap();
    assert_eq!(&tokens[..], &[(eth_address, [0u8; 32].into())]);

    // attempt to remove non-existant entry should do nothig
    let result = ft_client
        .remove_fungible_token(eth_address2)
        .send_recv(program_id)
        .await
        .unwrap();
    assert!(result.is_none());

    // another account isn't permitted to manage FTs
    let api = GearApi::dev().await.unwrap().with("//Bob").unwrap();
    let remoting = GClientRemoting::new(api);

    let mut ft_client = erc20_relay_client::FtManage::new(remoting.clone());

    // there should be the single mapping
    let tokens = ft_client.tokens().recv(program_id).await.unwrap();
    assert_eq!(&tokens[..], &[(eth_address, [0u8; 32].into())]);

    let eth_address3 = hex!("52c953cac2269c599b075359EdA11E738a75c6F8");
    let result = ft_client
        .add_fungible_token(eth_address3.into(), [0u8; 32].into())
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    let result = ft_client
        .remove_fungible_token(eth_address)
        .send_recv(program_id)
        .await;
    assert!(result.is_err());
}
