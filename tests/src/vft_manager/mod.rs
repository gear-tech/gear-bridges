use anyhow::anyhow;
use gclient::{Event, EventProcessor, GearApi, GearEvent, Result};
use gear_core::gas::GasInfo;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use std::collections::HashMap;
use vft::WASM_BINARY as WASM_VFT;
use vft_client::traits::*;
use vft_manager::WASM_BINARY as WASM_VFT_MANAGER;
use vft_manager_client::{traits::*, Config, InitConfig, Order, TokenSupply};

use crate::{connect_to_node, Connection, DEFAULT_BALANCE};

async fn calculate_reply_gas(
    api: &GearApi,
    service: &mut vft_manager_client::VftManager<GClientRemoting>,
    i: u64,
    supply_type: TokenSupply,
    gas_limit: u64,
    vft_manager_id: ActorId,
) -> Result<GasInfo> {
    let route = match supply_type {
        TokenSupply::Ethereum => <vft_client::vft_admin::io::Mint as ActionIo>::ROUTE,
        TokenSupply::Gear => <vft_client::vft::io::TransferFrom as ActionIo>::ROUTE,
    };

    let account: &[u8; 32] = api.account_id().as_ref();
    let origin = H256::from_slice(account);
    let account = ActorId::from(*account);

    let mut listener = api.subscribe().await?;

    service
        .calculate_gas_for_reply(i, i, supply_type.clone())
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
        if let TokenSupply::Gear = supply_type {
            reply.encode_to(&mut result);
        }

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

            a / 2 + b / 2 + (a % 2 + b % 2) / 2
        }

        // odd
        _ => array[len / 2],
    }
}

async fn test(supply_type: TokenSupply, amount: U256) -> Result<(bool, U256)> {
    assert!(!(amount / 2).is_zero());

    let conn @ Connection { .. } = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;

    let api = conn.api.clone();
    let suri = conn.accounts[0].2.clone();
    let suri_unauthorized = conn.accounts[1].2.clone();
    let code_id = conn.code_ids[0];
    let code_id_vft = conn.code_ids[1];
    let api = api.with(suri).unwrap();
    let account: &[u8; 32] = api.account_id().as_ref();
    let account = ActorId::from(*account);
    let salt = conn.salt;

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
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout: 100,
                fee_bridge: 0,
                fee_incoming: 0,
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
    let factory = vft_client::VftFactory::new(remoting.clone());
    let vft_id = factory
        .new("TEST_TOKEN".into(), "TT".into(), 20)
        .send_recv(code_id_vft, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (extended_vft)", hex::encode(vft_id));

    // Allocating underlying shards.
    let mut vft_extension = vft_client::VftExtension::new(remoting.clone());
    while vft_extension
        .allocate_next_balances_shard()
        .send_recv(vft_id)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .send_recv(vft_id)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    let mut service_vft = vft_client::VftAdmin::new(remoting.clone());
    // allow VFT-manager to burn funds
    service_vft
        .set_burner(vft_manager_id)
        .send_recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // mint some tokens to the user
    service_vft
        .mint(account, amount)
        .send_recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // the user grants allowance to VFT-manager for a specified token amount
    let mut service_vft = vft_client::Vft::new(remoting.clone());
    let amount = amount / 2;
    if !service_vft
        .approve(vft_manager_id, amount)
        .send_recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?
    {
        return Err(anyhow!("Unable to approve transfer").into());
    }

    // add the VFT program to the VFT-manager mapping
    let eth_token_id = H160::from([1u8; 20]);
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .map_vara_to_eth_address(vft_id, eth_token_id, supply_type)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // before the user submits their bridging request, an unauthorized user learns
    // about the allowance (e.g., by monitoring transactions or allowances on-chain)
    // and submits `request_bridging` to the VFT-manager on the user behalf. It is worth noting
    // that VFT-manager has burner role so is able to call burn functionality on any user funds.
    let mut service =
        vft_manager_client::VftManager::new(remoting.clone().with_suri(suri_unauthorized));
    let reply = service
        .request_bridging(vft_id, amount, Default::default())
        .send(vft_manager_id)
        .await;

    let balance = service_vft
        .balance_of(account)
        .recv(vft_id)
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

    let conn @ Connection { gas_limit, .. } = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let api = conn.api.clone();
    let salt = conn.salt;
    let api = api.with("//Bob").unwrap();
    let code_id = conn.code_ids[0];
    let _code_id_vft = conn.code_ids[1];

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
                    gas_for_swap_token_maps: 1_500_000_000,
                    reply_timeout: 100,
                    fee_bridge: 0,
                    fee_incoming: 0,
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

#[tokio::test]
async fn getter_transactions() -> Result<()> {
    const CAPACITY: usize = 10;

    let conn @ Connection { gas_limit, .. } = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let api = conn.api.clone();
    let suri = conn.accounts[0].2.clone();
    let salt = conn.salt;
    let code_id = conn.code_ids[0];
    let _code_id_vft = conn.code_ids[1];
    let api = api.with(suri).unwrap();

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
                    gas_for_swap_token_maps: 1_500_000_000,
                    reply_timeout: 100,
                    fee_bridge: 0,
                    fee_incoming: 0,
                },
            },
            slot_start,
            Some(CAPACITY as u32),
        )
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    let service = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    let result = service
        .transactions(Order::Direct, CAPACITY as u32, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(result.is_empty());

    let result = service
        .transactions(Order::Reverse, CAPACITY as u32, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(result.is_empty());

    let result = service
        .transactions(Order::Direct, 0, CAPACITY as u32)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    result
        .into_iter()
        .fold((slot_start, 0u64), |prev, (current_slot, current_index)| {
            assert_eq!(prev, (current_slot, current_index));

            (current_slot, current_index + 1)
        });

    let result = service
        .transactions(Order::Reverse, 0, CAPACITY as u32)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    result.into_iter().fold(
        (slot_start, CAPACITY as u64 - 1),
        |prev, (current_slot, current_index)| {
            assert_eq!(prev, (current_slot, current_index));

            (current_slot, current_index - 1)
        },
    );

    Ok(())
}

#[tokio::test]
async fn msg_tracker_state() -> Result<()> {
    let conn @ Connection { gas_limit, .. } = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let api = conn.api.clone();
    let suri = conn.accounts[0].2.clone();
    let code_id = conn.code_ids[0];
    let _code_id_vft = conn.code_ids[1];
    let salt = conn.salt;
    let api = api.with(suri).unwrap();

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let vft_manager_id = factory
        .new(InitConfig {
            erc20_manager_address: Default::default(),
            gear_bridge_builtin: Default::default(),
            historical_proxy_address: Default::default(),
            config: Config {
                gas_for_token_ops: 20_000_000_000,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_builtin: 20_000_000_000,
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout: 100,
                fee_bridge: 0,
                fee_incoming: 0,
            },
        })
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    let mut service = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    service
        .insert_message_info(
            Default::default(),
            vft_manager_client::MessageStatus::SendingMessageToBridgeBuiltin,
            vft_manager_client::TxDetails {
                vara_token_id: Default::default(),
                sender: Default::default(),
                amount: Default::default(),
                receiver: Default::default(),
                token_supply: vft_manager_client::TokenSupply::Ethereum,
            },
        )
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let result = service
        .request_briding_msg_tracker_state(1, 10)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(result.is_empty());

    let result = service
        .request_briding_msg_tracker_state(0, 2)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0],
        (
            Default::default(),
            vft_manager_client::MessageInfo {
                status: vft_manager_client::MessageStatus::SendingMessageToBridgeBuiltin,
                details: vft_manager_client::TxDetails {
                    vara_token_id: Default::default(),
                    sender: Default::default(),
                    amount: Default::default(),
                    receiver: Default::default(),
                    token_supply: vft_manager_client::TokenSupply::Ethereum,
                },
            }
        )
    );

    Ok(())
}

#[tokio::test]
async fn upgrade() -> Result<()> {
    let Connection {
        api,
        accounts,
        code_ids,
        gas_limit,
        salt,
    } = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let suri = accounts[0].2.clone();
    let suri2 = accounts[1].2.clone();
    let code_id = code_ids[0];
    let code_id_vft = code_ids[1];
    let api = api.with(suri).unwrap();

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_id = factory
        .new(InitConfig {
            erc20_manager_address: Default::default(),
            gear_bridge_builtin: Default::default(),
            historical_proxy_address: Default::default(),
            config: Config {
                gas_for_token_ops: 20_000_000_000,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_builtin: 20_000_000_000,
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout: 100,
                fee_bridge: 0,
                fee_incoming: 0,
            },
        })
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    // upgrade request from a non-authorized source should fail
    let api_unauthorized = api.clone().with(suri2).unwrap();
    let mut service =
        vft_manager_client::VftManager::new(GClientRemoting::new(api_unauthorized.clone()));
    let result = service
        .upgrade(Default::default())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err(), "result = {result:?}");

    // upgrade request to the running VftManager should fail
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    let result = service
        .upgrade(Default::default())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err(), "result = {result:?}");

    // deploy Vara Fungible Token
    let factory = vft_client::VftFactory::new(remoting.clone());
    let vft_id_1 = factory
        .new("TEST_TOKEN1".into(), "TT1".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, [])
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (extended_vft1)", hex::encode(vft_id_1));

    // Allocating underlying shards.
    let mut vft_extension = vft_client::VftExtension::new(remoting.clone());
    while vft_extension
        .allocate_next_balances_shard()
        .send_recv(vft_id_1)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .send_recv(vft_id_1)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    let mut service_vft = vft_client::VftAdmin::new(remoting.clone());
    // mint some tokens to the user
    service_vft
        .mint(vft_manager_id, 100.into())
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // deploy another Vara Fungible Token
    let factory = vft_client::VftFactory::new(remoting.clone());
    let vft_id_2 = factory
        .new("TEST_TOKEN2".into(), "TT2".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (extended_vft2)", hex::encode(vft_id_2));

    // Allocating underlying shards.
    while vft_extension
        .allocate_next_balances_shard()
        .send_recv(vft_id_2)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .send_recv(vft_id_2)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    // add token mappings
    service
        .map_vara_to_eth_address(vft_id_1, [1u8; 20].into(), TokenSupply::Gear)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();
    service
        .map_vara_to_eth_address(vft_id_2, [2u8; 20].into(), TokenSupply::Ethereum)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    // pause the VftManager
    service
        .pause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    // upgrade the VftManager
    let mut service = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    service
        .upgrade(Default::default())
        .with_gas_limit(gas_limit)
        .send(vft_manager_id)
        .await
        .unwrap();

    let result = service.erc_20_manager_address().recv(vft_manager_id).await;
    assert!(result.is_err(), "result = {result:?}");
    let error = format!("{result:?}");
    assert!(error.contains("InactiveProgram"));

    Ok(())
}

#[ignore = "Used to benchmark gas usage for swapping collections of TokenMap"]
#[tokio::test]
async fn bench_gas_for_token_map_swap() -> Result<()> {
    const COUNT: usize = 1_000;

    let Connection {
        api,
        accounts: _,
        code_ids,
        gas_limit,
        salt,
    } = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let code_id = code_ids[0];
    let api = api.with("//Bob").unwrap();

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let vft_manager_id = factory
        .new(InitConfig {
            erc20_manager_address: Default::default(),
            gear_bridge_builtin: Default::default(),
            historical_proxy_address: Default::default(),
            config: Config {
                gas_for_token_ops: 20_000_000_000,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_builtin: 20_000_000_000,
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout: 100,
                fee_bridge: 0,
                fee_incoming: 0,
            },
        })
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    let mut service = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    for i in 0..10 {
        let supply_type = match i > 0 {
            true => TokenSupply::Ethereum,
            false => TokenSupply::Gear,
        };

        service
            .map_vara_to_eth_address([i; 32].into(), [i; 20].into(), supply_type)
            .with_gas_limit(gas_limit)
            .send_recv(vft_manager_id)
            .await
            .unwrap();
    }

    println!("prepared");

    let mut results_burned = Vec::with_capacity(COUNT);
    let mut results_min_limit = Vec::with_capacity(COUNT);

    let account: &[u8; 32] = api.account_id().as_ref();
    let origin = H256::from_slice(account);

    for _i in 0..COUNT {
        let gas_info = api
            .calculate_handle_gas(
                Some(origin),
                vft_manager_id,
                vft_manager_client::vft_manager::io::CalculateGasForTokenMapSwap::ROUTE.to_vec(),
                0,
                true,
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

#[tokio::test]
async fn update_vfts() -> Result<()> {
    let Connection {
        api,
        accounts,
        code_ids,
        gas_limit,
        salt,
    } = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let suri = accounts[0].2.clone();
    let suri2 = accounts[1].2.clone();
    let code_id = code_ids[0];
    let code_id_vft = code_ids[1];
    let api = api.with(suri).unwrap();

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_id = factory
        .new(InitConfig {
            erc20_manager_address: Default::default(),
            gear_bridge_builtin: Default::default(),
            historical_proxy_address: Default::default(),
            config: Config {
                gas_for_token_ops: 20_000_000_000,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_builtin: 20_000_000_000,
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout: 100,
                fee_bridge: 0,
                fee_incoming: 0,
            },
        })
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    // non-authorized user cannot update VFT-addresses
    let api_unauthorized = api.clone().with(suri2).unwrap();
    let mut service = vft_manager_client::VftManager::new(GClientRemoting::new(api_unauthorized));
    let result = service
        .update_vfts([].to_vec())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err());

    // deploy another VFT-manager that used as an upgraded VFT
    let salt2 = {
        let mut salt_new = Vec::with_capacity(2 * salt.len());

        salt_new.extend_from_slice(&salt);
        salt_new.extend_from_slice(&salt);

        salt_new
    };
    let vft = factory
        .new(InitConfig {
            erc20_manager_address: Default::default(),
            gear_bridge_builtin: Default::default(),
            historical_proxy_address: Default::default(),
            config: Config {
                gas_for_token_ops: 20_000_000_000,
                gas_for_reply_deposit: 10_000_000_000,
                gas_to_send_request_to_builtin: 20_000_000_000,
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout: 100,
                fee_bridge: 0,
                fee_incoming: 0,
            },
        })
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt2.clone())
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft)", hex::encode(vft));

    // deploy Vara Fungible Token
    let factory = vft_client::VftFactory::new(remoting.clone());
    let extended_vft_id_1 = factory
        .new("TEST_TOKEN1".into(), "TT1".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (extended_vft1)",
        hex::encode(extended_vft_id_1)
    );

    // deploy another Vara Fungible Token
    let factory = vft_client::VftFactory::new(remoting.clone());
    let extended_vft_id_2 = factory
        .new("TEST_TOKEN2".into(), "TT2".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, salt2)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (extended_vft2)",
        hex::encode(extended_vft_id_2)
    );

    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .map_vara_to_eth_address(vft, [1u8; 20].into(), TokenSupply::Gear)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();
    service
        .map_vara_to_eth_address(extended_vft_id_1, [2u8; 20].into(), TokenSupply::Ethereum)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();
    service
        .map_vara_to_eth_address(extended_vft_id_2, [3u8; 20].into(), TokenSupply::Ethereum)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    // pause the VftManager
    service
        .pause()
        .with_gas_limit(gas_limit)
        .send_recv(vft)
        .await
        .unwrap();

    // upgrade the VftManager so it exits
    service
        .upgrade(Default::default())
        .with_gas_limit(gas_limit)
        .send(vft)
        .await
        .unwrap();

    service
        .update_vfts(
            [
                // upgraded "VFT"
                (vft, Default::default()),
                // the VFT isn't upgraded so should stay the same
                (extended_vft_id_1, [1u8; 32].into()),
            ]
            .to_vec(),
        )
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    let mut expected: HashMap<_, _> = [
        (
            ActorId::default(),
            (H160::from([1u8; 20]), TokenSupply::Gear),
        ),
        (extended_vft_id_1, ([2u8; 20].into(), TokenSupply::Ethereum)),
        (extended_vft_id_2, ([3u8; 20].into(), TokenSupply::Ethereum)),
    ]
    .into();
    for (vft, erc20, supply) in service
        .vara_to_eth_addresses()
        .with_gas_limit(gas_limit)
        .recv(vft_manager_id)
        .await
        .unwrap()
        .into_iter()
    {
        let (expected_erc20, expected_supply) = expected.remove(&vft).unwrap();
        assert_eq!(expected_erc20, erc20);
        assert_eq!(expected_supply, supply);
    }

    assert!(expected.is_empty());

    Ok(())
}
