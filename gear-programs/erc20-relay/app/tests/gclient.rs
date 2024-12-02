// Incorporate code generated based on the IDL file
#[allow(dead_code)]
mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-manager.rs"));
}

use erc20_relay_client::traits::{Erc20Relay, Erc20RelayFactory};
use gclient::GearApi;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vft::vft_manager;

async fn spin_up_node() -> (GClientRemoting, GearApi, CodeId, GasUnit) {
    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();
    let remoting = GClientRemoting::new(api.clone());
    let (code_id, _) = api.upload_code(erc20_relay::WASM_BINARY).await.unwrap();

    (remoting, api, code_id, gas_limit)
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn set_vft_manager() {
    use erc20_relay_client::Config;

    let (remoting, _api, code_id, gas_limit) = spin_up_node().await;

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());

    let program_id = factory
        .new(
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

    // by default address of the VFT manager is not set
    let vft_manager = client.vft_manager().recv(program_id).await.unwrap();
    assert_eq!(vft_manager, Default::default());

    let vft_manager_new = ActorId::from([1u8; 32]);

    // admin should be able to set the VFT manager address
    client
        .set_vft_manager(vft_manager_new)
        .send_recv(program_id)
        .await
        .unwrap();

    let vft_manager = client.vft_manager().recv(program_id).await.unwrap();
    assert_eq!(vft_manager, vft_manager_new);

    // and reset it
    client
        .set_vft_manager(Default::default())
        .send_recv(program_id)
        .await
        .unwrap();

    let vft_manager = client.vft_manager().recv(program_id).await.unwrap();
    assert_eq!(vft_manager, Default::default());

    // another account isn't permitted to change the VFT manager address
    let api = GearApi::dev().await.unwrap().with("//Bob").unwrap();
    let remoting = GClientRemoting::new(api);

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());
    let result = client
        .set_vft_manager(Default::default())
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    let result = client
        .set_vft_manager(vft_manager_new)
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    // anyone should be able to read the address
    let vft_manager = client.vft_manager().recv(program_id).await.unwrap();
    assert_eq!(vft_manager, Default::default());
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn update_config() {
    use erc20_relay_client::Config;

    let (remoting, _api, code_id, gas_limit) = spin_up_node().await;

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());

    let checkpoints = ActorId::from([1u8; 32]);
    let reply_timeout = 10_000;
    let reply_deposit = 1_000_000_000;
    let program_id = factory
        .new(
            checkpoints,
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
