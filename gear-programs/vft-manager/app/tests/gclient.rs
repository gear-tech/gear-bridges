use anyhow::anyhow;
use extended_vft::WASM_BINARY as WASM_EXTENDED_VFT;
use extended_vft_client::traits::*;
use gclient::{GearApi, Result, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use sp_core::crypto::DEV_PHRASE;
use tokio::sync::Mutex;
use vft_manager::WASM_BINARY as WASM_VFT_MANAGER;
use vft_manager_client::{traits::*, Config, InitConfig, TokenSupply};

static LOCK: Mutex<(u32, Option<CodeId>, Option<CodeId>)> = Mutex::const_new((3_000, None, None));

async fn upload_code(
    api: &GearApi,
    wasm_binary: &[u8],
    store: &mut Option<CodeId>,
) -> Result<CodeId> {
    Ok(match store {
        Some(code_id) => *code_id,
        None => {
            let code_id = api
                .upload_code(wasm_binary)
                .await
                .map(|(code_id, ..)| code_id)
                .unwrap_or_else(|_| CodeId::generate(wasm_binary));

            *store = Some(code_id);

            code_id
        }
    })
}

async fn connect_to_node() -> Result<(GearApi, GearApi, CodeId, CodeId, GasUnit, [u8; 4])> {
    let api = GearApi::dev().await?;
    let gas_limit = api.block_gas_limit()?;

    let (api1, api2, code_id, code_id_vft, salt) = {
        let mut lock = LOCK.lock().await;

        let code_id = upload_code(&api, WASM_VFT_MANAGER, &mut lock.1).await?;
        let code_id_vft = upload_code(&api, WASM_EXTENDED_VFT, &mut lock.2).await?;

        let salt = lock.0;
        lock.0 += 2;

        let suri = format!("{DEV_PHRASE}//vft-manager-{salt}:");
        let api2 = GearApi::init_with(WSAddress::dev(), suri).await?;
        let account_id: &[u8; 32] = api2.account_id().as_ref();
        api.transfer_keep_alive((*account_id).into(), 100_000_000_000_000)
            .await?;

        let suri = format!("{DEV_PHRASE}//vft-manager-{salt}2:");
        let api3 = GearApi::init_with(WSAddress::dev(), suri).await?;
        let account_id: &[u8; 32] = api3.account_id().as_ref();
        api.transfer_keep_alive((*account_id).into(), 100_000_000_000_000)
            .await?;

        (api2, api3, code_id, code_id_vft, salt)
    };

    Ok((
        api1,
        api2,
        code_id,
        code_id_vft,
        gas_limit,
        salt.to_le_bytes(),
    ))
}

async fn test(supply_type: TokenSupply, amount: U256) -> Result<(bool, U256)> {
    assert!(!(amount / 2).is_zero());

    let (api_unauthorized, api, code_id, code_id_vft, _gas_limit, salt) = connect_to_node().await?;
    let account: &[u8; 32] = api.account_id().as_ref();
    let account = ActorId::from(*account);

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let vft_manager_id = factory
        .new(InitConfig {
            erc20_manager_address: Default::default(),
            gear_bridge_builtin: Default::default(),
            historical_proxy_address: Default::default(),
            config: Config {
                gas_for_token_ops: 10_000_000_000,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_builtin: 10_000_000_000,
                reply_timeout: 100,
            },
        })
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    // deploy Vara Fungible Token
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

    let mut service_vft = extended_vft_client::Vft::new(GClientRemoting::new(api.clone()));
    // allow VFT-manager to burn funds
    service_vft
        .grant_burner_role(vft_manager_id)
        .send_recv(extended_vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // mint some tokens to the user
    if !service_vft
        .mint(account, amount)
        .send_recv(extended_vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?
    {
        return Err(anyhow!("Unable to mint tokens").into());
    }

    // the user grants allowance to VFT-manager for a specified token amount
    let amount = amount / 2;
    if !service_vft
        .approve(vft_manager_id, amount)
        .send_recv(extended_vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?
    {
        return Err(anyhow!("Unable to approve transfer").into());
    }

    // add the VFT program to the VFT-manager mapping
    let eth_token_id = H160::from([1u8; 20]);
    let mut service = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    service
        .map_vara_to_eth_address(extended_vft_id, eth_token_id, supply_type)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // before the user submits their bridging request, an unauthorized user learns
    // about the allowance (e.g., by monitoring transactions or allowances on-chain)
    // and submits `request_bridging` to the VFT-manager on the user behalf. It is worth noting
    // that VFT-manager has burner role so is able to call burn functionality on any user funds.
    let mut service =
        vft_manager_client::VftManager::new(GClientRemoting::new(api_unauthorized.clone()));
    let reply = service
        .request_bridging(extended_vft_id, amount, Default::default())
        .send(vft_manager_id)
        .await;

    let balance = service_vft
        .balance_of(account)
        .recv(extended_vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    Ok((reply.is_ok(), balance))
}

#[tokio::test]
async fn unauthorized_teleport_native() {
    let amount = 10_000_000.into();
    let result = test(TokenSupply::Gear, amount).await.unwrap();

    // since the request is sent by a non-authorized user it should fail
    // and the vft-balance of the user should remain unchanged
    assert_eq!(result, (false, amount));
}

#[tokio::test]
async fn unauthorized_teleport_erc20() {
    let amount = 10_000_000.into();
    let result = test(TokenSupply::Ethereum, amount).await.unwrap();

    // since the request is sent by a non-authorized user it should fail
    // and the vft-balance of the user should remain unchanged
    assert_eq!(result, (false, amount));
}
