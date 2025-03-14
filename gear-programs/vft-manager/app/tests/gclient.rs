use anyhow::anyhow;
use extended_vft::WASM_BINARY as WASM_EXTENDED_VFT;
use extended_vft_client::traits::*;
use gclient::{Event, EventProcessor, GearApi, GearEvent, Result};
use gear_core::{gas::GasInfo, ids::prelude::*};
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use sp_core::{crypto::DEV_PHRASE, sr25519::Pair, Pair as _};
use sp_runtime::{
    traits::{IdentifyAccount, Verify},
    MultiSignature,
};
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

async fn create_account(api: &GearApi, suri: &str) -> Result<()> {
    let pair = Pair::from_string(suri, None).map_err(|e| anyhow!("{e:?}"))?;
    let account = <MultiSignature as Verify>::Signer::from(pair.public()).into_account();
    let account_id: &[u8; 32] = account.as_ref();

    api.transfer_keep_alive((*account_id).into(), 100_000_000_000_000)
        .await?;

    Ok(())
}

async fn connect_to_node() -> Result<(GearApi, String, String, CodeId, CodeId, GasUnit, [u8; 4])> {
    let api = GearApi::dev().await?;
    let gas_limit = api.block_gas_limit()?;

    let (suri1, suri2, code_id, code_id_vft, salt) = {
        let mut lock = LOCK.lock().await;

        let code_id = upload_code(&api, WASM_VFT_MANAGER, &mut lock.1).await?;
        let code_id_vft = upload_code(&api, WASM_EXTENDED_VFT, &mut lock.2).await?;

        let salt = lock.0;
        lock.0 += 2;

        let suri1 = format!("{DEV_PHRASE}//vft-manager-{salt}");
        create_account(&api, &suri1).await?;

        let salt = 1 + salt;
        let suri2 = format!("{DEV_PHRASE}//vft-manager-{salt}");
        create_account(&api, &suri2).await?;

        (suri1, suri2, code_id, code_id_vft, salt)
    };

    Ok((
        api,
        suri1,
        suri2,
        code_id,
        code_id_vft,
        gas_limit,
        salt.to_le_bytes(),
    ))
}

async fn calculate_reply_gas(
    api: &GearApi,
    service: &mut vft_manager_client::VftManager<GClientRemoting>,
    i: u64,
    supply_type: TokenSupply,
    gas_limit: u64,
    vft_manager_id: ActorId,
) -> Result<GasInfo> {
    let route = match supply_type {
        TokenSupply::Ethereum => <extended_vft_client::vft::io::Mint as ActionIo>::ROUTE,
        TokenSupply::Gear => <extended_vft_client::vft::io::TransferFrom as ActionIo>::ROUTE,
    };

    let account: &[u8; 32] = api.account_id().as_ref();
    let origin = H256::from_slice(account);
    let account = ActorId::from(*account);

    let mut listener = api.subscribe().await?;

    service
        .calculate_gas_for_reply(i, i, supply_type)
        .with_gas_limit(gas_limit)
        .send(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let message_id = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.destination == account.into() && message.details.is_none() =>
            {
                message.payload.0.starts_with(route).then_some(message.id)
            }
            _ => None,
        })
        .await?;

    let reply = true;
    let payload = {
        let mut result = Vec::with_capacity(route.len() + reply.encoded_size());
        result.extend_from_slice(route);
        reply.encode_to(&mut result);

        result
    };

    api.calculate_reply_gas(Some(origin), message_id.into(), payload, 0, true)
        .await
}

fn average(array: &[u64]) -> u64 {
    let len = array.len();
    match len % 2 {
        // even
        0 => {
            let i = len / 2;
            let a = array[i - 1];
            let b = array[i];

            a / 2 + b / 2 + a % 2 + b % 2
        }

        // odd
        _ => array[len / 2],
    }
}

async fn test(supply_type: TokenSupply, amount: U256) -> Result<(bool, U256)> {
    assert!(!(amount / 2).is_zero());

    let (api, suri, suri_unauthorized, code_id, code_id_vft, _gas_limit, salt) =
        connect_to_node().await?;
    let api = api.with(suri).unwrap();
    let account: &[u8; 32] = api.account_id().as_ref();
    let account = ActorId::from(*account);

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
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
    let factory = extended_vft_client::ExtendedVftFactory::new(remoting.clone());
    let extended_vft_id = factory
        .new("TEST_TOKEN".into(), "TT".into(), 20)
        .send_recv(code_id_vft, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (extended_vft)",
        hex::encode(extended_vft_id)
    );

    let mut service_vft = extended_vft_client::Vft::new(remoting.clone());
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
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .map_vara_to_eth_address(extended_vft_id, eth_token_id, supply_type)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // before the user submits their bridging request, an unauthorized user learns
    // about the allowance (e.g., by monitoring transactions or allowances on-chain)
    // and submits `request_bridging` to the VFT-manager on the user behalf. It is worth noting
    // that VFT-manager has burner role so is able to call burn functionality on any user funds.
    let mut service = vft_manager_client::VftManager::new(
        GClientRemoting::new(api.clone()).with_suri(suri_unauthorized),
    );
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

#[ignore = "Used to benchmark gas usage"]
#[tokio::test]
async fn bench_gas_for_reply() -> Result<()> {
    const CAPACITY: usize = 1_000;

    let (api, _suri, _suri2, code_id, _code_id_vft, gas_limit, salt) = connect_to_node().await?;
    let api = api.with("//Bob").unwrap();

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let slot_start = 2_000;
    let vft_manager_id = factory
        .gas_calculation(
            InitConfig {
                erc20_manager_address: Default::default(),
                gear_bridge_builtin: Default::default(),
                historical_proxy_address: Default::default(),
                config: Config {
                    gas_for_token_ops: 20_000_000_000,
                    gas_for_reply_deposit: 10_000_000_000,
                    gas_to_send_request_to_builtin: 20_000_000_000,
                    reply_timeout: 100,
                },
            },
            slot_start,
            None,
        )
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    // fill the collection with processed transactions info to bench the edge case
    let mut service = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    while service
        .fill_transactions()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap()
    {}

    println!("prepared");

    let mut results_burned = Vec::with_capacity(CAPACITY);
    let mut results_min_limit = Vec::with_capacity(CAPACITY);
    // data inserted in the head
    for i in (slot_start - CAPACITY as u64 / 2)..slot_start {
        let supply_type = match i % 2 {
            0 => TokenSupply::Ethereum,
            _ => TokenSupply::Gear,
        };

        let gas_info = calculate_reply_gas(
            &api,
            &mut service,
            i,
            supply_type,
            gas_limit,
            vft_manager_id,
        )
        .await
        .unwrap();

        results_burned.push(gas_info.burned);
        results_min_limit.push(gas_info.min_limit);
    }

    // data inserted in the tail
    for i in (2 * slot_start)..(2 * slot_start + CAPACITY as u64 / 2) {
        let supply_type = match i % 2 {
            0 => TokenSupply::Ethereum,
            _ => TokenSupply::Gear,
        };

        let gas_info = calculate_reply_gas(
            &api,
            &mut service,
            i,
            supply_type,
            gas_limit,
            vft_manager_id,
        )
        .await
        .unwrap();

        results_burned.push(gas_info.burned);
        results_min_limit.push(gas_info.min_limit);
    }

    results_burned.sort_unstable();
    results_min_limit.sort_unstable();

    println!(
        "burned: min = {:?}, max = {:?}, average = {}",
        results_burned.first(),
        results_burned.last(),
        average(&results_burned[..])
    );
    println!(
        "min_limit: min = {:?}, max = {:?}, average = {}",
        results_min_limit.first(),
        results_min_limit.last(),
        average(&results_min_limit[..])
    );

    Ok(())
}
