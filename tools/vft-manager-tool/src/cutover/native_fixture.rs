// Native Gear authority/recovery smoke with signed LocalBeaconFixture execution finality.
#[tokio::test]
#[ignore = "requires explicitly owned loopback Gear archives and current program Wasm paths"]
async fn owned_native_retired_wrapper_reconciles_each_role_original() -> Result<()> {
    owned_native_cutover_fixture(None).await
}

#[tokio::test]
#[ignore = "requires owned loopback Gear archives, current program Wasm and CUTOVER_NATIVE_CHECKPOINT_WASM"]
async fn owned_native_finalize_recovers_roles_and_frozen_publication() -> Result<()> {
    for stop in 2..=5 {
        owned_native_cutover_fixture(Some(stop)).await?;
    }
    Ok(())
}

async fn owned_native_cutover_fixture(finalize_after: Option<usize>) -> Result<()> {
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
    let mut directory = PathBuf::from(std::env::var("CUTOVER_NATIVE_OUTPUT")?);
    if let Some(step) = finalize_after {
        directory = directory.join(format!("finalize-after-role-{step}"));
    }
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
    let (mut work, mut chain) = interrupted_coverage_fixture().await;
    let boundary_server = if finalize_after.is_some() {
        chain = native_registration_chain(hash_actor(new), work.intent.erc20_manager).await?;
        let (server, block) = NativeBoundaryRpc::new(&chain[0]).await?;
        let code = upload(&api, "CUTOVER_NATIVE_CHECKPOINT_WASM").await?;
        let root = H256::from_slice(block.tree_hash_root().as_bytes());
        let (_, checkpoint, _) = api.create_program_bytes(
            code, Uuid::now_v7().as_bytes(),
            vec![(block.slot, (block.slot, root))].encode(), gas, 0,
        ).await?;
        work.intent.old_identity.checkpoint_light_client_address = hash_actor(checkpoint);
        work.intent.old_identity.ethereum_genesis_hash = chain[0].hash;
        work.intent.old_identity.ethereum_start_block = 0;
        work.intent.boundary = CutoverBoundary {
            slot: block.slot, beacon_root: root,
            execution_number: 0, execution_hash: chain[0].hash,
        };
        work.intent.boundary_execution_timestamp = chain[0].header.timestamp;
        // This deploys the existing test-only checkpoint program, not a trusted Beacon actor.
        let beacon = BeaconClient::new(server.url.clone(), Some(Duration::from_secs(30))).await?;
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                if authenticate_boundary(&api, &beacon, &work.intent).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }).await.context("synthetic checkpoint constructor did not finalize")?;
        Some(server)
    } else {
        None
    };
    let primary = WorkEvidenceRpc::new(chain.clone()).await;
    let ethereum_witness = WorkEvidenceRpc::new(chain.clone()).await;
    if finalize_after.is_some() {
        for rpc in [&primary, &ethereum_witness] {
            *rpc.registrations.lock().unwrap() = Some((
                work.intent.erc20_manager, [hash_actor(old), hash_actor(new)],
            ));
        }
    }
    let finality_beacon = work_beacon_fixture(&chain).await?;
    let finality_store = directory.join("beacon-finality");
    let eth = PollingEthApi::new(&primary.url)
        .await?
        .with_beacon_finality(finality_beacon.config.clone(), &finality_store)
        .await?
        .with_data_witness(&ethereum_witness.url, 31337, chain[0].hash.0.into())
        .await?;
    let view = eth.verified_finalized_view().await?;
    work.schema_version = 4;
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
    if finalize_after.is_some() {
        work.intent.new_identity.ethereum_start_block = work.intent.boundary.execution_number + 1;
    }
    work.intent.new_manager_code_id = H256(manager_code.into_bytes());
    work.intent.old_transactions = directory.join("originals");
    work.intent.old_discovery = directory.join("discovery.json");
    work.intent.native_wrapper = Some(hash_actor(wrapper));
    let wrapped_erc20 = H160::repeat_byte(21);
    let token_erc20 = H160::repeat_byte(22);
    let approval = gear_common::native_wrapper::NativeWrapperApproval {
        schema_version: 1, gear_genesis_hash: genesis,
        source_manager: gear_common::native_wrapper::ManagerProgramApproval {
            actor_id: hash_actor(old), code_id: H256(manager_code.into_bytes()), wrapper_abi: ManagerWrapperAbi::CapabilityBound,
        },
        replacement_manager: gear_common::native_wrapper::ManagerProgramApproval {
            actor_id: hash_actor(new), code_id: H256(manager_code.into_bytes()), wrapper_abi: ManagerWrapperAbi::CapabilityBound,
        },
        wrapper: gear_common::native_wrapper::NativeWrapperBinding {
            actor_id: hash_actor(wrapper), ethereum_token: wrapped_erc20,
            code_id: H256(wrapper_code.into_bytes()), mode: NativeWrapperMode::HeldEscrowV2,
        },
    };
    let approval_bytes = serde_json::to_vec(&approval)?;
    let approval_path = directory.join("owned-fixture-native-approval.json");
    fs::write(&approval_path, &approval_bytes)?;
    use sha2::Digest as _;
    work.intent.native_wrapper_approval = Some(load_native_wrapper_approval(&approval_path, H256::from_slice(&sha2::Sha256::digest(&approval_bytes)))?);
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
            .configure_native_wrapper(Some(vft_manager_client::NativeWrapperConfig {
                actor_id: wrapper, ethereum_token: wrapped_erc20.0.into(), code_id: wrapper_code.into_bytes().into(),
                mode: vft_manager_client::NativeWrapperMode::HeldEscrowV2,
            }))
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
        read_role_pin(&api, hash_actor(wrapper), Some(NativeWrapperMode::HeldEscrowV2), None).await?,
        read_role_pin(&api, hash_actor(token), None, None).await?,
    ];
    if let Some(stop) = finalize_after {
        fs::create_dir(&work.intent.old_transactions)?;
        fs::write(work.intent.old_transactions.join("state.json"), serde_json::to_vec(&seal::OriginalJournal {
            schema_version: 7, runtime_identity: Some(work.intent.old_identity.clone()),
            predecessor_cutover: None, transactions: Default::default(),
            completed: Default::default(), failed: Default::default(),
        })?)?;
        fs::write(work.intent.old_transactions.join("blocks.json"), b"{}")?;
        use relayer::message_relayer::common::ethereum::block_storage::DiscoveryState;
        let discovery = DiscoveryState {
            schema_version: 2, identity: work.intent.old_identity.clone(),
            predecessor_cutover: None, start_parent_hash: None,
            discovered: None, extracted: None, pending: Default::default(),
        };
        discovery.validate()?;
        fs::write(&work.intent.old_discovery, serde_json::to_vec(&discovery)?)?;
        let path = work_path(&directory);
        save(&path, &work)?;
        run_cutover_call(&api, &eth, &path, CutoverStep::BeginCutover, &work.intent).await?;
        work = read_work(&path)?;
        work.intent.config_fingerprint = H256(status(&api, hash_actor(old)).await?.0
            .config_fingerprint.context("source fingerprint missing")?.0);
        save(&path, &work)?;
        for step in [CutoverStep::SourcePause, CutoverStep::PrepareCutover, CutoverStep::Upgrade(0)] {
            run_cutover_call(&api, &eth, &path, step, &work.intent).await?;
        }
        assert_eq!(status(&api, hash_actor(old)).await?.0.phase, CutoverPhase::Retired);
        let steps = [
            CutoverStep::TokenMinter(hash_actor(token)), CutoverStep::TokenBurner(hash_actor(token)),
            CutoverStep::WrapperPause, CutoverStep::WrapperConfigureManager,
            CutoverStep::WrapperSetBurner, CutoverStep::WrapperResume,
        ];
        for step in &steps[..=stop] {
            run_cutover_call(&api, &eth, &path, step.clone(), &work.intent).await?;
        }
        let mut interrupted = read_work(&path)?;
        assert!(interrupted.retirement.is_none(), "retirement must exist onchain before its local pin");
        let originals = interrupted.calls.clone();
        interrupted.calls.last_mut().context("role original missing")?.original.finalized_reply = None;
        save(&path, &interrupted)?;
        let connection = GearConnectionArgs {
            gear_endpoint: source.parse()?, max_reconnect_attempts: 3,
        };
        let finality_approval = directory.join("beacon-bootstrap-approval.json");
        fs::copy(&finality_beacon.approval_path, &finality_approval)?;
        let args = FinalizeCutover {
            cutover_dir: directory.clone(), new_transactions: directory.join("successor"),
            new_discovery: directory.join("successor-discovery.json"),
            ethereum_rpc: primary.url.clone(),
            ethereum_finality: EthereumFinalityArgs {
                ethereum_witness_rpc: Some(ethereum_witness.url.clone()),
                ethereum_finality_beacon_rpc: finality_beacon.beacon_rpc.clone(),
                ethereum_finality_bootstrap: finality_approval,
                ethereum_finality_bootstrap_sha256: format!("{:#x}", finality_beacon.approval_sha256),
                ethereum_finality_store: finality_store,
                ethereum_finality_proof_sources: vec![],
            },
            beacon_rpc: boundary_server.as_ref().unwrap().url.clone(),
            native_capability: NativeCapabilityArgs {
                native_wrapper_approval: Some(approval_path),
                native_wrapper_approval_sha256: Some(work.intent.native_wrapper_approval.as_ref().unwrap().approval_digest),
            },
            evm_registration_transaction: Some(chain[1].transactions[0]), sponsor_unpaid: false,
        };
        // finalize owns this lock itself; never hold an outer signer flock across its call.
        drop(_owner);
        drop(view);
        drop(eth);
        return native_finalize_restarts(&api, &connection, &args, &originals, stop == 5).await;
    }
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
    assert!(!final_wrapper.paused && zero_blocking(final_wrapper, completed.intent.wrapper_mode_for(final_wrapper.token))?);
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

// Synthetic EL authority and a real trie-bound registration original. This is not
// execution of an EVM governance transaction and is never production qualification.
async fn native_registration_chain(successor: H256, emitter: H160) -> Result<Vec<CoveredBlock>> {
    use alloy::{
        consensus::{Receipt, ReceiptEnvelope, SignableTransaction, TxLegacy},
        network::TxSigner,
        primitives::{Address, Log, TxKind},
        signers::local::PrivateKeySigner,
    };
    let genesis = alloy::consensus::Header {
        extra_data: Uuid::now_v7().as_bytes().to_vec().into(),
        gas_limit: 30_000_000,
        transactions_root: alloy_primitives::keccak256([0x80]),
        receipts_root: alloy_primitives::keccak256([0x80]),
        ..Default::default()
    };
    let first = CoveredBlock {
        number: 0, hash: H256(genesis.hash_slow().0),
        parent_hash: H256::zero(), receipts_root: H256(genesis.receipts_root.0),
        header: genesis, transactions: vec![], raw_transactions: vec![], receipts: vec![],
    };
    let log = Log {
        address: Address::from(emitter.0),
        data: IERC20Manager::VftManagerAdded { vftManager: successor.0.into() }.encode_log_data(),
    };
    let envelope = ReceiptEnvelope::Legacy(Receipt {
        status: true.into(), cumulative_gas_used: 21_000, logs: vec![log.clone()],
    }.with_bloom());
    let signer = PrivateKeySigner::from_bytes(&B256::from([17; 32]))?;
    let mut transaction = TxLegacy {
        chain_id: Some(31337), gas_price: 1, gas_limit: 100_000,
        to: TxKind::Call(log.address),
        ..Default::default()
    };
    let signature = signer.sign_transaction(&mut transaction).await?;
    let transaction = TxEnvelope::Legacy(transaction.into_signed(signature));
    let raw = transaction.encoded_2718();
    let hash = alloy_primitives::keccak256(&raw);
    let header = alloy::consensus::Header {
        number: 1, parent_hash: first.hash.0.into(), timestamp: 12,
        transactions_root: alloy::consensus::proofs::calculate_transaction_root(&[transaction]),
        receipts_root: alloy::consensus::proofs::calculate_receipt_root(std::slice::from_ref(&envelope)),
        gas_used: 21_000, gas_limit: 30_000_000,
        ..Default::default()
    };
    let block_hash = header.hash_slow();
    let mut receipt = serde_json::to_value(&envelope)?;
    let fields = serde_json::json!({
        "transactionHash":hash, "transactionIndex":"0x0", "blockHash":block_hash,
        "blockNumber":"0x1", "gasUsed":"0x5208", "effectiveGasPrice":"0x1",
        "from":signer.address(), "to":log.address, "contractAddress":null,
        "logs":[{"address":log.address, "topics":log.data.topics(), "data":log.data.data,
            "transactionHash":hash, "transactionIndex":"0x0", "blockHash":block_hash,
            "blockNumber":"0x1", "logIndex":"0x0", "removed":false}]
    });
    receipt.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
    let second = CoveredBlock {
        number: 1, hash: H256(block_hash.0), parent_hash: first.hash,
        receipts_root: H256(header.receipts_root.0), header,
        transactions: vec![hash], raw_transactions: vec![raw],
        receipts: vec![serde_json::from_value(receipt)?],
    };
    for block in [&first, &second] {
        validate_covered_block(block)?;
    }
    Ok(vec![first, second])
}

struct NativeBoundaryRpc {
    url: String,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for NativeBoundaryRpc {
    fn drop(&mut self) { self.server.abort(); }
}
impl NativeBoundaryRpc {
    async fn new(execution: &CoveredBlock) -> Result<(Self, ethereum_common::beacon::electra::Block)> {
        use serde_json::json;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // Reuse the existing full-block JSON wire shape, not a second SSZ codec.
        // Every chain-specific field below is synthetic; the mock actor pins its exact root.
        let mut response: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/src/historical_proxy/headers/holesky-block-2_498_456.json"
        ))?;
        response["version"] = json!("electra");
        let block = &mut response["data"]["message"];
        block["slot"] = json!("32");
        block["parent_root"] = json!(H256::zero());
        block["state_root"] = json!(H256::zero());
        let body = &mut block["body"];
        for field in ["attestations", "attester_slashings", "proposer_slashings", "deposits",
            "voluntary_exits", "bls_to_execution_changes", "blob_kzg_commitments"] {
            body[field] = json!([]);
        }
        body["execution_requests"] = json!({"deposits":[], "withdrawals":[], "consolidations":[]});
        let payload = &mut body["execution_payload"];
        payload["block_number"] = json!(execution.number.to_string());
        payload["block_hash"] = json!(execution.hash);
        payload["parent_hash"] = json!(execution.parent_hash);
        payload["receipts_root"] = json!(execution.receipts_root);
        payload["state_root"] = json!(execution.header.state_root);
        payload["timestamp"] = json!(execution.header.timestamp.to_string());
        payload["transactions"] = json!([]);
        payload["withdrawals"] = json!([]);
        let block = serde::Deserialize::deserialize(&*block)
            .context("decoding synthetic full Beacon block")?;
        let response = serde_json::to_vec(&response)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let response = response.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    loop {
                        let mut chunk = [0; 2048];
                        let count = socket.read(&mut chunk).await.unwrap();
                        if count == 0 { return; }
                        bytes.extend_from_slice(&chunk[..count]);
                        if bytes.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                    }
                    assert!(bytes.starts_with(b"GET /eth/v2/beacon/blocks/32 HTTP/"));
                    let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
                    socket.write_all(headers.as_bytes()).await.unwrap();
                    socket.write_all(&response).await.unwrap();
                });
            }
        });
        Ok((Self { url, server }, block))
    }
}

async fn native_fixture_nonce(api: &GearApi) -> Result<u64> {
    let signer: gsdk::signer::Signer = api.clone().into();
    Ok(signer.api().tx().account_nonce(signer.account_id()).await?)
}

async fn native_finalize_restarts(
    api: &GearApi,
    connection: &GearConnectionArgs,
    args: &FinalizeCutover,
    originals: &[FinalizedCutoverCall],
    exercise_crashes: bool,
) -> Result<()> {
    let path = work_path(&args.cutover_dir);
    // An explicit fixture-owned collision stops activation *after* real finalize
    // has authenticated and published every immutable report and marker.
    fs::create_dir(&args.new_transactions)?;
    let stop = args.new_transactions.join("fixture-activation-stop");
    fs::write(&stop, b"owned test barrier, not a successor journal")?;
    async fn reaches_activation(
        connection: &GearConnectionArgs, api: &GearApi, args: &FinalizeCutover,
    ) -> Result<()> {
        let error = finalize(connection, api, args).await
            .expect_err("fixture collision must prevent activation");
        ensure!(format!("{error:#}").contains("successor transaction path is not new and unused"),
            "finalize failed before its activation barrier: {error:#}");
        Ok(())
    }
    let before_recovery = native_fixture_nonce(api).await?;
    reaches_activation(connection, api, args).await?;
    let ready = read_work(&path)?;
    for (saved, original) in ready.calls.iter().zip(originals) {
        assert!(saved.original.same_request(&original.original));
        assert_eq!(saved.original.finalized_reply, original.original.finalized_reply);
    }
    assert_eq!(ready.calls.len(), 11, "one original per lifecycle/role call through Ready");
    assert_eq!(status(api, ready.intent.new_identity.vft_manager_address).await?.0.phase, CutoverPhase::Ready);
    assert!(ready.pending_seal.is_some());
    let nonce = native_fixture_nonce(api).await?;
    assert_eq!(nonce, before_recovery + u64::try_from(ready.calls.len() - originals.len())?,
        "only genuinely missing lifecycle steps may allocate nonces");
    let ready_bytes = fs::read(&path)?;
    let seal_path = args.cutover_dir.join("cutover.json");
    let sealed = seal::verify_markers(&seal_path)?.0;
    let artifacts = [
        sealed.coverage.path.clone(), sealed.lifecycle_evidence.path.clone(), seal_path.clone(),
        seal::marker_path(&ready.intent.old_transactions),
        seal::marker_path(&ready.intent.old_discovery),
    ];
    let exact = artifacts.iter().map(fs::read).collect::<std::io::Result<Vec<_>>>()?;
    let evidence = args.cutover_dir.join("restart-evidence");
    fs::create_dir(&evidence)?;
    fs::write(evidence.join("ready-work.json"), &ready_bytes)?;
    for (index, bytes) in exact.iter().enumerate() {
        fs::write(evidence.join(format!("uninterrupted-artifact-{index}.json")), bytes)?;
    }
    let old_files = [&sealed.old_state, &sealed.old_blocks, &sealed.old_discovery];
    if exercise_crashes {
        for prefix in 0..artifacts.len() {
            for state in 0..=2 {
                // Reconstruct only task-owned crash images. Originals, chain state,
                // finality archive and frozen projection are never rewritten.
                for file in &artifacts {
                    if file.exists() { fs::remove_file(file)?; }
                    if pending_path(file).exists() { fs::remove_file(pending_path(file))?; }
                }
                fs::write(&path, &ready_bytes)?;
                for index in 0..prefix { fs::write(&artifacts[index], &exact[index])?; }
                if state != 0 {
                    fs::write(pending_path(&artifacts[prefix]), &exact[prefix])?;
                }
                if state == 2 { fs::hard_link(pending_path(&artifacts[prefix]), &artifacts[prefix])?; }
                reaches_activation(connection, api, args).await?;
                for (file, bytes) in artifacts.iter().zip(&exact) {
                    assert_eq!(fs::read(file)?, *bytes, "crash {prefix}/{state}: immutable bytes changed");
                    assert!(!pending_path(file).exists());
                }
                assert_eq!(fs::read(&path)?, ready_bytes, "frozen Work changed on restart");
                assert_eq!(native_fixture_nonce(api).await?, nonce, "restart signed a replacement");
                for file in &old_files { assert_eq!(seal::sealed_file(&file.path)?, **file); }
                println!("full finalize recovered immutable boundary {prefix}, state={state}, nonce={nonce}");
            }
        }
        // A historical report can precede pending_seal. It owns the checkpoint.
        for file in &artifacts[1..] { fs::remove_file(file)?; }
        let mut historical = ready.clone();
        historical.pending_seal = None;
        fs::write(&path, serde_json::to_vec(&historical)?)?;
        reaches_activation(connection, api, args).await?;
        assert_eq!(fs::read(&path)?, ready_bytes);
        assert_eq!(native_fixture_nonce(api).await?, nonce);
        for (index, artifact) in artifacts.iter().enumerate() {
            let temporary = pending_path(artifact);
            fs::write(&temporary, b"conflicting owned crash image")?;
            let error = finalize(connection, api, args).await.expect_err("conflicting immutable temporary must HOLD");
            assert_eq!(fs::read(&temporary)?, b"conflicting owned crash image");
            assert_eq!(fs::read(&path)?, ready_bytes);
            assert_eq!(native_fixture_nonce(api).await?, nonce);
            for (file, bytes) in artifacts.iter().zip(&exact) { assert_eq!(fs::read(file)?, *bytes); }
            fs::write(evidence.join(format!("artifact-{index}-hold.txt")), format!("{error:#}"))?;
            fs::remove_file(temporary)?;
        }

        for field in ["identity", "checkpoint"] {
            let mut changed = ready.clone();
            if field == "identity" {
                changed.intent.new_manager_code_id = H256::repeat_byte(77);
            } else {
                checkpoint_header_mut(changed.ethereum_authentication.as_mut().unwrap()).timestamp += 1;
            }
            let changed_bytes = serde_json::to_vec(&changed)?;
            fs::write(&path, &changed_bytes)?;
            let error = finalize(connection, api, args).await.expect_err("frozen mutation must HOLD");
            assert_eq!(fs::read(&path)?, changed_bytes, "HOLD rewrote conflicting Work");
            assert_eq!(native_fixture_nonce(api).await?, nonce, "HOLD signed a request");
            for (file, bytes) in artifacts.iter().zip(&exact) { assert_eq!(fs::read(file)?, *bytes); }
            fs::write(evidence.join(format!("{field}-hold.txt")), format!("{error:#}"))?;
            fs::write(&path, &ready_bytes)?;
        }
    }
    fs::remove_file(stop)?;
    finalize(connection, api, args).await?;
    let active = read_work(&path)?;
    assert_eq!(active.calls.len(), ready.calls.len() + 1);
    for (saved, original) in active.calls.iter().zip(&ready.calls) {
        assert_eq!(serde_json::to_vec(saved)?, serde_json::to_vec(original)?);
    }
    assert_eq!(native_fixture_nonce(api).await?, nonce + 1);
    assert_eq!(status(api, ready.intent.new_identity.vft_manager_address).await?.0.phase, CutoverPhase::Active);
    let active_nonce = native_fixture_nonce(api).await?;
    finalize(connection, api, args).await?;
    assert_eq!(native_fixture_nonce(api).await?, active_nonce);
    for (file, bytes) in artifacts.iter().zip(&exact) { assert_eq!(fs::read(file)?, *bytes); }
    if exercise_crashes {
        let activation_path = args.cutover_dir.join("cutover-activation.json");
        let activation_bytes = fs::read(&activation_path)?;
        let active_work = fs::read(&path)?;
        let successor_paths = [
            args.new_transactions.join("state.json"),
            args.new_transactions.join("blocks.json"),
            args.new_discovery.clone(),
        ];
        let successor_bytes = successor_paths.iter().map(fs::read)
            .collect::<std::io::Result<Vec<_>>>()?;
        fs::write(evidence.join("original-activation.json"), &activation_bytes)?;
        for linked in [false, true] {
            fs::remove_file(&activation_path)?;
            fs::write(pending_path(&activation_path), &activation_bytes)?;
            File::open(pending_path(&activation_path))?.sync_all()?;
            if linked {
                fs::hard_link(pending_path(&activation_path), &activation_path)?;
            }
            finalize(connection, api, args).await?;
            assert_eq!(fs::read(&activation_path)?, activation_bytes,
                "restart must retain the original unpause and status pin");
            assert!(!pending_path(&activation_path).exists());
            assert_eq!(fs::read(&path)?, active_work);
            assert_eq!(native_fixture_nonce(api).await?, active_nonce,
                "activation recovery must not sign again");
            for (file, bytes) in successor_paths.iter().zip(&successor_bytes) {
                assert_eq!(fs::read(file)?, *bytes, "activated journals changed");
            }
        }
        for conflict in ["final-temp", "seal", "original", "pin", "status", "archived"] {
            let mut changed: (H256, FinalizedCutoverCall, FinalizedCutoverStatus) =
                serde_json::from_slice(&activation_bytes)?;
            match conflict {
                "final-temp" | "seal" => changed.0 = H256::repeat_byte(99),
                "original" => changed.1.original.raw_extrinsic.push(0),
                "pin" => changed.2.block_hash = H256::repeat_byte(99),
                "status" => {
                    let mut status = CutoverStatus::decode(&mut changed.2.status.as_slice())?;
                    status.phase = CutoverPhase::Ready;
                    changed.2.status = status.encode();
                }
                "archived" => changed.1.archived_no_effect = true,
                _ => unreachable!(),
            }
            let changed_bytes = serde_json::to_vec(&changed)?;
            if conflict != "final-temp" {
                fs::remove_file(&activation_path)?;
            }
            fs::write(pending_path(&activation_path), &changed_bytes)?;
            let error = finalize(connection, api, args).await
                .expect_err("conflicting activation evidence must HOLD");
            assert_eq!(fs::read(pending_path(&activation_path))?, changed_bytes);
            if conflict == "final-temp" {
                assert_eq!(fs::read(&activation_path)?, activation_bytes);
            } else {
                assert!(!activation_path.exists(), "unauthenticated evidence was published");
            }
            assert_eq!(fs::read(&path)?, active_work);
            assert_eq!(native_fixture_nonce(api).await?, active_nonce);
            for (file, bytes) in successor_paths.iter().zip(&successor_bytes) {
                assert_eq!(fs::read(file)?, *bytes);
            }
            fs::write(evidence.join(format!("activation-{conflict}-hold.txt")), format!("{error:#}"))?;
            fs::remove_file(pending_path(&activation_path))?;
            if !activation_path.exists() {
                fs::write(&activation_path, &activation_bytes)?;
            }
        }
        finalize(connection, api, args).await?;
        assert_eq!(fs::read(&activation_path)?, activation_bytes);
        assert_eq!(native_fixture_nonce(api).await?, active_nonce);
    }
    println!("Full native finalize: {} retained originals, Ready nonce {nonce}, Active nonce {active_nonce}; evidence {}; synthetic checkpoint/EL, NOT Beacon/mainnet qualification",
        active.calls.len(), evidence.display());
    Ok(())
}
