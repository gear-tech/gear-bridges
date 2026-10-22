// Native Gear authority/recovery smoke; the execution-header fixture is not Beacon qualification.
#[tokio::test]
#[ignore = "requires explicitly owned loopback Gear archives and current program Wasm paths"]
async fn owned_native_retired_wrapper_reconciles_each_role_original() -> Result<()> {
    use gear_core::ids::prelude::*;
    use sails_rs::calls::*;
    use vft_client::traits::{VftAdmin as _, VftFactory as _};
    use vft_manager_client::traits::VftManagerFactory as _;
    use vft_vara_client::traits::{NativeEscrow as _, VftAdmin as _, VftVaraFactory as _};

    let source = std::env::var("CUTOVER_NATIVE_SOURCE")?;
    let witness = std::env::var("CUTOVER_NATIVE_WITNESS")?;
    for rpc in [&source, &witness] {
        ensure!(
            rpc.starts_with("ws://127.0.0.1:") && rpc[15..].bytes().all(|b| b.is_ascii_digit()),
            "native fixture may only sign on explicitly owned loopback nodes"
        );
    }
    let genesis = H256::from_str(&std::env::var("CUTOVER_NATIVE_GENESIS")?)?;
    let api = GearApi::builder()
        .suri("//Alice")
        .uri(&source)
        .build()
        .await?;
    let second = GearApi::builder().uri(&witness).build().await?;
    ensure!(
        api.get_block_hash(0).await? == genesis && second.get_block_hash(0).await? == genesis,
        "owned native fixture genesis differs"
    );
    let _owner = signer_ownership::lock_signer(genesis, api.account_id().clone().into())?;
    let directory = PathBuf::from(std::env::var("CUTOVER_NATIVE_OUTPUT")?);
    ensure!(!directory.exists(), "native fixture output already exists");
    fs::create_dir_all(&directory)?;

    async fn upload(api: &GearApi, variable: &str) -> Result<CodeId> {
        let binary = fs::read(std::env::var(variable)?)?;
        let id = CodeId::generate(&binary);
        let signer: gsdk::signer::Signer = api.clone().into();
        let at = api.last_block_hash().await?;
        if let Ok(saved) = signer.api().original_code_storage_at(id, Some(at)).await {
            ensure!(saved == binary, "native fixture code bytes differ");
            return Ok(id);
        }
        api.upload_code(&binary).await?;
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                let at = api.last_block_hash().await?;
                if let Ok(saved) = signer.api().original_code_storage_at(id, Some(at)).await {
                    ensure!(
                        saved == binary,
                        "finalized native fixture code bytes differ"
                    );
                    return Ok::<_, anyhow::Error>(id);
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        })
        .await
        .context("native fixture code did not finalize")?
    }
    let manager_code = upload(&api, "CUTOVER_NATIVE_MANAGER_WASM").await?;
    let wrapper_code = upload(&api, "CUTOVER_NATIVE_WRAPPER_WASM").await?;
    let token_code = upload(&api, "CUTOVER_NATIVE_VFT_WASM").await?;
    let remoting = GClientRemoting::new(api.clone());
    let gas = api.block_gas_limit()?;
    let wrapper = vft_vara_client::VftVaraFactory::new(remoting.clone())
        .new(vft_vara_client::Mainnet::No)
        .with_gas_limit(gas)
        .send_recv(wrapper_code, Uuid::now_v7().as_bytes().to_vec())
        .await?;
    let token = vft_client::VftFactory::new(remoting.clone())
        .new("Cutover recovery fixture".into(), "CRF".into(), 12)
        .with_gas_limit(gas)
        .send_recv(token_code, Uuid::now_v7().as_bytes().to_vec())
        .await?;
    let config = vft_manager_client::Config {
        gas_for_token_ops: 10_000_000_000,
        gas_for_reply_deposit: 10_000_000_000,
        gas_to_send_request_to_builtin: 10_000_000_000,
        gas_for_swap_token_maps: 1_500_000_000,
        reply_timeout: 100,
        fee_bridge: 0,
        fee_incoming: 0,
    };
    let mut managers = Vec::with_capacity(2);
    for _ in 0..2 {
        managers.push(
            vft_manager_client::VftManagerFactory::new(remoting.clone())
                .new(vft_manager_client::InitConfig {
                    gear_bridge_builtin: actor(H256::from_str(
                        "0xf2816ced0b15749595392d3a18b5a2363d6fefe5b3b6153739f218151b7acdbf",
                    )?),
                    historical_proxy_address: wrapper,
                    config: config.clone(),
                })
                .with_gas_limit(gas)
                .send_recv(manager_code, Uuid::now_v7().as_bytes().to_vec())
                .await?,
        );
    }
    let (old, new) = (managers[0], managers[1]);
    let (mut work, chain) = interrupted_coverage_fixture().await;
    let primary = WorkEvidenceRpc::new(chain.clone()).await;
    let ethereum_witness = WorkEvidenceRpc::new(chain.clone()).await;
    let eth = PollingEthApi::new(&primary.url)
        .await?
        .with_finality_witness(&ethereum_witness.url, 1, chain[0].hash.0.into())
        .await?;
    let view = eth.verified_finalized_view().await?;
    work.schema_version = 3;
    work.ethereum_authentication = Some(view.checkpoint().clone());
    work.admissions.clear();
    work.blocks.clear();
    work.calls.clear();
    work.role_pins.clear();
    work.registration = None;
    work.retirement = None;
    work.readiness = None;
    work.pending_seal = None;
    work.new_transactions = None;
    work.new_discovery = None;
    work.intent.old_identity.gear_genesis_hash = genesis;
    work.intent.old_identity.gear_sender = api.account_id().clone().into();
    work.intent.old_identity.vft_manager_address = hash_actor(old);
    work.intent.old_identity.historical_proxy_address = hash_actor(wrapper);
    work.intent.new_identity = work.intent.old_identity.clone();
    work.intent.new_identity.vft_manager_address = hash_actor(new);
    work.intent.new_manager_code_id = H256(manager_code.into_bytes());
    work.intent.old_transactions = directory.join("originals");
    work.intent.old_discovery = directory.join("discovery.json");
    work.intent.native_wrapper = Some(hash_actor(wrapper));
    let wrapped_erc20 = H160::repeat_byte(21);
    let token_erc20 = H160::repeat_byte(22);
    work.intent.mappings = vec![
        (hash_actor(wrapper), wrapped_erc20, 1),
        (hash_actor(token), token_erc20, 0),
    ];
    let mut manager = VftManager::new(remoting.clone());
    for target in [old, new] {
        manager
            .update_erc_20_manager_address(work.intent.erc20_manager.0.into())
            .with_gas_limit(gas)
            .send_recv(target)
            .await?;
        manager
            .map_vara_to_eth_address(wrapper, wrapped_erc20.0.into(), TokenSupply::Gear)
            .with_gas_limit(gas)
            .send_recv(target)
            .await?;
        manager
            .map_vara_to_eth_address(token, token_erc20.0.into(), TokenSupply::Ethereum)
            .with_gas_limit(gas)
            .send_recv(target)
            .await?;
        manager
            .configure_native_wrapper(Some((wrapper, wrapped_erc20.0.into())))
            .with_gas_limit(gas)
            .send_recv(target)
            .await?;
    }
    let mut admin = vft_vara_client::VftAdmin::new(remoting.clone());
    admin.pause().with_gas_limit(gas).send_recv(wrapper).await?;
    vft_vara_client::NativeEscrow::new(remoting.clone())
        .configure_manager(old)
        .with_gas_limit(gas)
        .send_recv(wrapper)
        .await?;
    admin
        .set_burner(old)
        .with_gas_limit(gas)
        .send_recv(wrapper)
        .await?;
    admin
        .resume()
        .with_gas_limit(gas)
        .send_recv(wrapper)
        .await?;
    let mut token_admin = vft_client::VftAdmin::new(remoting.clone());
    token_admin
        .set_minter(old)
        .with_gas_limit(gas)
        .send_recv(token)
        .await?;
    token_admin
        .set_burner(old)
        .with_gas_limit(gas)
        .send_recv(token)
        .await?;
    work.intent.initial_role_pins = vec![
        read_role_pin(&api, hash_actor(wrapper), true, None).await?,
        read_role_pin(&api, hash_actor(token), false, None).await?,
    ];
    manager
        .begin_cutover(new, work.intent.boundary.slot)
        .with_gas_limit(gas)
        .send_recv(old)
        .await?;
    let (draining, _) = status(&api, hash_actor(old)).await?;
    work.intent.config_fingerprint = H256(
        draining
            .config_fingerprint
            .context("source fingerprint missing")?
            .0,
    );
    fs::create_dir(&work.intent.old_transactions)?;
    fs::write(
        work.intent.old_transactions.join("state.json"),
        serde_json::to_vec(&seal::OriginalJournal {
            schema_version: 7,
            runtime_identity: Some(work.intent.old_identity.clone()),
            predecessor_cutover: None,
            transactions: Default::default(),
            completed: Default::default(),
            failed: Default::default(),
        })?,
    )?;
    let path = work_path(&directory);
    save(&path, &work)?;
    admin.pause().with_gas_limit(gas).send_recv(wrapper).await?;
    assert!(
        drain_source(&api, &eth, &path).await.is_err(),
        "paused pre-retirement wrapper must HOLD"
    );
    admin
        .resume()
        .with_gas_limit(gas)
        .send_recv(wrapper)
        .await?;
    manager
        .prepare_cutover(old, work.intent.boundary.slot)
        .with_gas_limit(gas)
        .send_recv(new)
        .await?;
    manager
        .upgrade(new)
        .with_gas_limit(gas)
        .send_recv(old)
        .await?;
    assert_eq!(
        status(&api, hash_actor(old)).await?.0.phase,
        CutoverPhase::Retired
    );

    // Missing EL witness stops any next signing, but only after durable native reconciliation.
    let unavailable_eth = PollingEthApi::new(&primary.url).await?;
    for step in [
        CutoverStep::TokenMinter(hash_actor(token)),
        CutoverStep::TokenBurner(hash_actor(token)),
        CutoverStep::WrapperPause,
        CutoverStep::WrapperConfigureManager,
        CutoverStep::WrapperSetBurner,
        CutoverStep::WrapperResume,
    ] {
        let completed = run_cutover_call(&api, &eth, &path, step.clone(), &work.intent).await?;
        let mut interrupted = read_work(&path)?;
        let index = interrupted
            .calls
            .iter()
            .position(|call| call.step == step)
            .context("role call missing")?;
        interrupted.calls[index].original.finalized_reply = None;
        assert!(
            interrupted.retirement.is_none(),
            "exercise chain phase, not local retirement cache"
        );
        save(&path, &interrupted)?;
        drain_source(&api, &eth, &path).await?;
        assert!(handoff_roles(&api, &unavailable_eth, &path).await.is_err());
        let recovered = read_work(&path)?;
        assert_eq!(
            recovered.calls.len(),
            interrupted.calls.len(),
            "no replacement or later signature allowed"
        );
        assert!(recovered.calls[index]
            .original
            .same_request(&completed.original));
        assert_eq!(
            recovered.calls[index].original.finalized_reply,
            completed.original.finalized_reply
        );
        println!(
            "Recovered {step:?} original {} nonce {} through native handoff orchestration",
            completed.original.extrinsic_hash, completed.original.nonce
        );
    }
    handoff_roles(&api, &eth, &path).await?;
    let completed = read_work(&path)?;
    assert_eq!(completed.calls.len(), 6);
    let final_wrapper = completed
        .role_pins
        .iter()
        .find(|pin| pin.token == hash_actor(wrapper))
        .context("wrapper role pin missing")?;
    assert_eq!(final_wrapper.manager, Some(hash_actor(new)));
    assert_eq!(final_wrapper.burner, hash_actor(new));
    assert!(!final_wrapper.paused && zero_blocking(final_wrapper, completed.schema_version)?);
    let final_token = completed
        .role_pins
        .iter()
        .find(|pin| pin.token == hash_actor(token))
        .context("token role pin missing")?;
    assert_eq!(final_token.minter, hash_actor(new));
    assert_eq!(final_token.burner, hash_actor(new));
    println!(
        "Owned native role recovery evidence: {}",
        directory.display()
    );
    Ok(())
}
