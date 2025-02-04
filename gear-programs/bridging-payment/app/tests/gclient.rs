// Clippy warns about some imports below so disable the check.
// Remove the directive once the clippy happy.
#![allow(clippy::literal_string_with_formatting_args)]

use anyhow::anyhow;
use bridging_payment::WASM_BINARY as WASM_BRIDGING_PAYMENT;
use bridging_payment_client::traits::*;
use extended_vft::WASM_BINARY as WASM_EXTENDED_VFT;
use extended_vft_client::traits::*;
use gclient::{Event, EventProcessor, GearApi, GearEvent, Result, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use sp_core::crypto::DEV_PHRASE;
use tokio::sync::Mutex;
use vft_manager::WASM_BINARY as WASM_VFT_MANAGER;
use vft_manager_client::{traits::*, Config, InitConfig, TokenSupply};

#[allow(clippy::type_complexity)]
static LOCK: Mutex<(u32, Option<CodeId>, Option<CodeId>, Option<CodeId>)> =
    Mutex::const_new((4_000, None, None, None));

async fn connect_to_node() -> Result<(GearApi, CodeId, CodeId, CodeId, GasUnit, [u8; 4])> {
    let api = GearApi::dev().await?;
    let gas_limit = api.block_gas_limit()?;

    let (api, code_id, code_id_vft, code_id_pay, salt) = {
        let mut lock = LOCK.lock().await;
        let code_id = match lock.1 {
            Some(code_id) => code_id,
            None => {
                let code_id = api
                    .upload_code(WASM_VFT_MANAGER)
                    .await
                    .map(|(code_id, ..)| code_id)
                    .unwrap_or_else(|_| CodeId::generate(WASM_VFT_MANAGER));
                lock.1 = Some(code_id);

                code_id
            }
        };

        let code_id_vft = match lock.2 {
            Some(code_id) => code_id,
            None => {
                let code_id = api
                    .upload_code(WASM_EXTENDED_VFT)
                    .await
                    .map(|(code_id, ..)| code_id)
                    .unwrap_or_else(|_| CodeId::generate(WASM_EXTENDED_VFT));
                lock.2 = Some(code_id);

                code_id
            }
        };

        let code_id_pay = match lock.3 {
            Some(code_id) => code_id,
            None => {
                let code_id = api
                    .upload_code(WASM_BRIDGING_PAYMENT)
                    .await
                    .map(|(code_id, ..)| code_id)
                    .unwrap_or_else(|_| CodeId::generate(WASM_BRIDGING_PAYMENT));
                lock.3 = Some(code_id);

                code_id
            }
        };

        let salt = lock.0;
        lock.0 += 2;

        let suri = format!("{DEV_PHRASE}//vft-manager-{salt}:");
        let api2 = GearApi::init_with(WSAddress::dev(), suri).await?;
        let account_id: &[u8; 32] = api2.account_id().as_ref();
        api.transfer_keep_alive((*account_id).into(), 100_000_000_000_000)
            .await?;

        (api2, code_id, code_id_vft, code_id_pay, salt)
    };

    Ok((
        api,
        code_id,
        code_id_vft,
        code_id_pay,
        gas_limit,
        salt.to_le_bytes(),
    ))
}

#[tokio::test]
async fn teleport_payed() -> Result<()> {
    let (api, code_id, code_id_vft, code_id_pay, gas_limit, salt) = connect_to_node().await?;
    let account: &[u8; 32] = api.account_id().as_ref();
    let account = ActorId::from(*account);

    let factory = extended_vft_client::ExtendedVftFactory::new(GClientRemoting::new(api.clone()));
    let extended_vft_id = factory
        .new("TEST_TOKEN".into(), "TT".into(), 20)
        .send_recv(code_id_vft, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (extended_vft)",
        hex::encode(extended_vft_id)
    );

    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let vft_manager_id = factory
        .new(InitConfig {
            erc20_manager_address: Default::default(),
            gear_bridge_builtin: account,
            historical_proxy_address: Default::default(),
            config: Config {
                gas_for_token_ops: 20_000_000_000,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_builtin: 20_000_000_000,
                reply_timeout: 3,
            },
        })
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    let eth_token_id = H160::from([1u8; 20]);
    let mut service_manager =
        vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    service_manager
        .map_vara_to_eth_address(extended_vft_id, eth_token_id, TokenSupply::Gear)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let mut service_vft = extended_vft_client::Vft::new(GClientRemoting::new(api.clone()));
    let amount = 1_000_000.into();
    assert!(service_vft
        .mint(account, amount)
        .send_recv(extended_vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?);

    let amount = amount / 2;
    assert!(service_vft
        .approve(vft_manager_id, amount)
        .send_recv(extended_vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?);

    let factory =
        bridging_payment_client::BridgingPaymentFactory::new(GClientRemoting::new(api.clone()));
    let fee = 100_000_000;
    let payment_id = factory
        .new(bridging_payment_client::InitConfig {
            admin_address: account,
            vft_manager_address: vft_manager_id,
            config: bridging_payment_client::Config {
                fee,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_vft_manager: 50_000_000_000,
                reply_timeout: 5,
                gas_for_request_to_vft_manager_msg: 50_000_000_000,
            },
        })
        .send_recv(code_id_pay, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (bridging_payment)",
        hex::encode(payment_id)
    );

    let mut service =
        bridging_payment_client::BridgingPayment::new(GClientRemoting::new(api.clone()));
    let reply = service
        .make_request(amount, Default::default(), extended_vft_id)
        .with_value(fee)
        .with_gas_limit(gas_limit)
        .send_recv(payment_id)
        .await;

    // the request should fail since the instance of bridging payment is not
    // registered in VFT-manager as an allowed fee charger
    assert!(reply.is_err());
    assert_eq!(
        service_vft
            .balance_of(account)
            .recv(extended_vft_id)
            .await
            .unwrap(),
        amount * 2
    );

    // register the fee charger
    service_manager
        .update_fee_charger(Some(payment_id))
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // now the request should success
    let mut listener = api.subscribe().await?;
    let reply = service
        .make_request(amount, Default::default(), extended_vft_id)
        .with_value(fee)
        .with_gas_limit(gas_limit)
        .send(payment_id)
        .await;

    assert!(reply.is_ok(), "{:?}", reply.err());

    // since the account address was set as a bridge builtin address there should be
    // the corresponding message
    listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.source == vft_manager_id.into()
                    && message.destination == account.into()
                    && message.details.is_none() =>
            {
                Some(())
            }

            _ => None,
        })
        .await?;

    assert_eq!(
        service_vft
            .balance_of(account)
            .recv(extended_vft_id)
            .await
            .unwrap(),
        amount
    );

    Ok(())
}
