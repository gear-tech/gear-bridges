use super::{connect_to_node, DEFAULT_BALANCE};
use anyhow::anyhow;
use gclient::Result;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vft::WASM_BINARY as WASM_VFT;
use vft_client::traits::*;

#[tokio::test]
async fn upgrade() -> Result<()> {
    let conn = connect_to_node(
        &[DEFAULT_BALANCE, DEFAULT_BALANCE, DEFAULT_BALANCE],
        "vft",
        &[WASM_VFT],
    )
    .await;

    let accounts = conn.accounts;
    let code_ids = conn.code_ids;
    let gas_limit = conn.gas_limit;
    let salt = conn.salt;
    let (id_admin, _salt, ref suri_admin) = accounts[0];
    let (id_user_1, _salt, ref suri_user_1) = accounts[1];
    let (id_user_2, _salt, ref suri_user_2) = accounts[2];
    let code_id = code_ids[0];

    let remoting = GClientRemoting::new(conn.api.clone().with(suri_admin).unwrap());
    // deploy Vara Fungible Token
    let factory = vft_client::VftFactory::new(remoting.clone());
    let vft_id_1 = factory
        .new("TEST_TOKEN1".into(), "TT1".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft)", hex::encode(vft_id_1));

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
    let vft_balance_1: U256 = 100.into();
    let vft_balance_2: U256 = 1_000_000.into();
    // mint some tokens to the users
    service_vft
        .mint(id_user_1, vft_balance_1)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    service_vft
        .mint(id_user_2, vft_balance_2)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // make some approvements
    let mut service_vft = vft_client::Vft::new(remoting.clone().with_suri(suri_user_1));
    service_vft
        .approve(id_admin, vft_balance_1 / 2)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    service_vft
        .approve(id_user_2, 1.into())
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    let mut service_vft = vft_client::Vft::new(remoting.clone().with_suri(suri_user_2));
    service_vft
        .approve(id_user_1, vft_balance_2 / 2)
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    // deploy a new Vara Fungible Token
    let vft_id_new = factory
        .new("TEST_TOKEN_NEW".into(), "TTNEW".into(), 20)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [salt, salt].concat())
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    println!("program_id = {:?} (vft_id_new)", hex::encode(vft_id_new));

    // Allocating underlying shards.
    let mut vft_extension = vft_client::VftExtension::new(remoting.clone());
    while vft_extension
        .allocate_next_balances_shard()
        .send_recv(vft_id_new)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .send_recv(vft_id_new)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    gear_common::migrate_balances(remoting.clone(), gas_limit, 100, vft_id_1, vft_id_new).await?;

    // new VFT should have no allowances
    let service_vft = vft_client::VftExtension::new(remoting.clone().with_suri(suri_user_2));
    let allowances = service_vft
        .allowances(0, 100)
        .with_gas_limit(gas_limit)
        .recv(vft_id_new)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert!(allowances.is_empty());

    // user balances should be migrated
    let balances = service_vft
        .balances(0, 10)
        .with_gas_limit(gas_limit)
        .recv(vft_id_new)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(balances.len(), 2);
    assert_eq!(
        balances
            .iter()
            .find_map(|(id, balance)| (id == &id_user_1).then_some(*balance))
            .unwrap(),
        vft_balance_1
    );
    assert_eq!(
        balances
            .iter()
            .find_map(|(id, balance)| (id == &id_user_2).then_some(*balance))
            .unwrap(),
        vft_balance_2
    );

    let service_vft = vft_client::Vft::new(remoting.clone());
    let balance = service_vft
        .balance_of(id_user_1)
        .with_gas_limit(gas_limit)
        .recv(vft_id_new)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(balance, vft_balance_1);

    let balance = service_vft
        .balance_of(id_user_2)
        .with_gas_limit(gas_limit)
        .recv(vft_id_new)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    assert_eq!(balance, vft_balance_2);

    // upgrade
    let mut service = vft_client::VftAdmin::new(remoting.clone());
    service
        .pause()
        .with_gas_limit(gas_limit)
        .send_recv(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    service
        .exit(vft_id_new)
        .with_gas_limit(gas_limit)
        .send(vft_id_1)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let result = service.is_paused().recv(vft_id_1).await;
    assert!(result.is_err(), "result = {result:?}");
    let error = format!("{result:?}");
    assert!(error.contains("InactiveProgram"));

    Ok(())
}
