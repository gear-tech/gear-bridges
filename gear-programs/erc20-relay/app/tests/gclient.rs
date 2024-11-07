// Incorporate code generated based on the IDL file
#[allow(dead_code)]
mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-gateway.rs"));
}

use erc20_relay_client::traits::{Erc20Relay, Erc20RelayFactory};
use gclient::{Event, EventProcessor, GearApi, GearEvent};
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use tokio::sync::Mutex;
use vft::vft_gateway;

static LOCK: Mutex<(u32, Option<CodeId>)> = Mutex::const_new((0, None));

async fn salt() -> [u8; 4] {
    let mut lock = LOCK.lock().await;
    let salt = lock.0;
    lock.0 += 1;

    salt.to_le_bytes()
}

// The struct purpose is to avoid the following error:
// GearSDK(Subxt(Rpc(ClientError(Call(Custom(ErrorObject { code: ServerError(1014), message: "Priority is too low: (16 vs 16)", data: Some(RawValue("The transaction has too low priority to replace another transaction already in the pool.")) }))))))
#[derive(Clone)]
struct TestRemoting(GClientRemoting);

impl sails_rs::calls::Remoting for TestRemoting {
    type Args = <GClientRemoting as sails_rs::calls::Remoting>::Args;

    async fn activate(
        self,
        code_id: CodeId,
        salt: impl AsRef<[u8]>,
        payload: impl AsRef<[u8]>,
        gas_limit: Option<GasUnit>,
        value: ValueUnit,
        args: Self::Args,
    ) -> sails_rs::errors::Result<
        impl future::Future<Output = sails_rs::errors::Result<(ActorId, Vec<u8>)>>,
    > {
        let _lock = LOCK.lock().await;

        self.0
            .activate(code_id, salt, payload, gas_limit, value, args)
            .await
    }

    async fn message(
        self,
        target: ActorId,
        payload: impl AsRef<[u8]>,
        gas_limit: Option<GasUnit>,
        value: ValueUnit,
        args: Self::Args,
    ) -> sails_rs::errors::Result<impl future::Future<Output = sails_rs::errors::Result<Vec<u8>>>>
    {
        let _lock = LOCK.lock().await;

        self.0
            .message(target, payload, gas_limit, value, args)
            .await
    }

    async fn query(
        self,
        target: ActorId,
        payload: impl AsRef<[u8]>,
        gas_limit: Option<GasUnit>,
        value: ValueUnit,
        args: Self::Args,
    ) -> sails_rs::errors::Result<Vec<u8>> {
        self.0.query(target, payload, gas_limit, value, args).await
    }
}

async fn spin_up_node() -> (impl Remoting + Clone, GearApi, CodeId, GasUnit) {
    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();
    let remoting = GClientRemoting::new(api.clone());
    let remoting = TestRemoting(remoting);
    let code_id = {
        let mut lock = LOCK.lock().await;
        match lock.1 {
            Some(code_id) => code_id,
            None => {
                let (code_id, _) = api.upload_code(erc20_relay::WASM_BINARY).await.unwrap();
                lock.1 = Some(code_id);

                code_id
            }
        }
    };

    (remoting, api, code_id, gas_limit)
}

#[tokio::test]
async fn gas_for_reply() {
    use erc20_relay_client::{traits::Erc20Relay as _, Erc20Relay, Erc20RelayFactory};

    let route = <vft_gateway::io::MintTokens as ActionIo>::ROUTE;

    let (remoting, api, code_id, gas_limit) = spin_up_node().await;
    let account_id: ActorId = <[u8; 32]>::from(api.account_id().clone()).into();

    let factory = Erc20RelayFactory::new(remoting.clone());

    let program_id = factory
        .gas_calculation(1_000, 5_500_000_000)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt().await)
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
        let mut listener = api.subscribe().await.unwrap();

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
        let gas_info = api
            .calculate_reply_gas(None, message_id.into(), payload, 0, true)
            .await
            .unwrap();

        println!("gas_info = {gas_info:?}");
    }
}

#[tokio::test]
async fn set_vft_gateway() {
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
        .send_recv(code_id, salt().await)
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
        .send_recv(code_id, salt().await)
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
