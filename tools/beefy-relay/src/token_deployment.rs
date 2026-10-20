use alloy::{
    primitives::{Address, B256, U256},
    providers::Provider,
    sol,
};
use anyhow::{anyhow, ensure, Result};
use beefy_relay::{
    root_publication::{decode_hash, ensure_source_identity, lock_token_deployment},
    source::Source,
};
use bridging_payment_client::traits::{BridgingPayment, BridgingPaymentFactory};
use checkpoint_light_client_client::traits::ServiceCheckpointFor;
use eth_events_electra_client::traits::{EthEventsElectraFactory, EthereumEventClient};
use gclient::GearApi;
use gear_core::{
    ids::{prelude::*, ActorId, CodeId},
    program::ProgramState,
};
use gsdk::{
    ext::subxt::{config::polkadot::PolkadotExtrinsicParamsBuilder, utils::H256 as GearHash},
    gear::{self, runtime_types::gear_common::event::MessageEntry},
};
use historical_proxy_client::traits::{HistoricalProxy, HistoricalProxyFactory};
use parity_scale_codec::Decode;
use sails_rs::{
    calls::*,
    errors::{Result as SailsResult, RtlError},
    gclient::calls::*,
    prelude::{GasUnit, ValueUnit, H160},
};
use serde_json::{json, Value};
use sp_core::H256;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use vft_client::traits::{VftAdmin, VftFactory, VftMetadata};
use vft_manager_client::{
    traits::{VftManager, VftManagerFactory},
    Config, InitConfig, TokenSupply,
};
use vft_vara_client::{
    traits::{NativeEscrow as _, VftAdmin as _, VftExtension as _, VftVaraFactory},
    Mainnet,
};

const BUILTIN: &str = "f2816ced0b15749595392d3a18b5a2363d6fefe5b3b6153739f218151b7acdbf";
const HOODI_CHAIN_ID: u64 = 560_048;

sol! {
    #[sol(rpc)]
    interface ConfiguredERC20Manager {
        function governanceAdmin() external view returns (address);
        function governancePauser() external view returns (address);
        function messageQueue() external view returns (address);
        function totalVftManagers() external view returns (uint256);
        function vftManagers() external view returns (bytes32[] memory);
        function isVftManager(bytes32 vftManager) external view returns (bool);
        function totalTokens() external view returns (uint256);
        function tokens() external view returns (address[] memory);
        function getTokenType(address token) external view returns (uint8);
        function totalBridgingPayments() external view returns (uint256);
        function bridgingPayments() external view returns (address[] memory);
        function hasRole(bytes32 role, address account) external view returns (bool);
        function paused() external view returns (bool);
    }
    #[sol(rpc)]
    interface ConfiguredERC20 {
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
    }
    #[sol(rpc)]
    interface ConfiguredBridgingPayment {
        function erc20Manager() external view returns (address);
        function fee() external view returns (uint256);
        function owner() external view returns (address);
    }
}
pub async fn anchor(source_rpc: &str, witness_rpc: &str) -> Result<()> {
    use beefy_relay::source::Source;
    use gear_rpc_client::GearApi as SourceApi;

    let (mut source, witness) = Source::connect_pair(
        SourceApi::new(source_rpc, 3).await?,
        SourceApi::new(witness_rpc, 3).await?,
    )
    .await?;
    ensure!(
        source.source_genesis == witness.source_genesis
            && source.bridge_domain == witness.bridge_domain
            && source.mmr_start_block == witness.mmr_start_block
            && source.beefy_activation_block == witness.beefy_activation_block,
        "source identity differs between authorities"
    );
    let target = source.seek_to_finalized().await?;
    let signed = source.next_commitment().await?;
    ensure!(
        signed.block >= target && u64::from(signed.block) > source.mmr_start_block,
        "bootstrap is not a new finalized signed commitment"
    );
    let witness_head = witness
        .api
        .block_hash_to_number(witness.api.latest_finalized_block().await?)
        .await?;
    ensure!(
        witness_head >= signed.block,
        "witness has not finalized bootstrap"
    );
    let hash = gsdk::ext::subxt::utils::H256(signed.block_hash);
    ensure!(
        hash == witness.api.block_number_to_hash(signed.block).await?,
        "finalized source hashes differ"
    );
    let (current, next) = witness.checkpoint_at_hash(hash).await?;
    ensure!(
        current == signed.current && next == signed.next,
        "signed BEEFY validator sets differ between authorities"
    );
    let proof = source.proof(signed.block - 1, &signed).await?;
    let second = witness.proof(signed.block - 1, &signed).await?;
    ensure!(
        proof.snapshot == second.snapshot && proof.raw_leaf == second.raw_leaf,
        "MMR/bootstrap snapshots differ between authorities"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "sourceGenesis": format!("0x{}", hex::encode(source.source_genesis)),
            "bridgeDomain": format!("0x{}", hex::encode(source.bridge_domain)),
            "mmrStartBlock": source.mmr_start_block,
            "beefyActivationBlock": source.beefy_activation_block,
            "domainBindingBlock": source.identity.domain_binding_block,
            "sourceIdentity": source.identity,
            "block": signed.block,
            "blockHash": format!("{hash:#x}"),
            "sourceTimestampMs": proof.snapshot.source_timestamp_ms,
            "freshnessSourceBlock": proof.source,
            "mmrRoot": format!("0x{}", hex::encode(signed.validated.mmr_root)),
            "signedCommitmentScale": format!("0x{}", hex::encode(&signed.raw)),
            "current": current,
            "next": next
        }))?
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn manifest(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    anchor_path: &Path,
    stack_path: &Path,
    client: &str,
    verifier: &str,
    queue: &str,
    manager: &str,
    output: &Path,
    local_rehearsal: bool,
    owned_fixture_network: Option<&Path>,
) -> Result<()> {
    use beefy_relay::{ethereum::Ethereum, follower::checkpoint_matches_source, source::Source};
    use gear_rpc_client::GearApi as SourceApi;
    use gsdk::ext::subxt::utils::H256;

    let fixture = owned_fixture_network
        .map(beefy_relay::ethereum::OwnedFixtureNetwork::read)
        .transpose()?;
    ensure!(
        !(local_rehearsal && fixture.is_some()),
        "owned fixture is not a local rehearsal"
    );
    if let Some(network) = &fixture {
        network.validate_endpoints(source_rpc, witness_rpc, ethereum_rpc)?;
    }
    let chain_id = fixture
        .as_ref()
        .map_or(HOODI_CHAIN_ID, |network| network.ethereum_chain_id);
    ensure!(
        source_rpc != witness_rpc,
        "two independent source endpoints required"
    );
    ensure!(
        if local_rehearsal || fixture.is_some() {
            beefy_relay::local_source_rpc(ethereum_rpc)
        } else {
            ethereum_rpc.starts_with("wss://") || ethereum_rpc.starts_with("https://")
        },
        "Hoodi requires HTTPS or WSS; explicit local rehearsal requires a loopback websocket"
    );
    let anchor: Value = serde_json::from_slice(&fs::read(anchor_path)?)?;
    let stack: Value = serde_json::from_slice(&fs::read(stack_path)?)?;
    ensure!(
        stack["lane"] == "beefy-token-hoodi",
        "not the isolated Gear token stack"
    );
    let gear_manager = actor(
        stack["programs"]["vftManager"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Gear manager missing"))?,
    )?;
    ensure!(
        stack["programs"]["vftManager"]["status"] == "active",
        "Gear manager is not active"
    );
    let (source, witness) = Source::connect_pair(
        SourceApi::new(source_rpc, 3).await?,
        SourceApi::new(witness_rpc, 3).await?,
    )
    .await?;
    source.validate_attachment(&anchor).await?;
    ensure!(
        source.source_genesis == witness.source_genesis
            && source.bridge_domain == witness.bridge_domain
            && source.mmr_start_block == witness.mmr_start_block
            && source.beefy_activation_block == witness.beefy_activation_block
            && anchor["sourceGenesis"] == format!("0x{}", hex::encode(source.source_genesis))
            && anchor["bridgeDomain"] == format!("0x{}", hex::encode(source.bridge_domain))
            && anchor["mmrStartBlock"] == source.mmr_start_block
            && anchor["beefyActivationBlock"] == source.beefy_activation_block,
        "bootstrap source identity differs between Gear authorities"
    );
    let block: u32 = anchor["block"]
        .as_u64()
        .ok_or_else(|| anyhow!("anchor block missing"))?
        .try_into()?;
    ensure!(block > 1, "invalid signed bootstrap block");
    let raw = hex::decode(
        anchor["signedCommitmentScale"]
            .as_str()
            .ok_or_else(|| anyhow!("signed anchor missing"))?
            .trim_start_matches("0x"),
    )?;
    let signed = source.recapture(block, raw).await?;
    let witness_finalized = witness
        .api
        .block_hash_to_number(witness.api.latest_finalized_block().await?)
        .await?;
    ensure!(
        witness_finalized >= block,
        "witness has not finalized bootstrap"
    );
    let hash = H256(signed.block_hash);
    ensure!(
        witness.api.block_number_to_hash(block).await? == hash
            && anchor["blockHash"] == format!("{hash:#x}")
            && anchor["mmrRoot"] == format!("0x{}", hex::encode(signed.validated.mmr_root))
            && anchor["current"] == serde_json::to_value(&signed.current)?
            && anchor["next"] == serde_json::to_value(&signed.next)?,
        "signed anchor is not the canonical witnessed bootstrap"
    );
    let (current, next) = witness.checkpoint_at_hash(hash).await?;
    let (proof, second) = tokio::try_join!(
        source.proof(block - 1, &signed),
        witness.proof(block - 1, &signed)
    )?;
    ensure!(
        current == signed.current
            && next == signed.next
            && proof.snapshot == second.snapshot
            && proof.raw_leaf == second.raw_leaf
            && anchor["freshnessSourceBlock"] == proof.source
            && anchor["sourceTimestampMs"] == proof.snapshot.source_timestamp_ms,
        "bootstrap proof or validator set differs between Gear authorities"
    );
    let addresses = json!({
        "chainId": chain_id, "client": client, "verifier": verifier,
        "queue": queue, "receiver": manager,
    });
    let mut ethereum_manifest =
        Ethereum::token_manifest(ethereum_rpc, wallet, &addresses, fixture.as_ref()).await?;
    ethereum_manifest["sourceGenesis"] = anchor["sourceGenesis"].clone();
    let ethereum = Ethereum::connect(
        ethereum_rpc,
        wallet,
        &ethereum_manifest,
        fixture.as_ref(),
        beefy_relay::ethereum::PublicEthereumNetwork::Hoodi,
    )
    .await?;
    ethereum
        .verify_token_bindings(gear_manager.into_bytes())
        .await?;
    let checkpoint = ethereum.checkpoint().await?;
    ensure!(
        checkpoint.block == u64::from(block)
            && checkpoint.root == [0; 32]
            && checkpoint_matches_source(
                &checkpoint,
                [0; 32],
                proof.snapshot.source_timestamp_ms,
                &signed.current,
                &signed.next
            ),
        "EVM token client bootstrap does not match signed source checkpoint"
    );
    ensure!(
        ethereum_manifest["sourceGenesis"] == anchor["sourceGenesis"]
            && ethereum_manifest["bridgeDomain"] == anchor["bridgeDomain"]
            && ethereum_manifest["mmrStartBlock"] == anchor["mmrStartBlock"],
        "EVM token client source identity differs from Gear"
    );
    let source_domain: [u8; 32] = hex::decode(
        ethereum_manifest["sourceDomain"]
            .as_str()
            .ok_or_else(|| anyhow!("sourceDomain missing"))?
            .trim_start_matches("0x"),
    )?
    .try_into()
    .map_err(|_| anyhow!("invalid sourceDomain"))?;
    let queue_address: [u8; 20] = hex::decode(
        ethereum_manifest["queue"]
            .as_str()
            .ok_or_else(|| anyhow!("queue missing"))?
            .trim_start_matches("0x"),
    )?
    .try_into()
    .map_err(|_| anyhow!("invalid queue"))?;
    ensure!(
        beefy_relay::bridge_domain(source_domain, chain_id, queue_address)? == source.bridge_domain,
        "source domain or destination differs from source genesis lane"
    );
    let mut deployment = json!({
        "mode": "hoodi-token-stack", "localRehearsal": local_rehearsal, "manager": ethereum_manifest["receiver"],
        "gearManager": format!("0x{}", hex::encode(gear_manager.into_bytes())),
        "anchor": anchor, "ethereum": ethereum_manifest,
    });
    if let Some(network) = &fixture {
        deployment["ownedFixtureNetwork"] = serde_json::to_value(network)?;
    }
    if !local_rehearsal {
        deployment["ethereumConfiguration"] =
            inspect_ethereum_configuration(&ethereum, gear_manager.into_bytes()).await?;
    }
    let temporary = output.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, &deployment)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    let published = fs::hard_link(&temporary, output);
    fs::remove_file(&temporary)?;
    published?;
    println!("Token deployment manifest pinned at {}", output.display());
    Ok(())
}
fn actor(text: &str) -> Result<ActorId> {
    let bytes = hex::decode(text.strip_prefix("0x").unwrap_or(text))?;
    Ok(ActorId::from(<[u8; 32]>::try_from(bytes).map_err(
        |_| anyhow!("expected a 32-byte Gear actor ID"),
    )?))
}

fn save(output: &Path, state: &Value) -> Result<()> {
    let temp = output.join("token-stack.json.tmp");
    let mut file = File::create(&temp)?;
    serde_json::to_writer_pretty(&mut file, state)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temp, output.join("token-stack.json"))?;
    File::open(output)?.sync_all()?;
    Ok(())
}

fn lock_token_stack(path: &Path) -> Result<File> {
    let parent = if path.exists() {
        fs::canonicalize(state_directory(&fs::canonicalize(path)?))?
    } else {
        fs::canonicalize(state_directory(path))?
    };
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join("token-stack.lock"))?;
    lock.try_lock()
        .map_err(|error| anyhow!("another setup writer owns token-stack.json: {error}; HOLD"))?;
    Ok(lock)
}

// Reload before every mutation: signed message records must survive status changes.
fn intent(
    output: &Path,
    component: &str,
    binary: &[u8],
) -> Result<(ActorId, bool, Vec<u8>)> {
    let mut state = read_config_stack(&output.join("token-stack.json"))?;
    let genesis = state["sourceGenesis"]
        .as_str()
        .ok_or_else(|| anyhow!("source genesis missing"))?;
    let salt = format!("beefy-token-hoodi-{genesis}-{component}").into_bytes();
    let code_id = CodeId::generate(binary);
    let id = ActorId::generate_from_user(code_id, &salt);
    let address = format!("0x{}", hex::encode(id.into_bytes()));
    if state["configuration"].is_null() {
        state["configuration"] = json!({"version":1,"status":"preparing","actions":{}});
    }
    if state["programs"][component].is_null() {
        state["programs"][component] = json!({"id":address,"codeId":code_hex(code_id),"status":"unsigned","salt":format!("0x{}",hex::encode(&salt))});
    }
    ensure!(
        state["programs"][component]["id"] == address
            && state["programs"][component]["salt"] == format!("0x{}", hex::encode(&salt)),
        "{component} artifact or salt changed; HOLD"
    );
    let status = state["programs"][component]["status"]
        .as_str()
        .ok_or_else(|| anyhow!("{component} status missing"))?;
    let signed = state["configuration"]["actions"][format!("constructor.{component}")]["messages"]
        .as_array()
        .is_some_and(|messages| !messages.is_empty());
    let activate = match status {
        "unsigned" => true,
        "pending" => signed,
        "active" | "initializing" => false,
        _ => anyhow::bail!("{component} unknown constructor status; HOLD"),
    };
    save(output, &state)?;
    Ok((id, activate, salt))
}

fn set_program_status(output: &Path, component: &str, status: &str) -> Result<()> {
    let mut state = read_config_stack(&output.join("token-stack.json"))?;
    ensure!(
        !state["programs"][component].is_null(),
        "program {component} has no intent"
    );
    state["programs"][component]["status"] = json!(status);
    save(output, &state)
}

fn finalized_code_result(
    result: std::result::Result<Vec<u8>, gsdk::Error>,
    binary: &[u8],
) -> Result<bool> {
    match result {
        Ok(existing) => {
            ensure!(existing == binary, "finalized code bytes differ; HOLD");
            Ok(true)
        }
        Err(gsdk::Error::StorageEntryNotFound) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn code(api: &GearApi, binary: &[u8]) -> Result<CodeId> {
    let id = CodeId::generate(binary);
    let signer: gsdk::signer::Signer = api.clone().into();
    let at = api.last_block_hash().await?;
    if finalized_code_result(
        signer.api().original_code_storage_at(id, Some(at)).await,
        binary,
    )? {
        return Ok(id);
    }
    api.upload_code(binary).await?;
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let at = api.last_block_hash().await?;
            if finalized_code_result(
                signer.api().original_code_storage_at(id, Some(at)).await,
                binary,
            )? {
                return Ok(id);
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("uploaded code is not finalized; HOLD"))?
}

pub async fn prepare(
    source_rpc: &str,
    expected_genesis: &str,
    suri: &str,
    checkpoint: &str,
    slot: u64,
    checkpoint_hash: &str,
    output: &Path,
) -> Result<()> {
    ensure!(
        beefy_relay::local_source_rpc(source_rpc),
        "public dev keys: source RPC must be loopback"
    );
    fs::create_dir_all(output)?;
    let path = output.join("token-stack.json");
    let _stack_lock = lock_token_stack(&path)?;
    let api = GearApi::builder()
        .suri(suri)
        .uri(source_rpc)
        .build()
        .await?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let genesis = format!(
        "{:#x}",
        signer
            .api()
            .legacy()
            .chain_get_block_hash(Some(0u32.into()))
            .await?
            .ok_or_else(|| anyhow!("source genesis block missing"))?
    );
    let checkpoint = actor(checkpoint)?;
    ensure!(genesis == expected_genesis, "wrong source genesis");
    let _signer_owner = gear_common::signer_ownership::lock_signer(
        decode_hash(&json!(genesis), "source genesis")?.into(),
        <[u8; 32]>::from(api.account_id().clone()),
    )?;
    let expected_hash = hex::decode(
        checkpoint_hash
            .strip_prefix("0x")
            .unwrap_or(checkpoint_hash),
    )?;
    ensure!(
        expected_hash.len() == 32,
        "expected 32-byte checkpoint root"
    );
    let remoting = GClientRemoting::new(api.clone());
    let mut finalized = api.last_block_hash().await?;
    let on_chain = checkpoint_light_client_client::ServiceCheckpointFor::new(remoting.clone())
        .get(slot)
        .at_block(sails_block_hash(finalized))
        .recv(checkpoint)
        .await
        .map_err(|e| anyhow!("checkpoint query failed: {e:?}"))?
        .map_err(|e| anyhow!("checkpoint not available: {e:?}"))?;
    ensure!(
        on_chain.0 == slot && on_chain.1.as_ref() == expected_hash,
        "checkpoint trust anchor does not match live Gear state"
    );
    if path.exists() {
        let state: Value = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(
            state["lane"] == "beefy-token-hoodi"
                && state["sourceGenesis"] == genesis
                && state["checkpoint"] == format!("0x{}", hex::encode(checkpoint.into_bytes()))
                && state["checkpointSlot"] == slot
                && state["checkpointHash"] == format!("0x{}", hex::encode(&expected_hash)),
            "token manifest conflicts with source or checkpoint"
        );
    } else {
        let state = json!({"lane": "beefy-token-hoodi", "sourceGenesis": genesis,
            "checkpoint": format!("0x{}", hex::encode(checkpoint.into_bytes())), "checkpointSlot": slot,
            "checkpointHash": format!("0x{}", hex::encode(&expected_hash)), "programs": {}});
        save(output, &state)?;
    }
    reconcile_saved_setup(&api, &path).await?;
    let gas = api.block_gas_limit()?;

    let proxy_code = code(&api, historical_proxy::WASM_BINARY).await?;
    let (proxy, new, salt) = intent(output, "historicalProxy", historical_proxy::WASM_BINARY)?;
    if new {
        let result = historical_proxy_client::HistoricalProxyFactory::new(JournaledRemoting::new(
            api.clone(),
            &path,
            "constructor.historicalProxy",
        ))
        .new()
        .with_gas_limit(gas)
        .send_recv(proxy_code, salt)
        .await
        .map_err(|e| anyhow!("historical proxy constructor: {e:?}"))?;
        ensure!(
            result == proxy,
            "historical proxy constructor returned wrong program ID"
        );
    }
    finalized =
        verify_prepared_constructor(&api, &path, "historicalProxy", proxy, proxy_code).await?;
    let proxy_service = historical_proxy_client::HistoricalProxy::new(remoting.clone());
    ensure!(
        proxy_service
            .admin()
            .at_block(sails_block_hash(finalized))
            .recv(proxy)
            .await
            .map_err(|e| anyhow!("historical proxy not readable: {e:?}"))?
            == ActorId::from(<[u8; 32]>::from(api.account_id().clone())),
        "historical proxy constructor admin mismatch; HOLD"
    );
    set_program_status(output, "historicalProxy", "active")?;

    let events_code = code(&api, eth_events_electra::WASM_BINARY).await?;
    let (events, new, salt) = intent(output, "ethEventsElectra", eth_events_electra::WASM_BINARY)?;
    if new {
        let result = eth_events_electra_client::EthEventsElectraFactory::new(
            JournaledRemoting::new(api.clone(), &path, "constructor.ethEventsElectra"),
        )
        .new(checkpoint)
        .with_gas_limit(gas)
        .send_recv(events_code, salt)
        .await
        .map_err(|e| anyhow!("Electra event verifier constructor: {e:?}"))?;
        ensure!(
            result == events,
            "event verifier constructor returned wrong program ID"
        );
    }
    finalized =
        verify_prepared_constructor(&api, &path, "ethEventsElectra", events, events_code).await?;
    let linked = eth_events_electra_client::EthereumEventClient::new(remoting.clone())
        .checkpoint_light_client_address()
        .at_block(sails_block_hash(finalized))
        .recv(events)
        .await
        .map_err(|e| anyhow!("Electra event verifier not readable: {e:?}"))?;
    ensure!(
        linked == checkpoint,
        "event verifier targets wrong checkpoint light client"
    );
    set_program_status(output, "ethEventsElectra", "active")?;

    let endpoints = proxy_service
        .endpoints()
        .at_block(sails_block_hash(finalized))
        .recv(proxy)
        .await
        .map_err(|e| anyhow!("proxy endpoints unreadable: {e:?}"))?;
    if endpoints.is_empty() {
        ensure_configuration_action(
            &path,
            "prepare.endpoint",
            json!({"proxy":format!("{proxy:?}"),"slot":slot,"events":format!("{events:?}")}),
        )?;
        historical_proxy_client::HistoricalProxy::new(JournaledRemoting::new(
            api.clone(),
            &path,
            "prepare.endpoint",
        ))
        .add_endpoint(slot, events)
        .with_gas_limit(gas)
        .send_recv(proxy)
        .await
        .map_err(|e| anyhow!("add event verifier endpoint: {e:?}"))?;
    }
    finalized = api.last_block_hash().await?;
    ensure!(
        proxy_service
            .endpoints()
            .at_block(sails_block_hash(finalized))
            .recv(proxy)
            .await
            .map_err(|e| anyhow!("proxy endpoints unreadable: {e:?}"))?
            == vec![(slot, events)],
        "historical proxy endpoint differs from the intended verifier"
    );

    let config = expected_config();
    let manager_code = code(&api, vft_manager::WASM_BINARY).await?;
    let (manager, new, salt) = intent(output, "vftManager", vft_manager::WASM_BINARY)?;
    if new {
        let result = vft_manager_client::VftManagerFactory::new(JournaledRemoting::new(
            api.clone(),
            &path,
            "constructor.vftManager",
        ))
        .new(InitConfig {
            gear_bridge_builtin: actor(BUILTIN)?,
            historical_proxy_address: proxy,
            config: config.clone(),
        })
        .with_gas_limit(gas)
        .send_recv(manager_code, salt)
        .await
        .map_err(|e| anyhow!("VFT manager constructor: {e:?}"))?;
        ensure!(
            result == manager,
            "VFT manager constructor returned wrong program ID"
        );
    }
    finalized =
        verify_prepared_constructor(&api, &path, "vftManager", manager, manager_code).await?;
    let service = vft_manager_client::VftManager::new(remoting.clone());
    ensure!(
        service
            .gear_bridge_builtin()
            .at_block(sails_block_hash(finalized))
            .recv(manager)
            .await
            .map_err(|e| anyhow!("manager not readable: {e:?}"))?
            == actor(BUILTIN)?,
        "manager builtin mismatch"
    );
    ensure!(
        service
            .historical_proxy_address()
            .at_block(sails_block_hash(finalized))
            .recv(manager)
            .await
            .map_err(|e| anyhow!("manager historical proxy not readable: {e:?}"))?
            == proxy,
        "manager proxy mismatch"
    );
    ensure!(
        service
            .is_paused()
            .at_block(sails_block_hash(finalized))
            .recv(manager)
            .await
            .map_err(|e| anyhow!("manager paused state not readable: {e:?}"))?,
        "new token manager must remain paused"
    );
    let actual_config = service
        .get_config()
        .at_block(sails_block_hash(finalized))
        .recv(manager)
        .await
        .map_err(|e| anyhow!("manager config unreadable: {e:?}"))?;
    ensure!(
        configuration_matches(&actual_config, &config),
        "manager configuration mismatch"
    );
    set_program_status(output, "vftManager", "active")?;

    let payment_code = code(&api, bridging_payment::WASM_BINARY).await?;
    let (payment, new, salt) = intent(output, "bridgingPayment", bridging_payment::WASM_BINARY)?;
    let initial_state = bridging_payment_client::State {
        admin_address: ActorId::from(<[u8; 32]>::from(api.account_id().clone())),
        fee: 1_000_000_000_000,
        priority_fee: 0,
    };
    if new {
        let deployed = bridging_payment_client::BridgingPaymentFactory::new(
            JournaledRemoting::new(api.clone(), &path, "constructor.bridgingPayment"),
        )
        .new(initial_state.clone())
        .with_gas_limit(gas)
        .send_recv(payment_code, salt)
        .await
        .map_err(|e| anyhow!("Gear payment constructor: {e:?}"))?;
        ensure!(
            deployed == payment,
            "Gear payment constructor returned wrong program ID"
        );
    }
    finalized =
        verify_prepared_constructor(&api, &path, "bridgingPayment", payment, payment_code).await?;
    let actual = bridging_payment_client::BridgingPayment::new(remoting.clone())
        .get_state()
        .at_block(sails_block_hash(finalized))
        .recv(payment)
        .await
        .map_err(|e| anyhow!("Gear payment state unreadable: {e:?}"))?;
    ensure!(
        actual.admin_address == initial_state.admin_address
            && actual.fee == initial_state.fee
            && actual.priority_fee == initial_state.priority_fee,
        "Gear payment fees or administrator mismatch"
    );
    set_program_status(output, "bridgingPayment", "active")?;
    let vft_code = code(&api, vft::WASM_BINARY).await?;
    for spec in GEAR_TOKEN_SPECS {
        let component = spec.component;
        let native = component == "nativeVft";
        let label = if native { "native VFT" } else { component };
        let (binary, token_code) = if native {
            (vft_vara::WASM_BINARY, code(&api, vft_vara::WASM_BINARY).await?)
        } else {
            (vft::WASM_BINARY, vft_code)
        };
        let (token, new, salt) = intent(output, component, binary)?;
        if new {
            let remoting = JournaledRemoting::new(
                api.clone(),
                &path,
                &format!("constructor.{component}"),
            );
            let deployed = if native {
                vft_vara_client::VftVaraFactory::new(remoting)
                    .new(Mainnet::No)
                    .with_gas_limit(gas)
                    .send_recv(token_code, salt)
                    .await
            } else {
                vft_client::VftFactory::new(remoting)
                    .new(spec.name.into(), spec.symbol.into(), spec.decimals)
                    .with_gas_limit(gas)
                    .send_recv(token_code, salt)
                    .await
            }
            .map_err(|e| anyhow!("{label} constructor: {e:?}"))?;
            ensure!(
                deployed == token,
                "{label} constructor returned wrong program ID"
            );
        }
        finalized = verify_prepared_constructor(&api, &path, component, token, token_code).await?;
        set_program_status(output, component, "initializing")?;
        // Both token programs expose the same VftMetadata and VftAdmin routes.
        let metadata = vft_client::VftMetadata::new(remoting.clone());
        ensure!(
            metadata
                .name()
                .at_block(sails_block_hash(finalized))
                .recv(token)
                .await
                .map_err(|e| anyhow!("{label} name: {e:?}"))?
                == spec.name
                && metadata
                    .symbol()
                    .at_block(sails_block_hash(finalized))
                    .recv(token)
                    .await
                    .map_err(|e| anyhow!("{label} symbol: {e:?}"))?
                    == spec.symbol
                && metadata
                    .decimals()
                    .at_block(sails_block_hash(finalized))
                    .recv(token)
                    .await
                    .map_err(|e| anyhow!("{label} decimals: {e:?}"))?
                    == spec.decimals,
            "{label} metadata mismatch"
        );
        if native {
            ensure_native_minimum_balance(
                vft_vara_client::VftExtension::new(remoting.clone())
                    .minimum_balance()
                    .at_block(sails_block_hash(finalized))
                    .recv(token)
                    .await?,
            )?;
        }
        let admin = vft_client::VftAdmin::new(remoting.clone());
        ensure!(
            token_authority(&admin, finalized, token, "minter")
                .await
                .map_err(|e| anyhow!("{label} minter: {e:?}"))?
                == initial_state.admin_address
                && token_authority(&admin, finalized, token, "burner")
                    .await
                    .map_err(|e| anyhow!("{label} burner: {e:?}"))?
                    == initial_state.admin_address,
            "{label} initial authority mismatch"
        );
        ensure_configuration_action(
            &path,
            &format!("prepare.shards.{component}"),
            json!({"token":format!("{token:?}")}),
        )?;
        vft_client::allocate_shards(
            JournaledRemoting::new(api.clone(), &path, &format!("prepare.shards.{component}")),
            token,
            gas,
        )
        .await
        .map_err(|e| anyhow!("{label} storage initialization: {e:?}"))?;
        set_program_status(output, component, "active")?;
    }
    println!("Isolated VFT constructors verified; roles and mappings remain unset, manager paused");
    println!(
        "Isolated Gear token stack prepared and paused: proxy={} events={} manager={}",
        hex::encode(proxy.into_bytes()),
        hex::encode(events.into_bytes()),
        hex::encode(manager.into_bytes())
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct EvmTokenSpec {
    name: &'static str,
    symbol: &'static str,
    decimals: u8,
    kind: u8,
}

const EVM_TOKEN_SPECS: [EvmTokenSpec; 6] = [
    EvmTokenSpec {
        name: "USD Coin",
        symbol: "USDC",
        decimals: 6,
        kind: 1,
    },
    EvmTokenSpec {
        name: "Tether USD",
        symbol: "USDT",
        decimals: 6,
        kind: 1,
    },
    EvmTokenSpec {
        name: "Wrapped Ether",
        symbol: "WETH",
        decimals: 18,
        kind: 1,
    },
    EvmTokenSpec {
        name: "Wrapped BTC",
        symbol: "WBTC",
        decimals: 8,
        kind: 1,
    },
    EvmTokenSpec {
        name: "Bridged Wrapped Testnet Vara",
        symbol: "WTVARA",
        decimals: 12,
        kind: 2,
    },
    EvmTokenSpec {
        name: "Bridged Gear Origin Test",
        symbol: "GOT",
        decimals: 12,
        kind: 2,
    },
];

#[derive(Clone)]
struct GearTokenSpec {
    component: &'static str,
    name: &'static str,
    symbol: &'static str,
    decimals: u8,
    evm_symbol: &'static str,
    supply: TokenSupply,
}

const GEAR_TOKEN_SPECS: [GearTokenSpec; 6] = [
    GearTokenSpec {
        component: "circleVft",
        name: "Hoodi Circle",
        symbol: "hUSDC",
        decimals: 6,
        evm_symbol: "USDC",
        supply: TokenSupply::Ethereum,
    },
    GearTokenSpec {
        component: "tetherVft",
        name: "Hoodi Tether",
        symbol: "hUSDT",
        decimals: 6,
        evm_symbol: "USDT",
        supply: TokenSupply::Ethereum,
    },
    GearTokenSpec {
        component: "bitcoinVft",
        name: "Hoodi Bitcoin",
        symbol: "hWBTC",
        decimals: 8,
        evm_symbol: "WBTC",
        supply: TokenSupply::Ethereum,
    },
    GearTokenSpec {
        component: "etherVft",
        name: "Hoodi Ether",
        symbol: "hWETH",
        decimals: 18,
        evm_symbol: "WETH",
        supply: TokenSupply::Ethereum,
    },
    GearTokenSpec {
        component: "gearOriginVft",
        name: "Gear Origin Test",
        symbol: "GOT",
        decimals: 12,
        evm_symbol: "GOT",
        supply: TokenSupply::Gear,
    },
    GearTokenSpec {
        component: "nativeVft",
        name: "Wrapped Testnet Vara",
        symbol: "WTVARA",
        decimals: 12,
        evm_symbol: "WTVARA",
        supply: TokenSupply::Gear,
    },
];

fn ensure_native_minimum_balance(minimum: sails_rs::prelude::U256) -> Result<()> {
    // A nonzero minimum can burn escrow dust or prevent an exact original-recipient refund.
    ensure!(
        minimum.is_zero(),
        "incompatible native wrapper: minimum VFT balance must be zero; refusing policy changes or dust repair"
    );
    Ok(())
}

fn authority_handoff(current: ActorId, setup: ActorId, manager: ActorId) -> Result<bool> {
    ensure!(
        setup != manager,
        "setup signer must be distinct from the VFT manager"
    );
    if current == manager {
        Ok(false)
    } else if current == setup {
        Ok(true)
    } else {
        Err(anyhow!(
            "VFT authority is neither the setup signer nor the intended manager"
        ))
    }
}

async fn token_authority(
    admin: &vft_client::VftAdmin<GClientRemoting>,
    at: GearHash,
    peer: ActorId,
    role: &str,
) -> SailsResult<ActorId> {
    if role == "minter" {
        admin
            .minter()
            .at_block(sails_block_hash(at))
            .recv(peer)
            .await
    } else {
        admin
            .burner()
            .at_block(sails_block_hash(at))
            .recv(peer)
            .await
    }
}

fn missing_token_mappings(
    current: &[(ActorId, H160, TokenSupply)],
    desired: &[(ActorId, H160, TokenSupply)],
) -> Result<Vec<(ActorId, H160, TokenSupply)>> {
    ensure!(
        desired.len() == GEAR_TOKEN_SPECS.len(),
        "four Ethereum-origin and two Gear-origin mappings are required"
    );
    let mut peers = BTreeSet::new();
    let mut evm_tokens = BTreeSet::new();
    for (peer, token, supply) in current {
        ensure!(
            desired.iter().any(
                |(wanted_peer, wanted_token, wanted_supply)| wanted_peer == peer
                    && wanted_token == token
                    && wanted_supply == supply
            ),
            "existing token mapping conflicts with the approved peer or supply direction"
        );
        ensure!(
            peers.insert(*peer) && evm_tokens.insert(*token),
            "duplicate Gear/Ethereum token mapping"
        );
    }
    Ok(desired
        .iter()
        .filter(|(peer, _, _)| !peers.contains(peer))
        .cloned()
        .collect())
}

fn config_error(error: impl std::fmt::Display) -> sails_rs::errors::Error {
    RtlError::ReplyHasErrorString(error.to_string()).into()
}

fn save_config_stack(path: &Path, state: &Value) -> Result<()> {
    save(state_directory(path), state)
}

fn read_config_stack(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn pin_configuration_identity(path: &Path, identity: Value) -> Result<bool> {
    let mut state = read_config_stack(path)?;
    if state["configuration"].is_null() {
        state["configuration"] = json!({"version": 1, "status": "configuring", "actions": {}});
    }
    ensure!(
        state["configuration"]["version"] == 1,
        "unsupported token configuration journal version"
    );
    let was_ready = state["configuration"]["status"] == "ready";
    if state["configuration"]["identity"].is_null() {
        state["configuration"]["identity"] = identity;
    } else {
        ensure!(
            state["configuration"]["identity"] == identity,
            "source, deployment, program, or token identity changed during configuration"
        );
    }
    if !was_ready {
        state["configuration"]["status"] = json!("configuring");
    }
    if state["configuration"]["actions"].is_null() {
        state["configuration"]["actions"] = json!({});
    }
    save_config_stack(path, &state)?;
    Ok(was_ready)
}

fn ensure_configuration_action(path: &Path, key: &str, intent: Value) -> Result<()> {
    let mut state = read_config_stack(path)?;
    let action = &mut state["configuration"]["actions"][key];
    if action.is_null() {
        *action = json!({"status": "pending", "intent": intent, "messages": []});
    } else {
        ensure!(
            action["intent"] == intent,
            "configuration action {key} intent changed"
        );
        if action["messages"].is_null() {
            action["messages"] = json!([]);
        }
    }
    save_config_stack(path, &state)
}

fn begin_configuration_action(path: &Path, key: &str, intent: Value) -> Result<()> {
    ensure_configuration_action(path, key, intent)?;
    let mut state = read_config_stack(path)?;
    let action = &mut state["configuration"]["actions"][key];
    ensure!(
        action["status"] != "verified",
        "verified configuration action {key} no longer matches source state"
    );
    action["status"] = json!("pending");
    save_config_stack(path, &state)
}

fn prepare_configuration_action(
    path: &Path,
    key: &str,
    intent: Value,
    already_applied: bool,
) -> Result<bool> {
    if already_applied {
        ensure_configuration_action(path, key, intent)?;
        Ok(false)
    } else {
        begin_configuration_action(path, key, intent)?;
        Ok(true)
    }
}

fn mark_configuration_action_verified(path: &Path, key: &str, evidence: Value) -> Result<()> {
    let mut state = read_config_stack(path)?;
    let action = &mut state["configuration"]["actions"][key];
    ensure!(
        !action.is_null(),
        "configuration action {key} was not journaled"
    );
    ensure!(
        action["messages"]
            .as_array()
            .is_some_and(|messages| messages
                .iter()
                .all(|m| m["status"] == "reply_received" && !m["replyEvidence"].is_null())),
        "original setup messages not reconciled; HOLD"
    );
    action["status"] = json!("verified");
    action["readback"] = evidence;
    save_config_stack(path, &state)
}

fn setup_bytes(value: &Value, name: &str) -> Result<Vec<u8>> {
    let text = value
        .as_str()
        .ok_or_else(|| anyhow!("{name} missing; HOLD"))?;
    Ok(hex::decode(text.strip_prefix("0x").unwrap_or(text))?)
}

fn setup_message_intent(
    target: ActorId,
    payload: &[u8],
    gas: GasUnit,
    value: ValueUnit,
    constructor: Option<(CodeId, &[u8])>,
) -> Value {
    let mut intent = json!({"target":format!("0x{}",hex::encode(target.into_bytes())),"payload":format!("0x{}",hex::encode(payload)),"gasLimit":gas,"value":value,"entry":"Handle"});
    if let Some((code, salt)) = constructor {
        intent["entry"] = json!("Init");
        intent["codeId"] = json!(code_hex(code));
        intent["salt"] = json!(format!("0x{}", hex::encode(salt)));
    }
    intent
}

fn match_setup_message(action: &Value, index: usize, intent: &Value) -> Result<bool> {
    let messages = action["messages"]
        .as_array()
        .ok_or_else(|| anyhow!("message list invalid; HOLD"))?;
    ensure!(
        index <= messages.len(),
        "setup message ordinal skipped; HOLD"
    );
    ensure!(
        messages[..index]
            .iter()
            .all(|m| m["status"] == "reply_received" && !m["replyEvidence"].is_null()),
        "preceding original reply unresolved; HOLD"
    );
    if let Some(message) = messages.get(index) {
        // Old Handle records stored these fields directly.
        for (key, expected) in intent
            .as_object()
            .ok_or_else(|| anyhow!("invalid setup intent"))?
        {
            let saved = if key == "entry" && message[key].is_null() {
                &json!("Handle")
            } else {
                &message[key]
            };
            ensure!(
                *saved == *expected,
                "original setup message {index} {key} changed; HOLD"
            );
        }
        ensure!(
            !message["submission"]["rawExtrinsic"].is_null(),
            "legacy setup message has no original signed extrinsic; HOLD"
        );
        return Ok(true);
    }
    Ok(false)
}

fn update_configuration_message(path: &Path, key: &str, index: usize, fields: Value) -> Result<()> {
    let mut state = read_config_stack(path)?;
    let message = state["configuration"]["actions"][key]["messages"]
        .as_array_mut()
        .and_then(|messages| messages.get_mut(index))
        .ok_or_else(|| anyhow!("original setup message missing"))?;
    let Value::Object(fields) = fields else {
        anyhow::bail!("invalid message update");
    };
    if fields.iter().all(|(name, value)| message[name] == *value) {
        return Ok(());
    }
    for (name, value) in fields {
        message[name] = value;
    }
    save_config_stack(path, &state)
}

async fn setup_evidence_range(
    api: &GearApi,
    evidence: &Value,
    floor: u32,
    finalized: u32,
) -> Result<std::ops::RangeInclusive<u32>> {
    if evidence.is_null() {
        return Ok(floor..=finalized);
    }
    let number = u32::try_from(
        evidence["blockNumber"]
            .as_u64()
            .ok_or_else(|| anyhow!("saved evidence height missing; HOLD"))?,
    )?;
    let hash = GearHash(decode_hash(&evidence["blockHash"], "saved evidence hash")?);
    ensure!(
        number >= floor && number <= finalized && api.get_block_hash(number).await? == hash,
        "saved setup evidence not canonical finalized history; HOLD"
    );
    Ok(number..=number)
}

async fn reconcile_setup_message(
    api: &GearApi,
    stack_path: &Path,
    action_key: &str,
    message_index: usize,
) -> Result<Option<Vec<u8>>> {
    let state = read_config_stack(stack_path)?;
    let message = state["configuration"]["actions"][action_key]["messages"]
        .as_array()
        .and_then(|messages| messages.get(message_index))
        .ok_or_else(|| anyhow!("original setup message missing; HOLD"))?;
    let submission = &message["submission"];
    let signer: gsdk::signer::Signer = api.clone().into();
    let genesis = decode_hash(&state["sourceGenesis"], "source genesis")?;
    ensure!(
        signer.api().genesis_hash().0 == genesis
            && decode_hash(&submission["sourceGenesis"], "original genesis")? == genesis,
        "original setup chain changed; HOLD"
    );
    let raw = setup_bytes(&submission["rawExtrinsic"], "original raw bytes")?;
    let hash = decode_hash(&submission["extrinsicHash"], "original extrinsic hash")?;
    ensure!(
        sp_core::blake2_256(&raw) == hash,
        "original setup raw hash changed; HOLD"
    );
    let nonce = submission["nonce"]
        .as_u64()
        .ok_or_else(|| anyhow!("original nonce missing; HOLD"))?;
    let target = actor(
        message["target"]
            .as_str()
            .ok_or_else(|| anyhow!("original target missing"))?,
    )?;
    let payload = setup_bytes(&message["payload"], "original payload")?;
    let gas = message["gasLimit"]
        .as_u64()
        .ok_or_else(|| anyhow!("original gas missing"))?;
    let value: u128 = serde_json::from_value(message["value"].clone())?;
    let init = message["entry"] == "Init";
    let call_data = if init {
        let code = CodeId::from(decode_hash(&message["codeId"], "constructor code")?);
        let salt = setup_bytes(&message["salt"], "constructor salt")?;
        ensure!(
            ActorId::generate_from_user(code, &salt) == target,
            "constructor target changed; HOLD"
        );
        signer
            .api()
            .tx()
            .call_data(&gear::tx().gear().create_program(
                code,
                salt,
                payload.clone(),
                gas,
                value,
                false,
            ))?
    } else {
        ensure!(
            message["entry"].is_null() || message["entry"] == "Handle",
            "original entry changed; HOLD"
        );
        signer
            .api()
            .tx()
            .call_data(&gear::tx().gear().send_message(
                target,
                payload.clone(),
                gas,
                value,
                false,
            ))?
    };
    let decoded =
        gsdk::ext::subxt::ext::subxt_core::blocks::Extrinsics::<gsdk::GearConfig>::decode_from(
            vec![raw.clone()],
            signer.api().metadata(),
        )?;
    let original = decoded
        .iter()
        .next()
        .ok_or_else(|| anyhow!("signed envelope missing; HOLD"))?;
    let sender = <[u8; 32]>::from(api.account_id().clone());
    let mut address = [0; 33];
    address[1..].copy_from_slice(&sender);
    ensure!(
        original.address_bytes() == Some(address.as_slice())
            && original
                .transaction_extensions()
                .and_then(|extensions| extensions.nonce())
                == Some(nonce)
            && original.call_bytes() == call_data
            && original.pallet_name()? == "Gear"
            && original.variant_name()?
                == if init {
                    "create_program"
                } else {
                    "send_message"
                },
        "original setup signer, nonce or call changed; HOLD"
    );
    let signature_bytes = original
        .signature_bytes()
        .ok_or_else(|| anyhow!("original signature missing; HOLD"))?;
    ensure!(
        signature_bytes.len() == 65 && signature_bytes[0] == 1,
        "original setup signature is not sr25519; HOLD"
    );

    if !submission["sender"].is_null() {
        ensure!(
            submission["sender"] == format!("0x{}", hex::encode(sender)),
            "original setup sender changed; HOLD"
        );
    }
    if !submission["callData"].is_null() {
        ensure!(
            setup_bytes(&submission["callData"], "original call data")? == call_data,
            "original call bytes changed; HOLD"
        );
    }
    let scan_from = if submission["scanFromNumber"].is_null() {
        0
    } else {
        u32::try_from(
            submission["scanFromNumber"]
                .as_u64()
                .ok_or_else(|| anyhow!("original scan height invalid"))?,
        )?
    };
    let scan_hash = if submission["scanFromHash"].is_null() {
        GearHash(genesis)
    } else {
        GearHash(decode_hash(
            &submission["scanFromHash"],
            "original scan hash",
        )?)
    };
    ensure!(
        api.get_block_hash(scan_from).await? == scan_hash,
        "original setup scan anchor changed; HOLD"
    );
    if submission["scanFromNumber"].is_null() {
        let mut migrated = submission.clone();
        migrated["scanFromNumber"] = json!(scan_from);
        migrated["scanFromHash"] = json!(format!("{scan_hash:#x}"));
        migrated["sender"] = json!(format!("0x{}", hex::encode(sender)));
        migrated["callData"] = json!(format!("0x{}", hex::encode(&call_data)));
        update_configuration_message(
            stack_path,
            action_key,
            message_index,
            json!({"submission":migrated,"entry":if init {"Init"} else {"Handle"}}),
        )?;
    }
    let finalized_hash = api.last_block_hash().await?;
    let finalized_number = api.block_number_at(finalized_hash).await?;
    ensure!(
        scan_from <= finalized_number,
        "original scan anchor not finalized; HOLD"
    );
    if !message["observedFinality"].is_null() {
        let previous = u32::try_from(
            message["observedFinality"]["blockNumber"]
                .as_u64()
                .ok_or_else(|| anyhow!("previous finality malformed; HOLD"))?,
        )?;
        ensure!(
            previous <= finalized_number,
            "setup finalized height regressed; HOLD"
        );
        setup_evidence_range(
            api,
            &message["observedFinality"],
            scan_from,
            finalized_number,
        )
        .await?;
    }
    let mut inclusion: Option<(u32, GearHash, [u8; 32])> = None;
    let chain = gear_rpc_client::GearApi::from(signer.api().clone());
    for number in setup_evidence_range(
        api,
        &message["inclusionEvidence"],
        scan_from,
        finalized_number,
    )
    .await?
    {
        let block_hash = api.get_block_hash(number).await?;
        let block = signer.api().blocks().at(block_hash).await?;
        let extrinsics = block.extrinsics().await?;
        for extrinsic in extrinsics.iter().filter(|extrinsic| extrinsic.hash() == GearHash(hash))
        {
            ensure!(
                inclusion.is_none() && extrinsic.bytes() == raw,
                "original setup inclusion changed; HOLD"
            );
            let mut success = false;
            let mut queued = None;
            for event in extrinsic.events().await?.iter() {
                let event = event?;
                if event.pallet_name() == "System" && event.variant_name() == "ExtrinsicFailed" {
                    update_configuration_message(
                        stack_path,
                        action_key,
                        message_index,
                        json!({"status":"dispatch_failed","failureEvidence":{"blockNumber":number,"blockHash":format!("{block_hash:#x}")}}),
                    )?;
                    anyhow::bail!("original setup failed dispatch; HOLD");
                }
                success |=
                    event.pallet_name() == "System" && event.variant_name() == "ExtrinsicSuccess";
                if let Some(gear::gear::events::MessageQueued {
                    id,
                    source,
                    destination,
                    entry,
                }) = event.as_event::<gear::gear::events::MessageQueued>()?
                {
                    ensure!(
                        source.0 == sender
                            && destination.as_ref() == target.into_bytes().as_slice()
                            && matches!(
                                (init, entry),
                                (true, MessageEntry::Init) | (false, MessageEntry::Handle)
                            ),
                        "original setup queued identity changed; HOLD"
                    );
                    ensure!(
                        queued.is_none(),
                        "original setup queued multiple messages; HOLD"
                    );
                    queued = Some(
                        <[u8; 32]>::try_from(id.as_ref())
                            .map_err(|_| anyhow!("invalid message ID"))?,
                    );
                }
            }
            ensure!(success, "original setup successful dispatch absent; HOLD");
            let id = queued.ok_or_else(|| anyhow!("original setup queued message absent; HOLD"))?;
            let evidence = json!({"blockNumber":number,"blockHash":format!("{block_hash:#x}"),"messageId":format!("0x{}",hex::encode(id))});
            ensure!(
                message["inclusionEvidence"].is_null() || message["inclusionEvidence"] == evidence,
                "original setup inclusion evidence changed; HOLD"
            );
            ensure!(
                message["messageId"].is_null() || message["messageId"] == evidence["messageId"],
                "original message ID changed; HOLD"
            );
            update_configuration_message(
                stack_path,
                action_key,
                message_index,
                json!({"status":if message["replyEvidence"].is_null() {"submitted"} else {message["status"].as_str().ok_or_else(|| anyhow!("original reply status missing; HOLD"))?},"messageId":evidence["messageId"],"enqueueBlockHash":format!("{block_hash:#x}"),"inclusionEvidence":evidence}),
            )?;
            inclusion = Some((number, block_hash, id));
        }
    }
    ensure!(
        api.get_block_hash(finalized_number).await? == finalized_hash
            && api.get_block_hash(scan_from).await? == scan_hash,
        "finalized setup history changed; HOLD"
    );
    update_configuration_message(
        stack_path,
        action_key,
        message_index,
        json!({"observedFinality":{"blockNumber":finalized_number,"blockHash":format!("{finalized_hash:#x}")}}),
    )?;
    if let Some((included, _, id)) = inclusion {
        let range =
            setup_evidence_range(api, &message["replyEvidence"], included, finalized_number).await?;
        if let Some(found) =
            gear_rpc_client::observation::find_reply(&chain, range, GearHash(id), |_, _| {}).await?
        {
            let number = found.number;
            let block_hash = found.block_hash;
            let reply = found.event.message;
            let details = reply.details().expect("discovered reply has details");
            ensure!(
                reply.source().as_ref() == target.into_bytes().as_slice()
                    && reply.destination().as_ref() == sender,
                "original reply actor/sender changed; HOLD"
            );
            let bytes = reply.payload_bytes().to_vec();
            let evidence = json!({"blockNumber":number,"blockHash":format!("{block_hash:#x}"),"messageId":format!("0x{}",hex::encode(reply.id().as_ref())),"payload":format!("0x{}",hex::encode(&bytes)),"code":format!("{:?}",details.to_reply_code())});
            ensure!(
                message["replyEvidence"].is_null() || message["replyEvidence"] == evidence,
                "original finalized reply changed; HOLD"
            );
            ensure!(
                message["replyPayload"].is_null()
                    || message["replyPayload"] == evidence["payload"],
                "original reply bytes changed; HOLD"
            );
            ensure!(
                api.get_block_hash(number).await? == block_hash
                    && api.get_block_hash(finalized_number).await? == finalized_hash,
                "original reply canonical pin changed; HOLD"
            );
            let success = details.to_reply_code().is_success();
            update_configuration_message(
                stack_path,
                action_key,
                message_index,
                json!({"status":if success {"reply_received"} else {"reply_error"},"replyEvidence":evidence,"replyPayload":format!("0x{}",hex::encode(&bytes))}),
            )?;
            ensure!(success, "original setup terminal reply failed; HOLD");
            return Ok(Some(bytes));
        }
        ensure!(
            message["replyEvidence"].is_null(),
            "saved original reply missing from archive; HOLD"
        );
    } else {
        ensure!(
            message["inclusionEvidence"].is_null() && message["messageId"].is_null(),
            "saved original inclusion missing from archive; HOLD"
        );
        let account = signer
            .api()
            .info_at(&api.account_id().to_string(), Some(finalized_hash))
            .await?;
        update_configuration_message(
            stack_path,
            action_key,
            message_index,
            json!({"nonceEvidence":{"blockNumber":finalized_number,"blockHash":format!("{finalized_hash:#x}"),"accountNonce":account.nonce}}),
        )?;
        ensure!(
            u64::from(account.nonce) <= nonce,
            "original nonce consumed without original success; HOLD"
        );
        let extensions = original
            .transaction_extensions()
            .ok_or_else(|| anyhow!("original signed extensions absent; HOLD"))?;
        let mortality = extensions
            .iter()
            .find(|extension| extension.name() == "CheckMortality")
            .ok_or_else(|| anyhow!("original mortality absent; HOLD"))?;
        let era = sp_runtime::generic::Era::decode(&mut mortality.bytes())?;
        let params = match era {
            sp_runtime::generic::Era::Immortal => PolkadotExtrinsicParamsBuilder::new()
                .immortal()
                .nonce(nonce)
                .build(),
            sp_runtime::generic::Era::Mortal(period, _) => {
                let birth = era.birth(u64::from(finalized_number));
                let hash = api.get_block_hash(u32::try_from(birth)?).await?;
                PolkadotExtrinsicParamsBuilder::new()
                    .mortal_from_unchecked(period, birth, hash)
                    .nonce(nonce)
                    .build()
            }
        };
        let mut partial = if init {
            let code = CodeId::from(decode_hash(&message["codeId"], "constructor code")?);
            let salt = setup_bytes(&message["salt"], "constructor salt")?;
            signer
                .api()
                .tx()
                .create_partial(
                    &gear::tx().gear().create_program(
                        code,
                        salt,
                        payload,
                        gas,
                        value,
                        false,
                    ),
                    signer.account_id(),
                    params,
                )
                .await?
        } else {
            signer
                .api()
                .tx()
                .create_partial(
                    &gear::tx()
                        .gear()
                        .send_message(target, payload, gas, value, false),
                    signer.account_id(),
                    params,
                )
                .await?
        };
        let signature = sp_core::sr25519::Signature::from_raw(
            signature_bytes[1..]
                .try_into()
                .map_err(|_| anyhow!("invalid signature"))?,
        );
        ensure!(
            <sp_core::sr25519::Pair as sp_core::Pair>::verify(
                &signature,
                partial.signer_payload(),
                &sp_core::sr25519::Public::from_raw(sender)
            ),
            "original setup signature/domain invalid, expired, or runtime identity changed; HOLD"
        );
        let signature = <gsdk::GearConfig as gsdk::ext::subxt::Config>::Signature::decode(
            &mut &signature_bytes[..],
        )?;
        ensure!(
            partial
                .sign_with_account_and_signature(signer.account_id(), &signature)
                .encoded()
                == raw,
            "original signed extension/call identity changed; HOLD"
        );

        let transaction =
            gsdk::ext::subxt::tx::SubmittableTransaction::<gsdk::GearConfig, _>::from_bytes(
                (**signer.api()).clone(),
                raw,
            );
        ensure!(
            transaction.hash().0 == hash,
            "original SDK hash changed; HOLD"
        );
        transaction
            .submit_and_watch()
            .await?
            .wait_for_finalized()
            .await?;
    }
    Ok(None)
}

async fn wait_setup_message(
    api: &GearApi,
    path: &Path,
    key: &str,
    index: usize,
) -> Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Some(reply) = reconcile_setup_message(api, path, key, index).await? {
                return Ok(reply);
            }
            tokio::time::sleep(Duration::from_secs(6)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("original setup reply still pending; HOLD/reconcile before retry"))?
}

async fn reconcile_saved_setup(api: &GearApi, path: &Path) -> Result<()> {
    let state = read_config_stack(path)?;
    if let Some(actions) = state["configuration"]["actions"].as_object() {
        let mut originals = Vec::new();
        for (key, action) in actions {
            for (index, message) in action["messages"]
                .as_array()
                .ok_or_else(|| anyhow!("setup message list malformed; HOLD"))?
                .iter()
                .enumerate()
            {
                let nonce = message["submission"]["nonce"]
                    .as_u64()
                    .ok_or_else(|| anyhow!("legacy setup original nonce missing; HOLD"))?;
                originals.push((nonce, key.as_str(), index));
            }
        }
        originals.sort_by_key(|original| original.0);
        ensure!(
            originals.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "setup originals share a nonce; HOLD"
        );
        for (_, key, index) in originals {
            wait_setup_message(api, path, key, index).await?;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct JournaledRemoting {
    api: GearApi,
    stack_path: PathBuf,
    action_key: String,
    next_message_index: Arc<Mutex<usize>>,
}
impl JournaledRemoting {
    fn new(api: GearApi, path: &Path, key: &str) -> Self {
        Self {
            api,
            stack_path: path.to_path_buf(),
            action_key: key.to_owned(),
            next_message_index: Arc::new(Mutex::new(0)),
        }
    }
    async fn send_setup(
        self,
        target: ActorId,
        payload: Vec<u8>,
        gas: GasUnit,
        value: ValueUnit,
        constructor: Option<(CodeId, Vec<u8>)>,
    ) -> Result<Vec<u8>> {
        let index = *self
            .next_message_index
            .lock()
            .map_err(|_| anyhow!("setup ordinal lock poisoned"))?;
        let intent = setup_message_intent(
            target,
            &payload,
            gas,
            value,
            constructor
                .as_ref()
                .map(|(code, salt)| (*code, salt.as_slice())),
        );
        if constructor.is_some() {
            ensure_configuration_action(&self.stack_path, &self.action_key, intent.clone())?;
        }
        let state = read_config_stack(&self.stack_path)?;
        let action = &state["configuration"]["actions"][&self.action_key];
        if !match_setup_message(action, index, &intent)? {
            // Settle every existing original before consuming another account nonce.
            reconcile_saved_setup(&self.api, &self.stack_path).await?;
            let signer: gsdk::signer::Signer = self.api.clone().into();
            let nonce = signer.api().tx().account_nonce(signer.account_id()).await?;
            let at = self.api.last_block_hash().await?;
            let scan_number = self.api.block_number_at(at).await?;
            let signed = if let Some((code, salt)) = constructor {
                signer
                    .api()
                    .original_code_storage_at(code, Some(at))
                    .await?;
                signer
                    .api()
                    .tx()
                    .create_signed(
                        &gear::tx().gear().create_program(
                            code,
                            salt,
                            payload,
                            gas,
                            value,
                            false,
                        ),
                        signer.signer(),
                        PolkadotExtrinsicParamsBuilder::new()
                            .immortal()
                            .nonce(nonce)
                            .build(),
                    )
                    .await?
            } else {
                signer
                    .api()
                    .tx()
                    .create_signed(
                        &gear::tx()
                            .gear()
                            .send_message(target, payload, gas, value, false),
                        signer.signer(),
                        PolkadotExtrinsicParamsBuilder::new()
                            .immortal()
                            .nonce(nonce)
                            .build(),
                    )
                    .await?
            };
            let call_data = gsdk::ext::subxt::ext::subxt_core::blocks::Extrinsics::<
                gsdk::GearConfig,
            >::decode_from(
                vec![signed.encoded().to_vec()], signer.api().metadata()
            )?;
            let call_data = call_data
                .iter()
                .next()
                .ok_or_else(|| anyhow!("new signed envelope missing"))?
                .call_bytes()
                .to_vec();
            let submission = json!({"nonce":nonce,"extrinsicHash":format!("{:#x}",signed.hash()),"rawExtrinsic":format!("0x{}",hex::encode(signed.encoded())),"callData":format!("0x{}",hex::encode(call_data)),"sourceGenesis":format!("{:#x}",signer.api().genesis_hash()),"sender":format!("0x{}",hex::encode(<[u8;32]>::from(self.api.account_id().clone()))),"scanFromNumber":scan_number,"scanFromHash":format!("{at:#x}")});
            let mut state = read_config_stack(&self.stack_path)?;
            let action = &mut state["configuration"]["actions"][&self.action_key];
            ensure!(
                !match_setup_message(action, index, &intent)?,
                "concurrent original setup writer; HOLD"
            );
            let mut message = intent;
            message["submission"] = submission;
            message["status"] = json!("submitting");
            action["messages"]
                .as_array_mut()
                .ok_or_else(|| anyhow!("setup messages malformed"))?
                .push(message);
            if let Some(component) = self.action_key.strip_prefix("constructor.") {
                state["programs"][component]["status"] = json!("pending");
            }
            save_config_stack(&self.stack_path, &state)?;
        }
        let reply =
            wait_setup_message(&self.api, &self.stack_path, &self.action_key, index).await?;
        let mut next = self
            .next_message_index
            .lock()
            .map_err(|_| anyhow!("setup ordinal lock poisoned"))?;
        ensure!(*next == index, "concurrent setup calls share ordinal; HOLD");
        *next += 1;
        Ok(reply)
    }
}
impl Remoting for JournaledRemoting {
    type Args = GClientArgs;
    async fn activate(
        self,
        code: CodeId,
        salt: impl AsRef<[u8]>,
        payload: impl AsRef<[u8]>,
        gas: Option<GasUnit>,
        value: ValueUnit,
        _args: Self::Args,
    ) -> SailsResult<impl Future<Output = SailsResult<(ActorId, Vec<u8>)>>> {
        let salt = salt.as_ref().to_vec();
        let payload = payload.as_ref().to_vec();
        let target = ActorId::generate_from_user(code, &salt);
        let gas = gas.unwrap_or(self.api.block_gas_limit().map_err(config_error)?);
        Ok(async move {
            let reply = self
                .send_setup(target, payload, gas, value, Some((code, salt)))
                .await
                .map_err(config_error)?;
            Ok((target, reply))
        })
    }
    async fn message(
        self,
        target: ActorId,
        payload: impl AsRef<[u8]>,
        gas: Option<GasUnit>,
        value: ValueUnit,
        _args: Self::Args,
    ) -> SailsResult<impl Future<Output = SailsResult<Vec<u8>>>> {
        let payload = payload.as_ref().to_vec();
        let gas = gas.unwrap_or(self.api.block_gas_limit().map_err(config_error)?);
        Ok(async move {
            self.send_setup(target, payload, gas, value, None)
                .await
                .map_err(config_error)
        })
    }
    async fn query(
        self,
        target: ActorId,
        payload: impl AsRef<[u8]>,
        gas: Option<GasUnit>,
        value: ValueUnit,
        args: Self::Args,
    ) -> SailsResult<Vec<u8>> {
        GClientRemoting::new(self.api)
            .query(target, payload, gas, value, args)
            .await
    }
}

fn sails_block_hash(hash: GearHash) -> H256 {
    H256::from_slice(hash.as_bytes())
}

async fn finalized_evidence(source: &Source) -> Result<(GearHash, u32, Value)> {
    let hash = source.api.latest_finalized_block().await?;
    let height = source.api.block_hash_to_number(hash).await?;
    Ok((
        hash,
        height,
        json!({"height": height, "hash": format!("{hash:#x}")}),
    ))
}

async fn storage_bool(source: &Source, entry: &str, at: GearHash) -> Result<Option<bool>> {
    let address = gsdk::ext::subxt::dynamic::storage("GearEthBridge", entry, vec![]);
    let value = source.api.api.storage().at(at).fetch(&address).await?;
    let Some(value) = value else {
        return Ok(None);
    };
    let mut input = value.encoded();
    let decoded = bool::decode(&mut input)?;
    ensure!(
        input.is_empty(),
        "GearEthBridge.{entry} storage has trailing bytes"
    );
    Ok(Some(decoded))
}

async fn ensure_source_bridge_ready(source: &Source, at: GearHash) -> Result<()> {
    ensure!(
        storage_bool(source, "Initialized", at).await? == Some(true),
        "source GearEthBridge is not initialized"
    );
    ensure!(
        storage_bool(source, "Paused", at).await? == Some(false),
        "source GearEthBridge is paused or has no recorded unpaused state"
    );
    Ok(())
}

fn code_hex(code: CodeId) -> String {
    format!("0x{}", hex::encode(code.into_bytes()))
}

async fn verify_prepared_constructor(
    api: &GearApi,
    path: &Path,
    component: &str,
    id: ActorId,
    code: CodeId,
) -> Result<GearHash> {
    let at = api.last_block_hash().await?;
    let state = read_config_stack(path)?;
    let legacy = state["programs"][component]["status"] == "pending"
        && state["configuration"]["actions"][format!("constructor.{component}")]["messages"]
            .as_array()
            .is_none_or(Vec::is_empty);
    verify_program(api, at, id, Some(code), component).await.map_err(|error| {
        if legacy { anyhow!("legacy constructor {component} has no original signed extrinsic and exact finalized program is unavailable: {error}; HOLD") }
        else { anyhow!("constructor {component} finalized readback failed: {error}; HOLD") }
    })?;
    ensure!(
        api.get_block_hash(api.block_number_at(at).await?).await? == at,
        "constructor finalized pin changed; HOLD"
    );
    Ok(at)
}

async fn verify_program(
    api: &GearApi,
    at: GearHash,
    program_id: ActorId,
    expected_code: Option<CodeId>,
    component: &str,
) -> Result<Value> {
    let program = api.program_at(program_id, Some(at)).await?;
    ensure!(
        program.state == ProgramState::Initialized,
        "source program {component} is not initialized"
    );
    if let Some(expected_code) = expected_code {
        ensure!(
            program.code_id == expected_code,
            "source program {component} has the wrong code identity"
        );
    }
    Ok(json!({
        "id": format!("0x{}", hex::encode(program_id.into_bytes())),
        "codeId": code_hex(program.code_id),
        "state": "initialized",
    }))
}

async fn verify_stack_programs(
    api: &GearApi,
    at: GearHash,
    stack: &Value,
    genesis: &str,
) -> Result<Value> {
    let specs: [(&str, &[u8]); 10] = [
        ("historicalProxy", historical_proxy::WASM_BINARY),
        ("ethEventsElectra", eth_events_electra::WASM_BINARY),
        ("vftManager", vft_manager::WASM_BINARY),
        ("bridgingPayment", bridging_payment::WASM_BINARY),
        ("circleVft", vft::WASM_BINARY),
        ("tetherVft", vft::WASM_BINARY),
        ("bitcoinVft", vft::WASM_BINARY),
        ("etherVft", vft::WASM_BINARY),
        ("gearOriginVft", vft::WASM_BINARY),
        ("nativeVft", vft_vara::WASM_BINARY),
    ];
    let mut programs = serde_json::Map::new();
    for (component, binary) in specs {
        let code_id = CodeId::generate(binary);
        let salt = format!("beefy-token-hoodi-{genesis}-{component}").into_bytes();
        let program_id = ActorId::generate_from_user(code_id, &salt);
        let stored = &stack["programs"][component];
        ensure!(
            stored["id"] == format!("0x{}", hex::encode(program_id.into_bytes()))
                && stored["salt"] == format!("0x{}", hex::encode(&salt))
                && stored["status"] == "active",
            "token stack program {component} differs from its prepared artifact or salt"
        );
        let identity = verify_program(api, at, program_id, Some(code_id), component).await?;
        programs.insert(component.to_owned(), identity);
    }
    let checkpoint = actor(
        stack["checkpoint"]
            .as_str()
            .ok_or_else(|| anyhow!("checkpoint program id is missing"))?,
    )?;
    let checkpoint_identity = verify_program(api, at, checkpoint, None, "checkpoint").await?;
    Ok(json!({"checkpoint": checkpoint_identity, "programs": programs}))
}

fn expected_config() -> Config {
    Config {
        gas_for_token_ops: 10_000_000_000,
        gas_for_reply_deposit: 10_000_000_000,
        gas_to_send_request_to_builtin: 10_000_000_000,
        gas_for_swap_token_maps: 1_500_000_000,
        reply_timeout: 100,
        fee_bridge: 0,
        fee_incoming: 1_000_000_000_000,
    }
}

fn configuration_matches(actual: &Config, expected: &Config) -> bool {
    actual.fee_bridge == expected.fee_bridge
        && actual.fee_incoming == expected.fee_incoming
        && actual.gas_for_token_ops == expected.gas_for_token_ops
        && actual.gas_for_reply_deposit == expected.gas_for_reply_deposit
        && actual.gas_to_send_request_to_builtin == expected.gas_to_send_request_to_builtin
        && actual.gas_for_swap_token_maps == expected.gas_for_swap_token_maps
        && actual.reply_timeout == expected.reply_timeout
}

async fn inspect_ethereum_configuration(
    ethereum: &beefy_relay::ethereum::Ethereum,
    gear_manager: [u8; 32],
) -> Result<Value> {
    let manager = ConfiguredERC20Manager::new(
        ethereum.receiver_address,
        ethereum.api.raw_provider().clone(),
    );
    let governance_admin = manager.governanceAdmin().call().await?;
    let governance_pauser = manager.governancePauser().call().await?;
    let expected_gear_manager = B256::from(gear_manager);
    let vft_managers = manager.vftManagers().call().await?;
    ensure!(
        governance_admin != Address::ZERO
            && governance_pauser != Address::ZERO
            && manager.messageQueue().call().await? == ethereum.queue_address
            && manager.totalVftManagers().call().await? == U256::from(1u8)
            && vft_managers == vec![expected_gear_manager]
            && manager.isVftManager(expected_gear_manager).call().await?
            && manager.totalTokens().call().await? == U256::from(EVM_TOKEN_SPECS.len())
            && !manager.paused().call().await?,
        "fresh EVM token manager roles, queue, token set, or VFT registration differ"
    );
    let mut pauser_role = [0u8; 32];
    pauser_role[31] = 1;
    ensure!(
        manager.hasRole(B256::ZERO, governance_admin).call().await?
            && manager
                .hasRole(B256::from(pauser_role), governance_admin)
                .call()
                .await?
            && manager
                .hasRole(B256::from(pauser_role), governance_pauser)
                .call()
                .await?,
        "fresh EVM token manager governance roles are incomplete"
    );
    let admin_code = ethereum
        .api
        .raw_provider()
        .get_code_at(governance_admin)
        .await?;
    let pauser_code = ethereum
        .api
        .raw_provider()
        .get_code_at(governance_pauser)
        .await?;
    ensure!(
        !admin_code.is_empty() && !pauser_code.is_empty(),
        "EVM governance identities are not deployed contracts"
    );
    let manager_code = ethereum
        .api
        .raw_provider()
        .get_code_at(ethereum.receiver_address)
        .await?;
    ensure!(
        !manager_code.is_empty(),
        "EVM token manager has no runtime code"
    );

    let token_addresses = manager.tokens().call().await?;
    ensure!(
        token_addresses.len() == EVM_TOKEN_SPECS.len(),
        "EVM token manager does not have exactly six fresh test tokens"
    );
    let mut by_symbol = BTreeMap::<String, Address>::new();
    let mut token_records = Vec::with_capacity(token_addresses.len());
    for address in token_addresses {
        let token = ConfiguredERC20::new(address, ethereum.api.raw_provider().clone());
        let name = token.name().call().await?;
        let symbol = token.symbol().call().await?;
        let decimals = token.decimals().call().await?;
        let kind = manager.getTokenType(address).call().await?;
        let spec = EVM_TOKEN_SPECS
            .iter()
            .find(|spec| spec.symbol == symbol)
            .ok_or_else(|| anyhow!("unexpected EVM token symbol {symbol}"))?;
        ensure!(name == spec.name && decimals == spec.decimals && kind == spec.kind, "EVM token {symbol} identity, decimals, or supply type differs from the fresh test deployment");
        ensure!(
            by_symbol.insert(symbol.clone(), address).is_none(),
            "duplicate EVM token symbol {symbol}"
        );
        let code = ethereum.api.raw_provider().get_code_at(address).await?;
        ensure!(!code.is_empty(), "EVM token {symbol} has no runtime code");
        token_records.push(json!({
            "address": format!("{address:#x}"),
            "name": name,
            "symbol": symbol,
            "decimals": decimals,
            "type": kind,
            "codeHash": format!("{:#x}", alloy::primitives::keccak256(code.as_ref())),
        }));
    }
    ensure!(
        by_symbol.len() == EVM_TOKEN_SPECS.len(),
        "EVM token registry is incomplete"
    );
    token_records.sort_by(|left, right| left["symbol"].as_str().cmp(&right["symbol"].as_str()));

    let payment_addresses = manager.bridgingPayments().call().await?;
    ensure!(
        manager.totalBridgingPayments().call().await? == U256::from(1u8)
            && payment_addresses.len() == 1,
        "fresh EVM token manager must have exactly one bridging-payment contract"
    );
    let payment_address = payment_addresses[0];
    let payment =
        ConfiguredBridgingPayment::new(payment_address, ethereum.api.raw_provider().clone());
    let payment_manager = payment.erc20Manager().call().await?;
    let payment_fee = payment.fee().call().await?;
    let payment_owner = payment.owner().call().await?;
    let payment_code = ethereum
        .api
        .raw_provider()
        .get_code_at(payment_address)
        .await?;
    ensure!(
        payment_address != Address::ZERO
            && payment_manager == ethereum.receiver_address
            && payment_fee > U256::ZERO
            && payment_owner != Address::ZERO
            && !payment_code.is_empty(),
        "EVM bridging-payment identity, owner, fee, or manager binding is invalid"
    );
    Ok(json!({
        "manager": format!("{:#x}", ethereum.receiver_address),
        "managerCodeHash": format!("{:#x}", alloy::primitives::keccak256(manager_code.as_ref())),
        "governanceAdmin": format!("{governance_admin:#x}"),
        "governanceAdminCodeHash": format!("{:#x}", alloy::primitives::keccak256(admin_code.as_ref())),
        "governancePauser": format!("{governance_pauser:#x}"),
        "governancePauserCodeHash": format!("{:#x}", alloy::primitives::keccak256(pauser_code.as_ref())),
        "messageQueue": format!("{:#x}", ethereum.queue_address),
        "beefyRecovery": ethereum.recovery_configuration().await?,
        "vftManagers": [format!("{expected_gear_manager:#x}")],
        "tokens": token_records,
        "bridgingPayments": [{
            "address": format!("{payment_address:#x}"),
            "manager": format!("{payment_manager:#x}"),
            "fee": payment_fee.to_string(),
            "owner": format!("{payment_owner:#x}"),
            "codeHash": format!("{:#x}", alloy::primitives::keccak256(payment_code.as_ref())),
        }],
        "paused": false,
    }))
}

/// Configure exactly one fresh local Gear token stack against its real Hoodi deployment.
#[allow(clippy::too_many_arguments)]
pub async fn configure(
    source_rpc: &str,
    witness_rpc: &str,
    expected_genesis: &str,
    suri: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    deployment_manifest: &Path,
    token_stack: &Path,
    owned_fixture_network: Option<&Path>,
) -> Result<()> {
    ensure!(
        beefy_relay::local_source_rpc(source_rpc)
            && beefy_relay::local_source_rpc(witness_rpc)
            && source_rpc != witness_rpc,
        "tokens-configure requires independent loopback source and witness RPCs"
    );
    ensure!(
        token_stack
            .file_name()
            .is_some_and(|name| name == "token-stack.json"),
        "--token-stack must point to token-stack.json"
    );
    let _deployment_lock = lock_token_deployment(deployment_manifest)?;
    let _stack_lock = lock_token_stack(token_stack)?;
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    let fixture = beefy_relay::ethereum::approved_fixture_network(
        owned_fixture_network,
        &deployment,
        source_rpc,
        witness_rpc,
        ethereum_rpc,
        beefy_relay::ethereum::PublicEthereumNetwork::Hoodi,
    )?;
    let chain_id = fixture
        .as_ref()
        .map_or(HOODI_CHAIN_ID, |network| network.ethereum_chain_id);
    ensure!(
        deployment["mode"] == "hoodi-token-stack"
            && deployment["localRehearsal"] == false
            && deployment["ethereum"]["chainId"] == chain_id,
        "tokens-configure requires a real approved network, never a local rehearsal"
    );
    let ethereum = beefy_relay::ethereum::Ethereum::connect(
        ethereum_rpc,
        wallet,
        &deployment["ethereum"],
        fixture.as_ref(),
        beefy_relay::ethereum::PublicEthereumNetwork::Hoodi,
    )
    .await?;
    let stack_path = fs::canonicalize(token_stack)?;
    let mut stack = read_config_stack(&stack_path)?;
    if !stack["configuration"]["identity"].is_null() {
        ensure!(
            stack["configuration"]["identity"]["ownedFixtureNetwork"]
                == fixture
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()?
                    .unwrap_or(Value::Null),
            "owned fixture approval changed across configuration resume; HOLD"
        );
    }
    ensure!(
        stack["lane"] == "beefy-token-hoodi",
        "not the isolated Hoodi token stack"
    );
    let expected_genesis = decode_hash(&json!(expected_genesis), "expected genesis")?;
    let (source, _witness) = Source::connect_pair(
        gear_rpc_client::GearApi::new(source_rpc, 3).await?,
        gear_rpc_client::GearApi::new(witness_rpc, 3).await?,
    )
    .await?;
    source.validate_attachment(&deployment["anchor"]).await?;
    ensure!(
        source.source_genesis == expected_genesis,
        "wrong source genesis"
    );
    ensure_source_identity(
        &deployment["anchor"],
        source.source_genesis,
        source.bridge_domain,
        source.source_genesis,
        source.bridge_domain,
    )?;
    ensure!(
        stack["sourceGenesis"] == format!("0x{}", hex::encode(source.source_genesis))
            && deployment["ethereum"]["bridgeDomain"]
                == format!("0x{}", hex::encode(source.bridge_domain)),
        "token stack or Hoodi deployment source identity differs from the local Gear chain"
    );
    let source_domain = decode_hash(&deployment["ethereum"]["sourceDomain"], "sourceDomain")?;
    let queue: Address = deployment["ethereum"]["queue"]
        .as_str()
        .ok_or_else(|| anyhow!("deployment queue is missing"))?
        .parse()?;
    ensure!(
        beefy_relay::bridge_domain(source_domain, chain_id, queue.into_array())?
            == source.bridge_domain,
        "source domain is not bound to this Hoodi queue"
    );

    let api = GearApi::builder()
        .suri(suri)
        .uri(source_rpc)
        .build()
        .await?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let signer_genesis = signer
        .api()
        .legacy()
        .chain_get_block_hash(Some(0u32.into()))
        .await?
        .ok_or_else(|| anyhow!("source signer genesis block is missing"))?;
    ensure!(
        signer_genesis.0 == source.source_genesis,
        "Gear signer is connected to a different genesis"
    );
    let _signer_owner = gear_common::signer_ownership::lock_signer(
        source.source_genesis.into(),
        <[u8; 32]>::from(api.account_id().clone()),
    )?;
    reconcile_saved_setup(&api, &stack_path).await?;
    let setup = ActorId::from(<[u8; 32]>::from(api.account_id().clone()));
    let manager_id = actor(
        stack["programs"]["vftManager"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("VFT manager ID missing"))?,
    )?;
    ensure!(
        deployment["gearManager"] == format!("0x{}", hex::encode(manager_id.into_bytes()))
            && setup != manager_id,
        "deployment manager identity differs or setup signer aliases the manager"
    );
    let checkpoint_id = actor(
        stack["checkpoint"]
            .as_str()
            .ok_or_else(|| anyhow!("checkpoint program ID missing"))?,
    )?;
    let checkpoint_slot = stack["checkpointSlot"]
        .as_u64()
        .ok_or_else(|| anyhow!("checkpoint slot missing"))?;
    let checkpoint_hash = decode_hash(&stack["checkpointHash"], "checkpoint hash")?;

    ensure!(
        ethereum.receiver_address
            == deployment["manager"]
                .as_str()
                .ok_or_else(|| anyhow!("deployment manager missing"))?
                .parse::<Address>()?,
        "Hoodi receiver differs from deployment manifest"
    );
    ethereum
        .verify_token_bindings(manager_id.into_bytes())
        .await?;
    let evm_configuration =
        inspect_ethereum_configuration(&ethereum, manager_id.into_bytes()).await?;
    ensure!(
        deployment["ethereumConfiguration"] == evm_configuration,
        "Hoodi governance, token, queue, or payment identity changed since tokens-manifest"
    );
    let anchor_block = deployment["anchor"]["block"]
        .as_u64()
        .ok_or_else(|| anyhow!("signed anchor block missing"))?;
    let evm_checkpoint = ethereum.checkpoint().await?;
    ensure!(
        evm_checkpoint.block == anchor_block && evm_checkpoint.root == [0; 32],
        "Hoodi token client is not at the fresh anchor with a zero root"
    );

    let (finalized_hash, _, _) = finalized_evidence(&source).await?;
    ensure_source_bridge_ready(&source, finalized_hash).await?;
    let programs = verify_stack_programs(
        &api,
        finalized_hash,
        &stack,
        &format!("0x{}", hex::encode(source.source_genesis)),
    )
    .await?;
    let remoting = GClientRemoting::new(api.clone());
    let finalized_sails_hash = sails_block_hash(finalized_hash);
    let checkpoint = checkpoint_light_client_client::ServiceCheckpointFor::new(remoting.clone())
        .get(checkpoint_slot)
        .at_block(finalized_sails_hash)
        .recv(checkpoint_id)
        .await
        .map_err(|error| anyhow!("checkpoint query failed: {error:?}"))?
        .map_err(|error| anyhow!("checkpoint trust anchor missing: {error:?}"))?;
    ensure!(
        checkpoint.0 == checkpoint_slot && checkpoint.1.as_ref() == checkpoint_hash,
        "Gear checkpoint trust anchor differs from token-stack manifest"
    );
    let events_id = actor(
        stack["programs"]["ethEventsElectra"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Electra verifier ID missing"))?,
    )?;
    let event_verifier = eth_events_electra_client::EthereumEventClient::new(remoting.clone());
    ensure!(
        event_verifier
            .checkpoint_light_client_address()
            .at_block(finalized_sails_hash)
            .recv(events_id)
            .await?
            == checkpoint_id,
        "Electra event verifier targets a different checkpoint program"
    );
    let proxy_id = actor(
        stack["programs"]["historicalProxy"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("historical proxy ID missing"))?,
    )?;
    let endpoints = historical_proxy_client::HistoricalProxy::new(remoting.clone())
        .endpoints()
        .at_block(finalized_sails_hash)
        .recv(proxy_id)
        .await?;
    ensure!(
        endpoints == vec![(checkpoint_slot, events_id)],
        "historical proxy endpoint differs from checkpoint and Electra verifier"
    );

    let manager = vft_manager_client::VftManager::new(remoting.clone());
    let manager_admin = manager
        .admin()
        .at_block(finalized_sails_hash)
        .recv(manager_id)
        .await?;
    let pause_admin = manager
        .pause_admin()
        .at_block(finalized_sails_hash)
        .recv(manager_id)
        .await?;
    ensure!(
        manager_admin == setup && pause_admin == setup,
        "setup signer is not both VFT manager administrator and pause administrator"
    );
    ensure!(
        manager
            .gear_bridge_builtin()
            .at_block(finalized_sails_hash)
            .recv(manager_id)
            .await?
            == actor(BUILTIN)?
            && manager
                .historical_proxy_address()
                .at_block(finalized_sails_hash)
                .recv(manager_id)
                .await?
                == proxy_id
            && !manager
                .is_emergency_stopped()
                .at_block(finalized_sails_hash)
                .recv(manager_id)
                .await?,
        "fresh Gear manager builtin, proxy, or emergency-stop state differs"
    );
    let manager_paused = manager
        .is_paused()
        .at_block(finalized_sails_hash)
        .recv(manager_id)
        .await?;
    let current_erc20_manager = manager
        .erc_20_manager_address()
        .at_block(finalized_sails_hash)
        .recv(manager_id)
        .await?;
    let erc20_manager: Address = deployment["manager"]
        .as_str()
        .ok_or_else(|| anyhow!("EVM manager missing"))?
        .parse()?;
    let wanted_h160 = H160::from_slice(erc20_manager.as_slice());
    if let Some(current) = current_erc20_manager {
        ensure!(
            current == wanted_h160,
            "Gear manager is attached to a different ERC20 manager"
        );
    }
    let config = manager
        .get_config()
        .at_block(finalized_sails_hash)
        .recv(manager_id)
        .await?;
    let wanted_config = expected_config();
    ensure!(
        configuration_matches(&config, &wanted_config),
        "fresh Gear manager configuration differs"
    );
    let was_ready = stack["configuration"]["status"] == "ready";
    ensure!(
        manager_paused
            || was_ready
            || matches!(
                stack["configuration"]["actions"]["unpause"]["status"].as_str(),
                Some("pending" | "verified")
            ),
        "Gear manager must stay paused throughout configuration"
    );
    ensure!(
        !manager_paused || !was_ready,
        "Gear manager was safety-paused after configuration; configure will not unpause it"
    );

    let payment_id = actor(
        stack["programs"]["bridgingPayment"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Gear payment program ID missing"))?,
    )?;
    let payment_state = bridging_payment_client::BridgingPayment::new(remoting.clone())
        .get_state()
        .at_block(finalized_sails_hash)
        .recv(payment_id)
        .await?;
    ensure!(
        payment_state.admin_address == setup
            && payment_state.fee == 1_000_000_000_000
            && payment_state.priority_fee == 0,
        "Gear payment identity, ordinary fee or reserved historical field differs from fresh setup"
    );

    let mut pairs = Vec::with_capacity(GEAR_TOKEN_SPECS.len());
    for spec in GEAR_TOKEN_SPECS {
        let peer = actor(
            stack["programs"][spec.component]["id"]
                .as_str()
                .ok_or_else(|| anyhow!("{} ID missing", spec.component))?,
        )?;
        let record = evm_configuration["tokens"]
            .as_array()
            .and_then(|tokens| {
                tokens
                    .iter()
                    .find(|token| token["symbol"] == spec.evm_symbol)
            })
            .ok_or_else(|| anyhow!("{} EVM token missing", spec.evm_symbol))?;
        let address: Address = record["address"]
            .as_str()
            .ok_or_else(|| anyhow!("{} EVM address missing", spec.evm_symbol))?
            .parse()?;
        let metadata = vft_client::VftMetadata::new(remoting.clone());
        ensure!(
            metadata
                .name()
                .at_block(finalized_sails_hash)
                .recv(peer)
                .await?
                == spec.name
                && metadata
                    .symbol()
                    .at_block(finalized_sails_hash)
                    .recv(peer)
                    .await?
                    == spec.symbol
                && metadata
                    .decimals()
                    .at_block(finalized_sails_hash)
                    .recv(peer)
                    .await?
                    == spec.decimals,
            "{} Gear peer metadata differs from expected test asset",
            spec.component
        );
        if spec.component == "nativeVft" {
            ensure_native_minimum_balance(
                vft_vara_client::VftExtension::new(remoting.clone())
                    .minimum_balance()
                    .at_block(finalized_sails_hash)
                    .recv(peer)
                    .await?,
            )?;
        }
        let admin = vft_client::VftAdmin::new(remoting.clone());
        let minter = token_authority(&admin, finalized_hash, peer, "minter").await?;
        let burner = token_authority(&admin, finalized_hash, peer, "burner").await?;
        let ethereum_supply = spec.supply == TokenSupply::Ethereum;
        if ethereum_supply {
            authority_handoff(minter, setup, manager_id)?;
        } else {
            ensure!(
                minter == setup,
                "{} native/origin mint authority changed",
                spec.component
            );
        }
        if ethereum_supply || spec.component == "nativeVft" {
            authority_handoff(burner, setup, manager_id)?;
        } else {
            ensure!(
                burner == setup,
                "ordinary Gear token burn authority changed"
            );
        }
        pairs.push((spec, peer, address));
    }
    let desired: Vec<(ActorId, H160, TokenSupply)> = pairs
        .iter()
        .map(|(spec, peer, address)| {
            (
                *peer,
                H160::from_slice(address.as_slice()),
                spec.supply.clone(),
            )
        })
        .collect();
    let current_mappings = manager
        .vara_to_eth_addresses()
        .at_block(finalized_sails_hash)
        .recv(manager_id)
        .await?;
    let _ = missing_token_mappings(&current_mappings, &desired)?;
    let mut identity = json!({
        "sourceRpc": source_rpc,
        "sourceGenesis": format!("0x{}", hex::encode(source.source_genesis)),
        "bridgeDomain": format!("0x{}", hex::encode(source.bridge_domain)),
        "setupAccount": format!("0x{}", hex::encode(setup.into_bytes())),
        "gearManager": format!("0x{}", hex::encode(manager_id.into_bytes())),
        "gearPrograms": programs,
        "checkpoint": {"id": format!("0x{}", hex::encode(checkpoint_id.into_bytes())), "slot": checkpoint_slot, "hash": format!("0x{}", hex::encode(checkpoint_hash))},
        "gearPayment": {"id": format!("0x{}", hex::encode(payment_id.into_bytes())), "fee": payment_state.fee.to_string(), "priorityFee": payment_state.priority_fee.to_string(), "admin": format!("0x{}", hex::encode(setup.into_bytes()))},
        "ethereum": evm_configuration,
        "gearTokenPeers": pairs.iter().map(|(spec, peer, address)| json!({
            "component": spec.component,
            "id": format!("0x{}", hex::encode(peer.into_bytes())),
            "erc20": format!("{address:#x}"),
            "supply": if spec.supply == TokenSupply::Gear { "gear" } else { "ethereum" },
        })).collect::<Vec<_>>(),
    });
    if let Some(network) = &fixture {
        identity["ownedFixtureNetwork"] = serde_json::to_value(network)?;
    }
    let was_ready = pin_configuration_identity(&stack_path, identity)?;
    stack = read_config_stack(&stack_path)?;
    if was_ready {
        ensure!(
            !manager_paused,
            "ready token manager is paused; refusing implicit recovery unpause"
        );
    }

    let gas = api.block_gas_limit()?;
    for (spec, peer, _) in &pairs {
        let key = format!("{}.allocateShards", spec.component);
        if stack["configuration"]["actions"][&key]["status"] == "verified" {
            continue;
        }
        begin_configuration_action(
            &stack_path,
            &key,
            json!({"peer": format!("0x{}", hex::encode(peer.into_bytes())), "operation": "allocate-configured-shards"}),
        )?;
        let remoting = JournaledRemoting::new(api.clone(), &stack_path, &key);
        vft_client::allocate_shards(remoting, *peer, gas)
            .await
            .map_err(|error| anyhow!("{} shard allocation: {error:?}", spec.component))?;
        let (_, _, evidence) = finalized_evidence(&source).await?;
        mark_configuration_action_verified(&stack_path, &key, evidence)?;
        stack = read_config_stack(&stack_path)?;
    }

    let manager_intent = json!({"manager": format!("{wanted_h160:?}")});
    let install_manager = prepare_configuration_action(
        &stack_path,
        "erc20Manager",
        manager_intent,
        current_erc20_manager.is_some(),
    )?;
    if install_manager {
        vft_manager_client::VftManager::new(JournaledRemoting::new(
            api.clone(),
            &stack_path,
            "erc20Manager",
        ))
        .update_erc_20_manager_address(wanted_h160)
        .with_gas_limit(gas)
        .send_recv(manager_id)
        .await
        .map_err(|error| anyhow!("install Gear ERC20 manager pointer: {error:?}"))?;
    }
    let (at, _, evidence) = finalized_evidence(&source).await?;
    ensure!(
        manager
            .erc_20_manager_address()
            .at_block(sails_block_hash(at))
            .recv(manager_id)
            .await?
            == Some(wanted_h160),
        "Gear ERC20 manager pointer did not persist"
    );
    if stack["configuration"]["actions"]["erc20Manager"]["status"] != "verified" {
        mark_configuration_action_verified(
            &stack_path,
            "erc20Manager",
            json!({"finalized": evidence, "manager": format!("{wanted_h160:?}")}),
        )?;
        stack = read_config_stack(&stack_path)?;
    }

    for (spec, peer, _) in &pairs {
        for role in ["minter", "burner"] {
            if spec.supply == TokenSupply::Gear
                && (spec.component != "nativeVft" || role == "minter")
            {
                continue;
            }
            let key = format!("{}.{}", spec.component, role);
            let intent = json!({"peer": format!("0x{}", hex::encode(peer.into_bytes())), "role": role, "authority": format!("0x{}", hex::encode(manager_id.into_bytes()))});
            let (current_at, _, _) = finalized_evidence(&source).await?;
            let admin = vft_client::VftAdmin::new(remoting.clone());
            let current = token_authority(&admin, current_at, *peer, role).await?;
            let should_change = authority_handoff(current, setup, manager_id)?;
            let send_handoff =
                prepare_configuration_action(&stack_path, &key, intent, !should_change)?;
            if send_handoff {
                let mut writer = vft_client::VftAdmin::new(JournaledRemoting::new(
                    api.clone(),
                    &stack_path,
                    &key,
                ));
                if role == "minter" {
                    writer
                        .set_minter(manager_id)
                        .with_gas_limit(gas)
                        .send_recv(*peer)
                        .await
                        .map_err(|error| anyhow!("{} minter handoff: {error:?}", spec.component))?;
                } else {
                    writer
                        .set_burner(manager_id)
                        .with_gas_limit(gas)
                        .send_recv(*peer)
                        .await
                        .map_err(|error| anyhow!("{} burner handoff: {error:?}", spec.component))?;
                }
            }
            let (at, _, evidence) = finalized_evidence(&source).await?;
            let readback = token_authority(&admin, at, *peer, role).await?;
            ensure!(
                readback == manager_id,
                "{} {role} authority did not move to the manager",
                spec.component
            );
            if stack["configuration"]["actions"][&key]["status"] != "verified" {
                mark_configuration_action_verified(
                    &stack_path,
                    &key,
                    json!({"finalized": evidence, "authority": format!("0x{}", hex::encode(manager_id.into_bytes()))}),
                )?;
                stack = read_config_stack(&stack_path)?;
            }
        }
    }

    for (spec, peer, address) in &pairs {
        let key = format!("{}.mapping", spec.component);
        let wanted = H160::from_slice(address.as_slice());
        let supply_name = if spec.supply == TokenSupply::Gear {
            "gear"
        } else {
            "ethereum"
        };
        let intent = json!({"peer": format!("0x{}", hex::encode(peer.into_bytes())), "erc20": format!("{address:#x}"), "supply": supply_name});
        let (before, _, _) = finalized_evidence(&source).await?;
        let current = manager
            .vara_to_eth_addresses()
            .at_block(sails_block_hash(before))
            .recv(manager_id)
            .await?;
        let missing = missing_token_mappings(&current, &desired)?;
        let add_mapping = prepare_configuration_action(
            &stack_path,
            &key,
            intent,
            !missing.iter().any(|(id, _, _)| id == peer),
        )?;
        if add_mapping {
            let mut writer = vft_manager_client::VftManager::new(JournaledRemoting::new(
                api.clone(),
                &stack_path,
                &key,
            ));
            writer
                .map_vara_to_eth_address(*peer, wanted, spec.supply.clone())
                .with_gas_limit(gas)
                .send_recv(manager_id)
                .await
                .map_err(|error| anyhow!("{} ERC20 mapping: {error:?}", spec.component))?;
        }
        let (at, _, evidence) = finalized_evidence(&source).await?;
        let mappings = manager
            .vara_to_eth_addresses()
            .at_block(sails_block_hash(at))
            .recv(manager_id)
            .await?;
        ensure!(
            missing_token_mappings(&mappings, &desired)?
                .iter()
                .all(|(id, _, _)| id != peer),
            "{} Gear/Ethereum mapping did not persist",
            spec.component
        );
        if stack["configuration"]["actions"][&key]["status"] != "verified" {
            mark_configuration_action_verified(
                &stack_path,
                &key,
                json!({"finalized": evidence, "peer": format!("0x{}", hex::encode(peer.into_bytes())), "erc20": format!("{address:#x}"), "supply": supply_name}),
            )?;
            stack = read_config_stack(&stack_path)?;
        }
    }

    let native_id = pairs
        .iter()
        .find(|(spec, _, _)| spec.component == "nativeVft")
        .map(|(_, peer, _)| *peer)
        .ok_or_else(|| anyhow!("native wrapper is absent"))?;
    let native = vft_vara_client::NativeEscrow::new(remoting.clone());
    let native_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    let (at, _, _) = finalized_evidence(&source).await?;
    let wrapper_manager = native
        .manager()
        .at_block(sails_block_hash(at))
        .recv(native_id)
        .await?;
    let manager_wrapper = manager
        .native_wrapper()
        .at_block(sails_block_hash(at))
        .recv(manager_id)
        .await?;
    let native_paused = native_admin
        .is_paused()
        .at_block(sails_block_hash(at))
        .recv(native_id)
        .await?;
    ensure!(
        wrapper_manager.is_none() || wrapper_manager == Some(manager_id),
        "native wrapper names another manager"
    );
    ensure!(
        manager_wrapper.is_none() || manager_wrapper == Some(native_id),
        "manager names another native wrapper"
    );
    let native_bound = wrapper_manager == Some(manager_id) && manager_wrapper == Some(native_id);
    ensure!(
        !was_ready || (native_bound && !native_paused),
        "ready native configuration changed or was safety-paused"
    );
    let native_identity = json!({"manager": format!("0x{}", hex::encode(manager_id.into_bytes())), "wrapper": format!("0x{}", hex::encode(native_id.into_bytes()))});
    if !native_bound {
        ensure!(
            manager_paused,
            "manager must remain paused while binding native settlement"
        );
        let key = "native.pause";
        if prepare_configuration_action(&stack_path, key, native_identity.clone(), native_paused)? {
            vft_vara_client::VftAdmin::new(JournaledRemoting::new(api.clone(), &stack_path, key))
                .pause()
                .with_gas_limit(gas)
                .send_recv(native_id)
                .await?;
        }
        let (at, _, evidence) = finalized_evidence(&source).await?;
        ensure!(
            native_admin
                .is_paused()
                .at_block(sails_block_hash(at))
                .recv(native_id)
                .await?,
            "native wrapper did not pause"
        );
        if read_config_stack(&stack_path)?["configuration"]["actions"][key]["status"] != "verified"
        {
            mark_configuration_action_verified(
                &stack_path,
                key,
                json!({"finalized": evidence, "paused": true}),
            )?;
        }
    }
    for (key, target, expected) in [
        ("native.wrapperManager", native_id, manager_id),
        ("native.managerWrapper", manager_id, native_id),
    ] {
        let (at, _, _) = finalized_evidence(&source).await?;
        let current = if target == native_id {
            native
                .manager()
                .at_block(sails_block_hash(at))
                .recv(native_id)
                .await?
        } else {
            manager
                .native_wrapper()
                .at_block(sails_block_hash(at))
                .recv(manager_id)
                .await?
        };
        ensure!(
            current.is_none() || current == Some(expected),
            "native policy changed during configuration"
        );
        if prepare_configuration_action(
            &stack_path,
            key,
            native_identity.clone(),
            current == Some(expected),
        )? {
            if target == native_id {
                vft_vara_client::NativeEscrow::new(JournaledRemoting::new(
                    api.clone(),
                    &stack_path,
                    key,
                ))
                .configure_manager(manager_id)
                .with_gas_limit(gas)
                .send_recv(native_id)
                .await?;
            } else {
                vft_manager_client::VftManager::new(JournaledRemoting::new(
                    api.clone(),
                    &stack_path,
                    key,
                ))
                .configure_native_wrapper(Some(native_id))
                .with_gas_limit(gas)
                .send_recv(manager_id)
                .await?;
            }
        }
        let (at, _, evidence) = finalized_evidence(&source).await?;
        let observed = if target == native_id {
            native
                .manager()
                .at_block(sails_block_hash(at))
                .recv(native_id)
                .await?
        } else {
            manager
                .native_wrapper()
                .at_block(sails_block_hash(at))
                .recv(manager_id)
                .await?
        };
        ensure!(
            observed == Some(expected),
            "native settlement binding did not persist"
        );
        if read_config_stack(&stack_path)?["configuration"]["actions"][key]["status"] != "verified"
        {
            mark_configuration_action_verified(
                &stack_path,
                key,
                json!({"finalized": evidence, "policy": native_identity}),
            )?;
        }
    }
    let (at, _, _) = finalized_evidence(&source).await?;
    ensure_native_minimum_balance(
        vft_vara_client::VftExtension::new(remoting.clone())
            .minimum_balance()
            .at_block(sails_block_hash(at))
            .recv(native_id)
            .await?,
    )?;
    let native_paused = native_admin
        .is_paused()
        .at_block(sails_block_hash(at))
        .recv(native_id)
        .await?;
    let key = "native.resume";
    if prepare_configuration_action(&stack_path, key, native_identity, !native_paused)? {
        vft_vara_client::VftAdmin::new(JournaledRemoting::new(api.clone(), &stack_path, key))
            .resume()
            .with_gas_limit(gas)
            .send_recv(native_id)
            .await?;
    }
    let (at, _, evidence) = finalized_evidence(&source).await?;
    ensure!(
        !native_admin
            .is_paused()
            .at_block(sails_block_hash(at))
            .recv(native_id)
            .await?,
        "configured native wrapper remains paused"
    );
    if read_config_stack(&stack_path)?["configuration"]["actions"][key]["status"] != "verified" {
        mark_configuration_action_verified(
            &stack_path,
            key,
            json!({"finalized": evidence, "paused": false}),
        )?;
    }

    let (at, _, _) = finalized_evidence(&source).await?;
    let manager = vft_manager_client::VftManager::new(remoting.clone());
    ensure_source_bridge_ready(&source, at).await?;
    ensure!(
        manager
            .erc_20_manager_address()
            .at_block(sails_block_hash(at))
            .recv(manager_id)
            .await?
            == Some(wanted_h160)
            && manager
                .gear_bridge_builtin()
                .at_block(sails_block_hash(at))
                .recv(manager_id)
                .await?
                == actor(BUILTIN)?
            && manager
                .historical_proxy_address()
                .at_block(sails_block_hash(at))
                .recv(manager_id)
                .await?
                == proxy_id
            && !manager
                .is_emergency_stopped()
                .at_block(sails_block_hash(at))
                .recv(manager_id)
                .await?,
        "Gear manager identity changed during configuration"
    );
    let final_mappings = manager
        .vara_to_eth_addresses()
        .at_block(sails_block_hash(at))
        .recv(manager_id)
        .await?;
    ensure!(
        missing_token_mappings(&final_mappings, &desired)?.is_empty(),
        "Gear token mapping set is incomplete"
    );
    ensure_native_minimum_balance(
        vft_vara_client::VftExtension::new(remoting.clone())
            .minimum_balance()
            .at_block(sails_block_hash(at))
            .recv(native_id)
            .await?,
    )?;
    for (spec, peer, _) in &pairs {
        let admin = vft_client::VftAdmin::new(remoting.clone());
        ensure!(
            token_authority(&admin, at, *peer, "minter").await?
                == if spec.supply == TokenSupply::Ethereum {
                    manager_id
                } else {
                    setup
                }
                && token_authority(&admin, at, *peer, "burner").await?
                    == if spec.supply == TokenSupply::Ethereum || spec.component == "nativeVft" {
                        manager_id
                    } else {
                        setup
                    },
            "{} token authority handoff is incomplete",
            spec.component
        );
    }
    ensure!(
        inspect_ethereum_configuration(&ethereum, manager_id.into_bytes()).await?
            == evm_configuration,
        "Hoodi EVM deployment identity changed during Gear configuration"
    );

    let unpause_intent = json!({"manager": format!("0x{}", hex::encode(manager_id.into_bytes()))});
    let should_unpause = prepare_configuration_action(
        &stack_path,
        "unpause",
        unpause_intent.clone(),
        !manager_paused,
    )?;
    if should_unpause {
        vft_manager_client::VftManager::new(JournaledRemoting::new(
            api.clone(),
            &stack_path,
            "unpause",
        ))
        .unpause()
        .with_gas_limit(gas)
        .send_recv(manager_id)
        .await
        .map_err(|error| anyhow!("unpause configured Gear manager: {error:?}"))?;
    }
    let (ready_at, _, ready_evidence) = finalized_evidence(&source).await?;
    ensure!(
        !manager
            .is_paused()
            .at_block(sails_block_hash(ready_at))
            .recv(manager_id)
            .await?
            && !manager
                .is_emergency_stopped()
                .at_block(sails_block_hash(ready_at))
                .recv(manager_id)
                .await?,
        "configured Gear manager did not become live"
    );
    ensure_source_bridge_ready(&source, ready_at).await?;
    let journal = read_config_stack(&stack_path)?;
    if journal["configuration"]["actions"]["unpause"]["status"] != "verified" {
        ensure_configuration_action(&stack_path, "unpause", unpause_intent)?;
        mark_configuration_action_verified(
            &stack_path,
            "unpause",
            json!({"finalized": ready_evidence, "paused": false}),
        )?;
    }
    let mut final_stack = read_config_stack(&stack_path)?;
    final_stack["configuration"]["status"] = json!("ready");
    final_stack["configuration"]["readyAt"] = ready_evidence;
    save_config_stack(&stack_path, &final_stack)?;
    println!(
        "Token configuration verified for source genesis 0x{} and manager 0x{}",
        hex::encode(source.source_genesis),
        hex::encode(manager_id.into_bytes())
    );
    Ok(())
}

fn state_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
#[cfg(test)]
mod configuration_recovery_tests {
    use super::*;

    #[test]
    fn native_wrapper_minimum_balance_rejects_dust_policy() -> Result<()> {
        use sails_rs::prelude::U256;

        ensure_native_minimum_balance(U256::zero())?;
        for minimum in [U256::one(), U256::from(1_000_000_000_000u64), U256::MAX] {
            assert!(ensure_native_minimum_balance(minimum).is_err());
        }
        Ok(())
    }

    #[test]
    fn token_mapping_resume_preserves_origin_and_rejects_conflicts() -> Result<()> {
        let desired: Vec<_> = GEAR_TOKEN_SPECS
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                let byte = u8::try_from(index + 1).unwrap();
                (
                    ActorId::from([byte; 32]),
                    H160::from([byte; 20]),
                    spec.supply.clone(),
                )
            })
            .collect();
        assert_eq!(
            missing_token_mappings(&desired[..5], &desired)?,
            vec![desired[5].clone()]
        );
        assert!(missing_token_mappings(&desired, &desired)?.is_empty());
        let mut conflicting = desired.clone();
        conflicting[5].2 = TokenSupply::Ethereum;
        assert!(missing_token_mappings(&conflicting, &desired).is_err());
        conflicting[5] = desired[0].clone();
        assert!(missing_token_mappings(&conflicting, &desired).is_err());
        Ok(())
    }

    fn directory(name: &str) -> Result<PathBuf> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("beefy-setup-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(path)
    }

    #[test]
    fn configuration_requires_original_reply_before_readback_verification() -> Result<()> {
        let directory = directory("actions")?;
        let path = directory.join("token-stack.json");
        save(&directory, &json!({"configuration":{"actions":{}}}))?;
        let intent = json!({"role":"minter","authority":"original"});
        assert!(prepare_configuration_action(
            &path,
            "role",
            intent.clone(),
            false,
        )?);
        let mut state = read_config_stack(&path)?;
        state["configuration"]["actions"]["role"]["messages"] =
            json!([{"messageId":"original","status":"submitted"}]);
        save_config_stack(&path, &state)?;
        assert!(
            mark_configuration_action_verified(&path, "role", json!({"authority":"original"}))
                .is_err()
        );
        assert!(prepare_configuration_action(
            &path,
            "role",
            intent.clone(),
            false,
        )?);
        assert!(!prepare_configuration_action(
            &path,
            "role",
            intent.clone(),
            true,
        )?);
        assert!(prepare_configuration_action(
            &path,
            "role",
            json!({"role":"minter","authority":"replacement"}),
            true,
        )
        .is_err());
        update_configuration_message(
            &path,
            "role",
            0,
            json!({"status":"reply_received","replyEvidence":{"blockNumber":20,"blockHash":"original"}}),
        )?;
        mark_configuration_action_verified(&path, "role", json!({"authority":"original"}))?;
        assert_eq!(
            read_config_stack(&path)?["configuration"]["actions"]["role"]["messages"][0]
                ["messageId"],
            "original"
        );
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn identical_shard_calls_replay_by_ordinal_and_require_exact_intent() -> Result<()> {
        let intent = setup_message_intent(ActorId::from([3; 32]), &[1, 2], 100, 0, None);
        let mut action = json!({"messages":[]});
        assert!(!match_setup_message(&action, 0, &intent)?);
        let mut first = intent.clone();
        first["submission"] = json!({"rawExtrinsic":"0x0102"});
        first["status"] = json!("submitted");
        action["messages"]
            .as_array_mut()
            .unwrap()
            .push(first.clone());
        assert!(match_setup_message(&action, 0, &intent)?);
        assert!(match_setup_message(&action, 1, &intent).is_err());
        first["status"] = json!("reply_received");
        first["replyEvidence"] = json!({"payload":"0x01","blockNumber":20});
        action["messages"][0] = first.clone();
        assert!(!match_setup_message(&action, 1, &intent)?);
        action["messages"]
            .as_array_mut()
            .unwrap()
            .push(first.clone());
        assert!(match_setup_message(&action, 1, &intent)?);
        assert!(!match_setup_message(&action, 2, &intent)?);
        assert!(match_setup_message(&action, 3, &intent).is_err());
        let mut changed = intent.clone();
        changed["gasLimit"] = json!(101);
        assert!(match_setup_message(&action, 0, &changed).is_err());
        changed = intent.clone();
        changed["payload"] = json!("0x03");
        assert!(match_setup_message(&action, 0, &changed).is_err());
        action["messages"][0]["submission"] = Value::Null;
        assert!(match_setup_message(&action, 0, &intent).is_err());
        Ok(())
    }

    #[test]
    fn unsigned_constructor_resumes_and_status_merge_retains_originals() -> Result<()> {
        let directory = directory("constructor")?;
        let path = directory.join("token-stack.json");
        save(
            &directory,
            &json!({"sourceGenesis":format!("0x{}",hex::encode([1;32])),"programs":{}}),
        )?;
        let (id, new, salt) = intent(&directory, "test", &[1, 2, 3])?;
        assert!(new);
        assert!(intent(&directory, "test", &[1, 2, 3])?.1);
        let constructor = setup_message_intent(
            id,
            &[4],
            100,
            0,
            Some((CodeId::generate(&[1, 2, 3]), &salt)),
        );
        ensure_configuration_action(&path, "constructor.test", constructor.clone())?;
        let mut current = read_config_stack(&path)?;
        current["programs"]["test"]["status"] = json!("pending");
        current["configuration"]["actions"]["constructor.test"]["messages"] =
            json!([{"submission":{"rawExtrinsic":"0xoriginal"}}]);
        save_config_stack(&path, &current)?;
        assert!(intent(&directory, "test", &[1, 2, 3])?.1);
        set_program_status(&directory, "test", "initializing")?;
        assert_eq!(
            read_config_stack(&path)?["configuration"]["actions"]["constructor.test"]["messages"]
                [0]["submission"]["rawExtrinsic"],
            "0xoriginal"
        );
        assert!(!intent(&directory, "test", &[1, 2, 3])?.1);
        current = read_config_stack(&path)?;
        current["programs"]["test"]["status"] = json!("pending");
        current["configuration"]["actions"]["constructor.test"]["messages"] = json!([]);
        save_config_stack(&path, &current)?;
        // Legacy pending does not authorize another constructor; only exact finalized adoption may finish it.
        assert!(!intent(&directory, "test", &[1, 2, 3])?.1);
        assert!(intent(&directory, "test", &[9]).is_err());
        assert!(read_config_stack(&path)?["configuration"]["identity"].is_null());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn stack_lock_canonicalizes_aliases_and_rejects_second_writer() -> Result<()> {
        let directory = directory("lock")?;
        save(&directory, &json!({"programs":{}}))?;
        let owner = lock_token_stack(&directory.join("token-stack.json"))?;
        assert!(lock_token_stack(&directory.join(".").join("token-stack.json")).is_err());
        #[cfg(unix)]
        {
            let aliases = directory.join("aliases");
            fs::create_dir(&aliases)?;
            let alias = aliases.join("token-stack.json");
            std::os::unix::fs::symlink("../token-stack.json", &alias)?;
            assert!(lock_token_stack(&alias).is_err());
        }
        drop(owner);
        assert!(lock_token_stack(&directory.join("token-stack.json")).is_ok());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn finalized_code_only_uploads_on_authenticated_absence() -> Result<()> {
        assert!(finalized_code_result(Ok(vec![1, 2]), &[1, 2])?);
        assert!(!finalized_code_result(
            Err(gsdk::Error::StorageEntryNotFound),
            &[1, 2]
        )?);
        assert!(finalized_code_result(Ok(vec![1, 3]), &[1, 2]).is_err());
        // A malformed/transport error is not absence and must not authorize upload.
        assert!(finalized_code_result(Err(gsdk::Error::SubscriptionDied), &[1, 2]).is_err());
        Ok(())
    }
}
