use crate::{connect_to_node, DEFAULT_BALANCE};
use anyhow::anyhow;
use gclient::{DispatchStatus, Event, EventProcessor, GearApi, GearEvent, Result};
use gear_core::rpc::GasInfo;
use sails_rs::{calls::*, events::EventIo, gclient::calls::*, prelude::*};
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use vft::WASM_BINARY as WASM_VFT;
use vft_client::traits::*;
use vft_manager::WASM_BINARY as WASM_VFT_MANAGER;
use vft_manager_client::{
    traits::*, vft_manager::events::VftManagerEvents, Config, Error, InitConfig, Order, TokenSupply,
};
use vft_vara::WASM_BINARY as WASM_VFT_VARA;

pub mod gtest;

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

    let mut listener = api.subscribe().await.map_err(|e| anyhow!("{e:?}"))?;

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

    let conn = connect_to_node(
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

    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .unpause()
        .with_gas_limit(conn.gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

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

    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;

    let gas_limit = conn.gas_limit;
    let api = conn.api.clone();
    let salt = conn.salt;
    let api = api.with("//Bob").unwrap();
    let code_id = conn.code_ids[0];

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let slot_start = 2_000;
    let vft_manager_id = factory
        .gas_calculation(
            InitConfig {
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

    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;

    let gas_limit = conn.gas_limit;
    let suri = conn.accounts[0].2.clone();
    let salt = conn.salt;
    let code_id = conn.code_ids[0];
    let api = conn.api.with(suri).unwrap();

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let slot_start = 2_000;
    let vft_manager_id = factory
        .gas_calculation(
            InitConfig {
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
    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;

    let gas_limit = conn.gas_limit;

    let suri = conn.accounts[0].2.clone();
    let code_id = conn.code_ids[0];
    let _code_id_vft = conn.code_ids[1];
    let salt = conn.salt;
    let api = conn.api.with(suri).unwrap();

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let vft_manager_id = factory
        .new(InitConfig {
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
    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;

    let accounts = conn.accounts;
    let code_ids = conn.code_ids;
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;
    let suri = accounts[0].2.clone();
    let suri2 = accounts[1].2.clone();
    let code_id = code_ids[0];
    let code_id_vft = code_ids[1];
    let api = conn.api.with(suri).unwrap();

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_id = factory
        .new(InitConfig {
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

    // deploy Vara Fungible Token
    let factory = vft_client::VftFactory::new(remoting.clone());
    let vft_id_1 = factory
        .new("TEST_TOKEN1".into(), "TT1".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, [])
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft_id_1)", hex::encode(vft_id_1));

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

    // upgrade request with a non-VftManager program should fail
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    let result = service
        .upgrade(vft_id_1)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err(), "result = {result:?}");
    let error = format!("{result:?}");
    assert!(error.contains("panicked with"), "error = {error}");

    // unpause the VftManager
    service
        .unpause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // upgrade request from a non-authorized source should fail
    let api_unauthorized = api.clone().with(suri2).unwrap();
    let mut service_unauthorized =
        vft_manager_client::VftManager::new(GClientRemoting::new(api_unauthorized.clone()));
    let result = service_unauthorized
        .upgrade(Default::default())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err(), "result = {result:?}");

    // upgrade request to the running VftManager should fail
    let result = service
        .upgrade(Default::default())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err(), "result = {result:?}");

    let mut service_vft = vft_client::VftAdmin::new(remoting.clone());
    // mint some tokens to the vft_manager_id
    let balance_1 = 100.into();
    service_vft
        .mint(vft_manager_id, balance_1)
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

    println!("program_id = {:?} (vft_id_2)", hex::encode(vft_id_2));

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

    // mint some vft2 tokens to the vft_manager_id
    let balance_2 = 222.into();
    service_vft
        .mint(vft_manager_id, balance_2)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_2)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

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

    // deploy another VftManager
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager2_id = factory
        .new(InitConfig {
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
        .send_recv(code_id, [salt, salt].concat())
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager2)",
        hex::encode(vft_manager2_id)
    );

    // upgrade the VftManager
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .upgrade(vft_manager2_id)
        .with_gas_limit(gas_limit)
        .send(vft_manager_id)
        .await
        .unwrap();

    let result = service.erc_20_manager_address().recv(vft_manager_id).await;
    assert!(result.is_err(), "result = {result:?}");
    let error = format!("{result:?}");
    assert!(error.contains("InactiveProgram"));

    // check that funds transferred to the new program
    let service_vft = vft_client::Vft::new(remoting.clone());
    let balance = service_vft
        .balance_of(vft_manager2_id)
        .recv(vft_id_1)
        .await
        .unwrap();
    assert_eq!(balance_1, balance);

    let balance = service_vft
        .balance_of(vft_manager2_id)
        .recv(vft_id_2)
        .await
        .unwrap();
    assert_eq!(balance_2, balance);

    Ok(())
}

#[ignore = "Used to benchmark gas usage for swapping collections of TokenMap"]
#[tokio::test]
async fn bench_gas_for_token_map_swap() -> Result<()> {
    const COUNT: usize = 1_000;

    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let code_id = conn.code_ids[0];
    let api = conn.api.with("//Bob").unwrap();
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;

    // deploy VFT-manager
    let factory = vft_manager_client::VftManagerFactory::new(GClientRemoting::new(api.clone()));
    let vft_manager_id = factory
        .new(InitConfig {
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
    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;
    let suri = conn.accounts[0].2.clone();
    let suri2 = conn.accounts[1].2.clone();
    let code_id = conn.code_ids[0];
    let code_id_vft = conn.code_ids[1];
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;
    let api = conn.api.with(suri).unwrap();

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_id = factory
        .new(InitConfig {
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

    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .unpause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // non-authorized user cannot update VFT-addresses
    let api_unauthorized = api.clone().with(suri2).unwrap();
    let mut service_unauthorized =
        vft_manager_client::VftManager::new(GClientRemoting::new(api_unauthorized));
    let result = service_unauthorized
        .update_vfts([].to_vec())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err());

    // deploy Vara Fungible Token programs
    let factory = vft_client::VftFactory::new(remoting.clone());
    let vft = factory
        .new("TEST_TOKEN1".into(), "TT1".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft)", hex::encode(vft));

    let vft_upgraded = factory
        .new("TEST_TOKEN12".into(), "TT12".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, [salt, salt].concat())
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_upgraded)",
        hex::encode(vft_upgraded)
    );

    let vft1 = factory
        .new("TEST_TOKEN1".into(), "TT1".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, [salt, salt, salt].concat())
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft1)", hex::encode(vft1));

    let vft2 = factory
        .new("TEST_TOKEN2".into(), "TT2".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, [salt, salt, salt, salt].concat())
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft2)", hex::encode(vft2));

    service
        .map_vara_to_eth_address(vft, [1u8; 20].into(), TokenSupply::Gear)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();
    service
        .map_vara_to_eth_address(vft1, [2u8; 20].into(), TokenSupply::Ethereum)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();
    service
        .map_vara_to_eth_address(vft2, [3u8; 20].into(), TokenSupply::Ethereum)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    // pause the Vft
    let mut service_vft = vft_client::VftAdmin::new(remoting.clone());
    let _ = service_vft
        .pause()
        .with_gas_limit(gas_limit)
        .send_recv(vft)
        .await;

    // upgrade the Vft
    service_vft
        .exit(vft_upgraded)
        .with_gas_limit(gas_limit)
        .send(vft)
        .await
        .unwrap();

    service
        .update_vfts(
            [
                // upgraded VFT
                (vft, vft_upgraded),
                // the VFT isn't upgraded so should stay the same
                (vft1, [1u8; 32].into()),
            ]
            .to_vec(),
        )
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    let mut expected: HashMap<_, _> = [
        (vft_upgraded, (H160::from([1u8; 20]), TokenSupply::Gear)),
        (vft1, ([2u8; 20].into(), TokenSupply::Ethereum)),
        (vft2, ([3u8; 20].into(), TokenSupply::Ethereum)),
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

#[tokio::test]
async fn init() -> Result<()> {
    let conn = connect_to_node(&[DEFAULT_BALANCE], "vft-manager", &[WASM_VFT_MANAGER]).await;
    let suri = conn.accounts[0].2.clone();
    let code_id = conn.code_ids[0];
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;
    let api = conn.api.with(suri).unwrap();

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_id = factory
        .new(InitConfig {
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

    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    let result = service
        .is_paused()
        .with_gas_limit(gas_limit)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(result);
    let result = service
        .erc_20_manager_address()
        .with_gas_limit(gas_limit)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(result.is_none());

    // teleport requests should fail since the VftManager is paused
    let result = service
        .submit_receipt(0, 0, vec![])
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(result.is_err(), "result = {result:?}");
    let result = service
        .request_bridging(Default::default(), 0.into(), Default::default())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(result.is_err(), "result = {result:?}");

    service
        .unpause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // teleport requests should fail since the ERC20Manager address is not set
    let result = service
        .submit_receipt(0, 0, vec![])
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"));
    assert!(result.is_err(), "result = {result:?}");
    let result = service
        .request_bridging(Default::default(), 0.into(), Default::default())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"));
    assert!(result.is_err(), "result = {result:?}");

    Ok(())
}

#[tokio::test]
async fn migrate_transactions() -> Result<()> {
    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER],
    )
    .await;

    let accounts = conn.accounts;
    let code_ids = conn.code_ids;
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;
    let suri = accounts[0].2.clone();
    let suri2 = accounts[1].2.clone();
    let code_id = code_ids[0];
    let api = conn.api.with(suri).unwrap();

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_id = factory
        .new(InitConfig {
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

    // request from a non-authorized source should fail
    let api_unauthorized = api.clone().with(suri2).unwrap();
    let mut service =
        vft_manager_client::VftManager::new(GClientRemoting::new(api_unauthorized.clone()));
    let result = service
        .insert_transactions(vec![])
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err(), "result = {result:?}");

    // request to the running VftManager should fail
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .unpause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    let result = service
        .insert_transactions(vec![])
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await;
    assert!(result.is_err(), "result = {result:?}");

    // pause the VftManager
    service
        .pause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    // insert some transactions
    let transactions = vec![(4_178_375, 22), (4_182_830, 22), (4_182_948, 26)];
    let mut service = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()));
    service
        .insert_transactions(transactions.clone())
        .with_gas_limit(gas_limit)
        .send(vft_manager_id)
        .await
        .unwrap();

    let result = service
        .transactions(Order::Direct, 0, transactions.len() as _)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(result, transactions);

    Ok(())
}

#[tokio::test]
async fn avft_burn_from() -> Result<()> {
    use vft_vara_client::{
        traits::{Vft, VftAdmin, VftExtension, VftVaraFactory},
        vft_2::events::Vft2Events,
    };

    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT_VARA],
    )
    .await;

    let accounts = conn.accounts;
    let code_ids = conn.code_ids;
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;
    let suri = accounts[0].2.clone();
    let code_id = code_ids[0];
    let code_id_vft = code_ids[1];
    let api = conn.api.with(suri).unwrap();

    // deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_id = factory
        .new(InitConfig {
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

    // unpause the VftManager
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .unpause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let address_erc20_manager = [1u8; 20];
    service
        .update_erc_20_manager_address(address_erc20_manager.into())
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    service
        .update_historical_proxy_address(accounts[0].0)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // deploy Vara Fungible Token
    let factory = vft_vara_client::VftVaraFactory::new(remoting.clone());
    let vft_id = factory
        .new()
        .with_gas_limit(gas_limit)
        .send_recv(code_id_vft, [])
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft)", hex::encode(vft_id));

    // Allocating underlying shards.
    let mut vft_extension = vft_vara_client::VftExtension::new(remoting.clone());
    while vft_extension
        .allocate_next_balances_shard()
        .with_gas_limit(gas_limit)
        .send_recv(vft_id)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .with_gas_limit(gas_limit)
        .send_recv(vft_id)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    // emulate that VftManager has some locked tokenized Vara

    let amount_total: U256 = 10_000_000_000_000u64.into();
    let amount_1 = amount_total / 3;
    let amount_2 = amount_total - amount_1;
    api.transfer_keep_alive(vft_id, amount_total.try_into().unwrap())
        .await?;

    let address_token = [2u8; 20];
    let address_from = [3u8; 20];
    let address_receiver = accounts[1].0;
    let mut service_vft_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    // mint tokens to the vft-manager
    service_vft_admin
        .mint(vft_manager_id, amount_total)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    service_vft_admin
        .set_minter(vft_manager_id)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    service_vft_admin
        .set_burner(vft_manager_id)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // add token mappings
    service
        .map_vara_to_eth_address(vft_id, address_token.into(), TokenSupply::Gear)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let receipt_rlp = crate::create_receipt_rlp(
        address_erc20_manager.into(),
        address_from.into(),
        address_receiver,
        address_token.into(),
        amount_1,
    );
    service
        .submit_receipt(10, 2, receipt_rlp)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("Failed to submit receipt: {e:?}"))?
        .unwrap();

    let service_vft = vft_vara_client::Vft::new(remoting.clone());
    let balance = service_vft
        .balance_of(vft_manager_id)
        .recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    // VftManager should have (amount_total - amount_1) on its balance after transfer
    assert_eq!(balance, amount_2);

    let balance = service_vft
        .balance_of(address_receiver)
        .recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(balance, 0.into());

    let api2 = api.clone().with(&accounts[1].2).unwrap();
    let messages = api2.get_mailbox_messages(1).await?;
    let (message, _interval) = messages.first().unwrap();
    assert_eq!(message.value(), u128::try_from(amount_1).unwrap());

    let (value_claimed, _block_hash) = api2.claim_value(message.id()).await?;
    assert_eq!(value_claimed, u128::try_from(amount_1).unwrap());

    let balance = api.total_balance(address_receiver).await?;
    assert!(balance > DEFAULT_BALANCE);

    // attempt to transfer and unwrap tokens to the program should fail

    let balance_vft_native_before = api.total_balance(vft_id).await?;

    let mut listener = api.subscribe().await?;
    let receipt_rlp = crate::create_receipt_rlp(
        address_erc20_manager.into(),
        address_from.into(),
        vft_id,
        address_token.into(),
        amount_2,
    );
    service
        .submit_receipt(10, 3, receipt_rlp)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("Failed to submit receipt 2: {e:?}"))?
        .unwrap();

    let balance = service_vft
        .balance_of(vft_manager_id)
        .recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(balance, 0.into());

    listener
        .proc(|event| match event {
            gclient::Event::Gear(gclient::GearEvent::UserMessageSent { message, .. })
                if message.source.0 == vft_id.into_bytes() && message.destination.0 == [0; 32] =>
            {
                if let Ok(Vft2Events::Transfer { from, to, value }) =
                    Vft2Events::decode_event(&message.payload.0)
                {
                    if from.is_zero() && to == vft_id {
                        assert_eq!(value, amount_2);
                        return Some(());
                    }
                }

                None
            }

            _ => None,
        })
        .await?;

    let balance = service_vft
        .balance_of(vft_id)
        .recv(vft_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(balance, amount_2);

    let balance_vft_native = api.total_balance(vft_id).await?;
    assert_eq!(balance_vft_native, balance_vft_native_before);

    Ok(())
}

// A helper function. Returns a remoting, a GearApi instance, the vft-manager id, the user id,
// an extended VFT id with specified token supply type.
async fn deploy_programs(
    create_vft: bool,
    add_roles: bool,
    supply_type: TokenSupply,
    reply_timeout: Option<u32>,
) -> Result<(
    impl Remoting + Clone,
    GearApi,
    ActorId,
    ActorId,
    ActorId,
    H160,
)> {
    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "vft-manager",
        &[WASM_VFT_MANAGER, WASM_VFT],
    )
    .await;

    let user_id = conn.accounts[0].0;
    let vft_manager_code_id = conn.code_ids[0];
    let vft_code_id = conn.code_ids[1];
    let api = conn.api.with(&conn.accounts[0].2).unwrap();
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;

    println!("user_id = {:?}", hex::encode(user_id));

    // Deploy VFT-manager
    let remoting = GClientRemoting::new(api.clone());
    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());

    let reply_timeout = reply_timeout.unwrap_or(100);

    let vft_manager_id = factory
        .new(InitConfig {
            gear_bridge_builtin: Default::default(),
            historical_proxy_address: user_id,
            config: Config {
                gas_for_token_ops: 20_000_000_000,
                gas_for_reply_deposit: 20_000_000_000,
                gas_to_send_request_to_builtin: 10_000_000_000,
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout,
                fee_bridge: 0,
                fee_incoming: 0,
            },
        })
        .send_recv(vft_manager_code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!(
        "program_id = {:?} (vft_manager)",
        hex::encode(vft_manager_id)
    );

    // Deploy an extended Vara Fungible Token program if a VFT id is not provided
    let extended_vft_id = if !create_vft {
        user_id
    } else {
        let factory = vft_client::VftFactory::new(remoting.clone());
        factory
            .new("TEST_TOKEN".into(), "TT".into(), 20)
            .send_recv(vft_code_id, salt)
            .await
            .map_err(|e| anyhow!("{e:?}"))?
    };
    println!(
        "program_id = {:?} (extended_vft)",
        hex::encode(extended_vft_id)
    );

    let erc20_manager_address = H160([1_u8; 20]);

    // Unpause and update the VFT-manager
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .unpause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    service
        .update_erc_20_manager_address(erc20_manager_address)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    let erc20_addr = H160([255_u8; 20]);

    // Add Vara to ETH mappings
    service
        .map_vara_to_eth_address(extended_vft_id, erc20_addr, supply_type)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // Add roles to the extended VFT if required
    let mut service_vft = vft_client::VftAdmin::new(remoting.clone());
    if add_roles {
        service_vft
            .set_minter(vft_manager_id)
            .send_recv(extended_vft_id)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;
        service_vft
            .set_burner(vft_manager_id)
            .send_recv(extended_vft_id)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;
    }

    Ok((
        remoting,
        api,
        vft_manager_id,
        user_id,
        extended_vft_id,
        erc20_addr,
    ))
}

// "Sunny day" test for the `submit_receipt` method of the VFT manager.
#[tokio::test]
async fn submit_receipt_works() -> Result<()> {
    let (remoting, api, vft_manager_id, user_id, vft_token_id, erc20_addr) =
        deploy_programs(true, true, TokenSupply::Ethereum, None).await?;

    let erc20_manager_address = H160([1_u8; 20]);
    let sender = H160([99_u8; 20]);
    let receipt_rlp = crate::create_receipt_rlp(
        erc20_manager_address,
        sender,
        42.into(),
        erc20_addr,
        U256::zero(),
    );

    let gas_limit = api.block_gas_limit().unwrap();

    let mut service = vft_manager_client::VftManager::new(remoting.clone());

    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(txs.is_empty());

    // Subscribe to the events listener
    let mut listener = api.subscribe().await.unwrap();

    // Send `submit_receipt` request to Vft-manager
    let result = service
        .submit_receipt(0, 1, receipt_rlp)
        .with_gas_limit(gas_limit / 100 * 95)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // Checking two things:
    // - a reply to the `submit_receipt` request has been sent to the user
    // - a `BridgingAccepted` event has been emitted by the VFT manager
    let predicate = |e| match e {
        Event::Gear(GearEvent::UserMessageSent { message, .. })
            if message.destination == user_id.into() && message.source == vft_manager_id.into() =>
        {
            message
                .payload
                .0
                .starts_with(vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE)
                .then_some(())
        }
        Event::Gear(GearEvent::UserMessageSent { message, .. })
            if message.destination.0 == [0_u8; 32] && message.source == vft_manager_id.into() =>
        {
            if let VftManagerEvents::BridgingAccepted {
                to, amount, token, ..
            } = VftManagerEvents::decode_event(&message.payload.0).unwrap()
            {
                assert_eq!(to, 42.into());
                assert_eq!(token, vft_token_id);
                assert_eq!(amount, U256::zero());

                Some(())
            } else {
                None
            }
        }
        _ => None,
    };

    listener
        .proc_many(predicate, |tuples| {
            let total = tuples.len();

            (tuples, total > 1)
        })
        .await
        .unwrap();

    assert!(result.is_ok(), "Result: {result:?}");

    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    assert_eq!(txs.len(), 1, "Transactions list must contain one pair");
    assert!(txs.iter().any(|(slot, tx_idx)| *slot == 0 && *tx_idx == 1));

    Ok(())
}

// Check whether an error in a VFT contract is propagated correctly to the sender
// of the `submit_receipt` message.
// Prerequisites:
// - Deploying a vft-manager.
// - Deploying a VFT contract with token supply type Ethereum, but not adding any roles to it.
// Test scenario:
// - on behalf of the historical proxy, sending `submit_receipt` message to the VFT manager,
//   with the rlp receipt indicating a transfer of an ERC20 token with Ethereum supply
// - the upstream VFT contract will panic attempting to mint tokens.
// Expecting:
// - no events are emitted
// - the transactions list is not updated
// - the correct error is propagated to the sender.
#[tokio::test]
async fn error_in_vft_propagated_correctly() -> Result<()> {
    use core::panic;

    let (remoting, api, vft_manager_id, user_id, _, erc20_addr) =
        deploy_programs(true, false, TokenSupply::Ethereum, Some(10)).await?;

    let gas_limit = api.block_gas_limit().unwrap();

    let mut service = vft_manager_client::VftManager::new(remoting.clone());

    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(txs.is_empty());

    // Subscribe to the events listeners
    let mut listener = api.subscribe().await.unwrap();
    let mut another_listener = api.clone().subscribe().await.unwrap();

    let erc20_manager_address = H160([1_u8; 20]);
    let sender = H160([99_u8; 20]);
    let receipt_rlp = crate::create_receipt_rlp(
        erc20_manager_address,
        sender,
        42.into(),
        erc20_addr,
        U256::zero(),
    );
    let result = service
        .submit_receipt(0, 1, receipt_rlp)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // Trying to listen for the `BridgingAccepted` event emitted by the VFT manager,
    // but there isn't supposed to be one since the VFT contract finished with an error.
    // We wait for 30 seconds before concluding that the event was not emitted.
    let emitted = tokio::select! {
        _ = listener.proc(|e| {
            match e {
                Event::Gear(GearEvent::UserMessageSent { message, .. })
                    if message.destination.0 == [0_u8; 32]
                        && message.source == vft_manager_id.into() =>
                {
                    if let VftManagerEvents::BridgingAccepted {
                        ..
                    } = VftManagerEvents::decode_event(&message.payload.0).unwrap()
                    {
                        Some(())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }) => {
            Some(())
        },
        _ = sleep(Duration::from_secs(30)) => {
            None
        }
    };

    let usr_msg = another_listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.destination == user_id.into() =>
            {
                message
                    .payload
                    .0
                    .starts_with(vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE)
                    .then_some(())
            }
            _ => None,
        })
        .await;

    // The `BridgingAccepted` event is not expected to be emitted, but the user message is
    // expected to be sent back to the user.
    assert!(emitted.is_none(), "BridgingAccepted event was emitted");
    assert!(usr_msg.is_ok(), "User message was not sent back");

    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(txs.is_empty());

    assert!(
        matches!(result, Err(Error::Internal { .. })),
        "Result: {result:?}"
    );

    Ok(())
}

// Check whether a failure check the reply in `handle_reply` hook is handled correctly.
//
// Prerequisites:
// - Deploying a vft-manager.
// - Registering ourselves (the user) as both the historical proxy and a VFT contract
// (with token supply type Gear).
// Test scenario:
// - on behalf of the historical proxy, sending `submit_receipt` message to the VFT manager,
//   with the rlp receipt indicating a transfer of an ERC20 token with Gear supply
// - waiting (as a VFT contract) for a message from the vft-manager to do `transfer_from` action
// - replying with `false` to the vft-manager meaning that the transfer was not successful
// Expecting:
// - the `handle_reply` hook fails to check the reply and exits without updating the transactions
// - the transactions are not updated
// - no `BridgingAccepted` event is emitted
#[tokio::test]
async fn illegal_reply_in_handle_reply() -> Result<()> {
    let (remoting, api, vft_manager_id, user_id, extended_vft_id, erc20_addr) =
        deploy_programs(false, false, TokenSupply::Gear, None).await?;

    let erc20_manager_address = H160([1_u8; 20]);
    let sender = H160([99_u8; 20]);
    let receipt_rlp = crate::create_receipt_rlp(
        erc20_manager_address,
        sender,
        42.into(),
        erc20_addr,
        U256::zero(),
    );

    let gas_limit = api.block_gas_limit().unwrap();

    let mut service = vft_manager_client::VftManager::new(remoting.clone());

    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(txs.is_empty());

    // Subscribe to the events listener
    let mut listener = api.subscribe().await.unwrap();

    // Send `submit_receipt` request to Vft-manager
    let reply_ticket = service
        .submit_receipt(0, 1, receipt_rlp)
        .with_gas_limit(gas_limit / 100 * 95)
        .send(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let mut vft_msg_id: MessageId = Default::default();
    let mut submit_receipt_msg_id: MessageId = Default::default();

    // At this point expecting the vft-manager to have sent a `transfer_from` call,
    // to the token minter, while itself having been placed in the waitlist.
    let predicate = |e| match e {
        Event::Gear(GearEvent::UserMessageSent { message, .. })
            if message.destination == extended_vft_id.into()
                && message.source == vft_manager_id.into() =>
        {
            message
                .payload
                .0
                .starts_with(vft_client::vft::io::TransferFrom::ROUTE)
                .then_some((MessageId::new(message.id.0), 0_usize))
        }
        Event::Gear(GearEvent::MessageWaited { id, .. }) => Some((MessageId::new(id.0), 1_usize)),
        _ => None,
    };

    for (msg_id, idx) in listener
        .proc_many(predicate, |pairs| {
            let len = pairs.len();

            (pairs, len > 1)
        })
        .await
        .unwrap()
    {
        if idx == 0 {
            vft_msg_id = msg_id;
        } else if idx == 1 {
            submit_receipt_msg_id = msg_id;
        }
    }

    // Sending `false` as a reply to the vft-manager
    let reply: <vft_client::vft::io::TransferFrom as ActionIo>::Reply = false;
    let reply_payload = {
        let mut buffer = vft_client::vft::io::TransferFrom::ROUTE.to_vec();
        reply.encode_to(&mut buffer);

        buffer
    };

    let mut listener = api.subscribe().await.unwrap();
    let (reply_message_id, _, _) = match api
        .send_reply_bytes(vft_msg_id, reply_payload, gas_limit / 100 * 95, 0)
        .await
    {
        Ok(reply) => reply,
        Err(err) => {
            let block = api.last_block_number().await.unwrap();
            println!("Failed to send reply to {vft_msg_id:?}: {err:?}, block={block}");
            let result = reply_ticket.recv().await.unwrap();
            println!("{result:?}");
            crate::panic!("{:?}", err);
        }
    };

    let result = reply_ticket.recv().await.map_err(|e| anyhow!("{e:?}"))?;

    let predicate = |e| match e {
        Event::Gear(GearEvent::UserMessageSent { message, .. })
            if message.destination == user_id.into() && message.source == vft_manager_id.into() =>
        {
            message
                .payload
                .0
                .starts_with(vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE)
                .then_some(())
        }
        Event::Gear(GearEvent::MessageWoken { id, .. }) => {
            assert_eq!(id.0, submit_receipt_msg_id.into_bytes());
            Some(())
        }
        Event::Gear(GearEvent::MessagesDispatched { statuses, .. }) => {
            statuses.into_iter().find_map(|(msg_id, status)| {
                if msg_id.0 == reply_message_id.into_bytes() {
                    assert_eq!(DispatchStatus::from(status), DispatchStatus::Success);
                    Some(())
                } else {
                    None
                }
            })
        }
        _ => None,
    };

    listener
        .proc_many(predicate, |tuples| {
            let total = tuples.len();

            (tuples, total > 2)
        })
        .await
        .unwrap();

    // Successful transactions number didn't change
    let service = vft_manager_client::VftManager::new(remoting.clone());
    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(txs.is_empty());

    assert!(
        matches!(result, Err(Error::InvalidReply)),
        "Result: {result:?}"
    );

    Ok(())
}

#[tokio::test]
async fn vft_token_operation_timeout_works() -> Result<()> {
    use core::panic;

    const REPLY_TIMEOUT: u32 = 10;

    let (remoting, api, vft_manager_id, user_id, _, erc20_addr) =
        deploy_programs(false, false, TokenSupply::Gear, Some(10)).await?;

    let erc20_manager_address = H160([1_u8; 20]);
    let sender = H160([99_u8; 20]);
    let receipt_rlp = crate::create_receipt_rlp(
        erc20_manager_address,
        sender,
        42.into(),
        erc20_addr,
        U256::zero(),
    );

    let gas_limit = api.block_gas_limit().unwrap();

    let mut service = vft_manager_client::VftManager::new(remoting.clone());

    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(txs.is_empty());

    // Subscribe to two event listeners
    let mut listener = api.subscribe().await.unwrap();
    let mut another_listener = api.clone().subscribe().await.unwrap();

    let start_block = api.last_block_number().await.unwrap();

    // Send `submit_receipt` request to Vft-manager
    let result = service
        .submit_receipt(0, 1, receipt_rlp)
        .with_gas_limit(gas_limit / 100 * 95)
        .send_recv(vft_manager_id)
        .await
        .unwrap();

    let _ = listener
        .proc(|e| match e {
            Event::Gear(GearEvent::UserMessageSent { message, .. })
                if message.destination == user_id.into() =>
            {
                message
                    .payload
                    .0
                    .starts_with(vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE)
                    .then_some(())
            }
            _ => None,
        })
        .await;
    let end_block = api.last_block_number().await.unwrap();

    // We shouldn't be getting the final user reply for at least `REPLY_TIMEOUT` blocks
    assert!(
        end_block >= start_block + REPLY_TIMEOUT,
        "end_block: {end_block}, start_block: {start_block}, REPLY_TIMEOUT: {REPLY_TIMEOUT}"
    );

    // Waiting for the `BridgingAccepted` event for another 30 seconds.
    // If no event is detected, we conclude that it has not been emitted.
    let emitted = tokio::select! {
        _ = another_listener.proc(|e| {
            match e {
                Event::Gear(GearEvent::UserMessageSent { message, .. })
                    if message.destination.0 == [0_u8; 32]
                        && message.source == vft_manager_id.into() =>
                {
                    if let VftManagerEvents::BridgingAccepted {
                        ..
                    } = VftManagerEvents::decode_event(&message.payload.0).unwrap()
                    {
                        Some(())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }) => {
            Some(())
        },
        _ = sleep(Duration::from_secs(30)) => {
            None
        }
    };

    assert!(emitted.is_none(), "BridgingAccepted event was emitted");

    // Successful transactions number didn't change
    let txs = service
        .transactions(Order::Direct, 0, 1)
        .recv(vft_manager_id)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(txs.is_empty());

    assert!(
        matches!(result, Err(Error::ReplyTimeout(..))),
        "Result: {result:?}"
    );

    Ok(())
}
