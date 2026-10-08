use crate::{hoodi::number, source::Source};
use alloy::{
    primitives::{Address, B256, U256},
    providers::Provider,
    sol,
};
use anyhow::{anyhow, ensure, Context as _, Result};
use bridging_payment_client::traits::{BridgingPayment, BridgingPaymentFactory};
use checkpoint_light_client_client::traits::ServiceCheckpointFor;
use eth_events_electra_client::traits::{EthEventsElectraFactory, EthereumEventClient};
use gclient::{EventProcessor, GearApi};
use gear_core::{
    ids::{prelude::*, ActorId, CodeId},
    program::ProgramState,
};
use gsdk::{
    ext::subxt::{config::polkadot::PolkadotExtrinsicParamsBuilder, utils::H256 as GearHash},
    gear::{
        self, gear::Event as GearEvent, runtime_types::gear_common::event::MessageEntry,
        Event as RuntimeEvent,
    },
    AsGear,
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
};
use vft_client::traits::{Vft as _, VftAdmin, VftFactory, VftMetadata};
use vft_manager_client::{
    traits::{VftManager, VftManagerFactory},
    Config, InitConfig, TokenSupply,
};
use vft_vara_client::{
    traits::{
        NativeEscrow as _, VftAdmin as _, VftMetadata as _, VftNativeExchange as _, VftVaraFactory,
    },
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
sol! {
    #[sol(rpc)]
    interface RecoveryQueueBinding {
        function verifier() external view returns (address);
        function recoveryController() external view returns (address);
        event RecoveryControllerInstalled(address indexed controller, address indexed recoveryWallet);
        event RecoveryVerifierActivated(address indexed previousVerifier, address indexed newVerifier);
    }
    #[sol(rpc)]
    interface RecoveryControllerBinding {
        function messageQueue() external view returns (address);
        function recoveryWallet() external view returns (address);
        function proposalNonce() external view returns (uint256);
        function RECOVERY_DELAY() external view returns (uint256);
        function pendingRecovery() external view returns (
            uint256 proposalId, address expectedOldVerifier, address candidateVerifier,
            uint256 executeAfter, bytes32 expectedOldVerifierCodeHash,
            address expectedOldClient, bytes32 expectedOldClientCodeHash,
            bytes32 candidateVerifierCodeHash, address candidateClient,
            bytes32 candidateClientCodeHash, bool exists
        );
        event RecoveryProposed(
            uint256 indexed proposalId, address indexed expectedOldVerifier, address indexed candidateVerifier,
            uint256 executeAfter, bytes32 expectedOldVerifierCodeHash, address expectedOldClient,
            bytes32 expectedOldClientCodeHash, bytes32 candidateVerifierCodeHash,
            address candidateClient, bytes32 candidateClientCodeHash
        );
        event RecoveryCancelled(uint256 indexed proposalId);
        event RecoveryExecuted(uint256 indexed proposalId, address indexed previousVerifier, address indexed newVerifier);
    }
    #[sol(rpc)]
    interface RecoveryVerifierBinding {
        function beefyClient() external view returns (address);
        function messageQueue() external view returns (address);
        function destinationChainId() external view returns (uint256);
    }
    #[sol(rpc)]
    interface RecoveryClientBinding {
        function isLive() external view returns (bool);
        function latestMMRRoot() external view returns (bytes32);
        function latestBeefyBlock() external view returns (uint64);
        function sourceDomain() external view returns (bytes32);
        function bridgeDomain() external view returns (bytes32);
        function destinationChainId() external view returns (uint256);
        function destinationQueue() external view returns (address);
        function mmrStartBlock() external view returns (uint64);
    }
}
pub async fn anchor(source_rpc: &str, witness_rpc: &str) -> Result<()> {
    use crate::source::Source;
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
) -> Result<()> {
    use crate::{ethereum::Ethereum, hoodi::checkpoint_matches_source, source::Source};
    use gear_rpc_client::GearApi as SourceApi;
    use gsdk::ext::subxt::utils::H256;

    ensure!(
        source_rpc != witness_rpc,
        "two independent source endpoints required"
    );
    ensure!(
        if local_rehearsal {
            crate::local_source_rpc(ethereum_rpc)
        } else {
            ethereum_rpc.starts_with("wss://")
        },
        "Hoodi requires wss; explicit local rehearsal requires a loopback websocket"
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
        "chainId": 560048, "client": client, "verifier": verifier,
        "queue": queue, "receiver": manager,
    });
    let mut ethereum_manifest = Ethereum::token_manifest(ethereum_rpc, wallet, &addresses).await?;
    let pinned_recovery_wallet = ethereum_manifest["recoveryWallet"]
        .as_str()
        .ok_or_else(|| anyhow!("recovery wallet missing from fresh deployment manifest"))?;
    require_recovery_wallet_identity(pinned_recovery_wallet)?;
    ethereum_manifest["sourceGenesis"] = anchor["sourceGenesis"].clone();
    let ethereum = Ethereum::connect_hoodi(ethereum_rpc, wallet, &ethereum_manifest).await?;
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
        beefy_relay::bridge_domain(source_domain, 560048, queue_address)? == source.bridge_domain,
        "source domain or destination differs from source genesis lane"
    );
    let mut deployment = json!({
        "mode": "hoodi-token-stack", "localRehearsal": local_rehearsal, "manager": ethereum_manifest["receiver"],
        "gearManager": format!("0x{}", hex::encode(gear_manager.into_bytes())),
        "anchor": anchor, "ethereum": ethereum_manifest,
    });
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

// Persist the deterministic destination before a constructor is sent. A pending
// constructor is queried on restart, never sent again with an uncertain outcome.
fn intent(
    output: &Path,
    state: &mut Value,
    component: &str,
    binary: &[u8],
) -> Result<(ActorId, bool, Vec<u8>)> {
    let genesis = state["sourceGenesis"]
        .as_str()
        .expect("source genesis in token manifest");
    let salt = format!("beefy-token-hoodi-{genesis}-{component}").into_bytes();
    let id = ActorId::generate_from_user(CodeId::generate(binary), &salt);
    let address = format!("0x{}", hex::encode(id.into_bytes()));
    if state["programs"][component].is_null() {
        state["programs"][component] = json!({"id": address, "status": "pending", "salt": format!("0x{}", hex::encode(&salt))});
        save(output, state)?;
        return Ok((id, true, salt));
    }
    ensure!(
        state["programs"][component]["id"] == address,
        "{component} artifact or salt changed"
    );
    Ok((id, false, salt))
}

fn complete(output: &Path, state: &mut Value, component: &str) -> Result<()> {
    state["programs"][component]["status"] = json!("active");
    save(output, state)
}

async fn code(api: &GearApi, binary: &[u8]) -> Result<CodeId> {
    let id = CodeId::generate(binary);
    if let Err(error) = api.upload_code(binary).await {
        let signer: gsdk::signer::Signer = api.clone().into();
        ensure!(
            matches!(signer.api().original_code_storage_at(id, None).await, Ok(existing) if existing == binary),
            "code upload {id:?} failed without matching code on chain: {error}"
        );
    }
    Ok(id)
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
        crate::local_source_rpc(source_rpc),
        "public dev keys: source RPC must be loopback"
    );
    fs::create_dir_all(output)?;
    let path = output.join("token-stack.json");
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
    let on_chain = checkpoint_light_client_client::ServiceCheckpointFor::new(remoting.clone())
        .get(slot)
        .recv(checkpoint)
        .await
        .map_err(|e| anyhow!("checkpoint query failed: {e:?}"))?
        .map_err(|e| anyhow!("checkpoint not available: {e:?}"))?;
    ensure!(
        on_chain.0 == slot && on_chain.1.as_ref() == expected_hash,
        "checkpoint trust anchor does not match live Gear state"
    );
    let mut state: Value = if path.exists() {
        let state: Value = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(
            state["lane"] == "beefy-token-hoodi"
                && state["sourceGenesis"] == genesis
                && state["checkpoint"] == format!("0x{}", hex::encode(checkpoint.into_bytes()))
                && state["checkpointSlot"] == slot
                && state["checkpointHash"] == format!("0x{}", hex::encode(&expected_hash)),
            "token manifest conflicts with source or checkpoint"
        );
        state
    } else {
        let state = json!({"lane": "beefy-token-hoodi", "sourceGenesis": genesis,
            "checkpoint": format!("0x{}", hex::encode(checkpoint.into_bytes())), "checkpointSlot": slot,
            "checkpointHash": format!("0x{}", hex::encode(&expected_hash)), "programs": {}});
        save(output, &state)?;
        state
    };
    let gas = api.block_gas_limit()?;

    let proxy_code = code(&api, historical_proxy::WASM_BINARY).await?;
    let (proxy, new, salt) = intent(
        output,
        &mut state,
        "historicalProxy",
        historical_proxy::WASM_BINARY,
    )?;
    if new {
        let result = historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
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
    let mut proxy_service = historical_proxy_client::HistoricalProxy::new(remoting.clone());
    proxy_service
        .admin()
        .recv(proxy)
        .await
        .map_err(|e| anyhow!("historical proxy not readable: {e:?}"))?;
    complete(output, &mut state, "historicalProxy")?;

    let events_code = code(&api, eth_events_electra::WASM_BINARY).await?;
    let (events, new, salt) = intent(
        output,
        &mut state,
        "ethEventsElectra",
        eth_events_electra::WASM_BINARY,
    )?;
    if new {
        let result = eth_events_electra_client::EthEventsElectraFactory::new(remoting.clone())
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
    let linked = eth_events_electra_client::EthereumEventClient::new(remoting.clone())
        .checkpoint_light_client_address()
        .recv(events)
        .await
        .map_err(|e| anyhow!("Electra event verifier not readable: {e:?}"))?;
    ensure!(
        linked == checkpoint,
        "event verifier targets wrong checkpoint light client"
    );
    complete(output, &mut state, "ethEventsElectra")?;

    let endpoints = proxy_service
        .endpoints()
        .recv(proxy)
        .await
        .map_err(|e| anyhow!("proxy endpoints unreadable: {e:?}"))?;
    if endpoints.is_empty() {
        proxy_service
            .add_endpoint(slot, events)
            .with_gas_limit(gas)
            .send_recv(proxy)
            .await
            .map_err(|e| anyhow!("add event verifier endpoint: {e:?}"))?;
    }
    ensure!(
        proxy_service
            .endpoints()
            .recv(proxy)
            .await
            .map_err(|e| anyhow!("proxy endpoints unreadable: {e:?}"))?
            == vec![(slot, events)],
        "historical proxy endpoint differs from the intended verifier"
    );

    let config = Config {
        gas_for_token_ops: 10_000_000_000,
        gas_for_reply_deposit: 10_000_000_000,
        gas_to_send_request_to_builtin: 10_000_000_000,
        gas_for_swap_token_maps: 1_500_000_000,
        reply_timeout: 100,
        fee_bridge: 0,
        fee_incoming: 1_000_000_000_000,
    };
    let manager_code = code(&api, vft_manager::WASM_BINARY).await?;
    let (manager, new, salt) = intent(output, &mut state, "vftManager", vft_manager::WASM_BINARY)?;
    if new {
        let result = vft_manager_client::VftManagerFactory::new(remoting.clone())
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
    let service = vft_manager_client::VftManager::new(remoting.clone());
    ensure!(
        service
            .gear_bridge_builtin()
            .recv(manager)
            .await
            .map_err(|e| anyhow!("manager not readable: {e:?}"))?
            == actor(BUILTIN)?,
        "manager builtin mismatch"
    );
    ensure!(
        service
            .historical_proxy_address()
            .recv(manager)
            .await
            .map_err(|e| anyhow!("manager historical proxy not readable: {e:?}"))?
            == proxy,
        "manager proxy mismatch"
    );
    ensure!(
        service
            .is_paused()
            .recv(manager)
            .await
            .map_err(|e| anyhow!("manager paused state not readable: {e:?}"))?,
        "new token manager must remain paused"
    );
    let actual_config = service
        .get_config()
        .recv(manager)
        .await
        .map_err(|e| anyhow!("manager config unreadable: {e:?}"))?;
    ensure!(
        actual_config.fee_bridge == config.fee_bridge
            && actual_config.fee_incoming == config.fee_incoming
            && actual_config.gas_for_token_ops == config.gas_for_token_ops
            && actual_config.reply_timeout == config.reply_timeout,
        "manager configuration mismatch"
    );
    complete(output, &mut state, "vftManager")?;

    let payment_code = code(&api, bridging_payment::WASM_BINARY).await?;
    let (payment, new, salt) = intent(
        output,
        &mut state,
        "bridgingPayment",
        bridging_payment::WASM_BINARY,
    )?;
    let initial_state = bridging_payment_client::State {
        admin_address: ActorId::from(<[u8; 32]>::from(api.account_id().clone())),
        fee: 1_000_000_000_000,
        priority_fee: 2_000_000_000_000,
    };
    if new {
        let deployed = bridging_payment_client::BridgingPaymentFactory::new(remoting.clone())
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
    let actual = bridging_payment_client::BridgingPayment::new(remoting.clone())
        .get_state()
        .recv(payment)
        .await
        .map_err(|e| anyhow!("Gear payment state unreadable: {e:?}"))?;
    ensure!(
        actual.admin_address == initial_state.admin_address
            && actual.fee == initial_state.fee
            && actual.priority_fee == initial_state.priority_fee,
        "Gear payment fees or administrator mismatch"
    );
    complete(output, &mut state, "bridgingPayment")?;
    let vft_code = code(&api, vft::WASM_BINARY).await?;
    for (component, name, symbol, decimals) in [
        ("circleVft", "Hoodi Circle", "hUSDC", 6),
        ("tetherVft", "Hoodi Tether", "hUSDT", 6),
        ("bitcoinVft", "Hoodi Bitcoin", "hWBTC", 8),
        ("etherVft", "Hoodi Ether", "hWETH", 18),
        ("gearOriginVft", "Gear Origin Test", "GOT", 12),
    ] {
        let (token, new, salt) = intent(output, &mut state, component, vft::WASM_BINARY)?;
        if new {
            let deployed = vft_client::VftFactory::new(remoting.clone())
                .new(name.into(), symbol.into(), decimals)
                .with_gas_limit(gas)
                .send_recv(vft_code, salt)
                .await
                .map_err(|e| anyhow!("{component} constructor: {e:?}"))?;
            ensure!(
                deployed == token,
                "{component} constructor returned wrong program ID"
            );
        }
        state["programs"][component]["status"] = json!("initializing");
        save(output, &state)?;
        let metadata = vft_client::VftMetadata::new(remoting.clone());
        ensure!(
            metadata
                .name()
                .recv(token)
                .await
                .map_err(|e| anyhow!("{component} name: {e:?}"))?
                == name
                && metadata
                    .symbol()
                    .recv(token)
                    .await
                    .map_err(|e| anyhow!("{component} symbol: {e:?}"))?
                    == symbol
                && metadata
                    .decimals()
                    .recv(token)
                    .await
                    .map_err(|e| anyhow!("{component} decimals: {e:?}"))?
                    == decimals,
            "{component} metadata mismatch"
        );
        let admin = vft_client::VftAdmin::new(remoting.clone());
        ensure!(
            admin
                .minter()
                .recv(token)
                .await
                .map_err(|e| anyhow!("{component} minter: {e:?}"))?
                == initial_state.admin_address
                && admin
                    .burner()
                    .recv(token)
                    .await
                    .map_err(|e| anyhow!("{component} burner: {e:?}"))?
                    == initial_state.admin_address,
            "{component} initial authority mismatch"
        );
        vft_client::allocate_shards(remoting.clone(), token, gas)
            .await
            .map_err(|e| anyhow!("{component} storage initialization: {e:?}"))?;
        complete(output, &mut state, component)?;
    }
    let native_code = code(&api, vft_vara::WASM_BINARY).await?;
    let (native, new, salt) = intent(output, &mut state, "nativeVft", vft_vara::WASM_BINARY)?;
    if new {
        let deployed = vft_vara_client::VftVaraFactory::new(remoting.clone())
            .new(Mainnet::No)
            .with_gas_limit(gas)
            .send_recv(native_code, salt)
            .await
            .map_err(|e| anyhow!("native VFT constructor: {e:?}"))?;
        ensure!(
            deployed == native,
            "native VFT constructor returned wrong program ID"
        );
    }
    state["programs"]["nativeVft"]["status"] = json!("initializing");
    save(output, &state)?;
    let metadata = vft_vara_client::VftMetadata::new(remoting.clone());
    ensure!(
        metadata
            .name()
            .recv(native)
            .await
            .map_err(|e| anyhow!("native VFT name: {e:?}"))?
            == "Wrapped Testnet Vara"
            && metadata
                .symbol()
                .recv(native)
                .await
                .map_err(|e| anyhow!("native VFT symbol: {e:?}"))?
                == "WTVARA"
            && metadata
                .decimals()
                .recv(native)
                .await
                .map_err(|e| anyhow!("native VFT decimals: {e:?}"))?
                == 12,
        "native VFT metadata mismatch"
    );
    let admin = vft_vara_client::VftAdmin::new(remoting.clone());
    ensure!(
        admin
            .minter()
            .recv(native)
            .await
            .map_err(|e| anyhow!("native VFT minter: {e:?}"))?
            == initial_state.admin_address
            && admin
                .burner()
                .recv(native)
                .await
                .map_err(|e| anyhow!("native VFT burner: {e:?}"))?
                == initial_state.admin_address,
        "native VFT initial authority mismatch"
    );
    vft_client::allocate_shards(remoting.clone(), native, gas)
        .await
        .map_err(|e| anyhow!("native VFT storage initialization: {e:?}"))?;
    complete(output, &mut state, "nativeVft")?;
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

fn begin_configuration_action(
    path: &Path,
    key: &str,
    intent: Value,
    retry_safe: bool,
) -> Result<()> {
    ensure_configuration_action(path, key, intent)?;
    let mut state = read_config_stack(path)?;
    let action = &mut state["configuration"]["actions"][key];
    ensure!(
        action["status"] != "verified",
        "verified configuration action {key} no longer matches source state"
    );
    if !retry_safe {
        ensure!(
            action["messages"].as_array().map_or(true, Vec::is_empty),
            "configuration action {key} has an unresolved prior Gear message; refusing to resubmit"
        );
    }
    action["status"] = json!("pending");
    save_config_stack(path, &state)
}

fn prepare_configuration_action(
    path: &Path,
    key: &str,
    intent: Value,
    already_applied: bool,
    retry_safe: bool,
) -> Result<bool> {
    if already_applied {
        ensure_configuration_action(path, key, intent)?;
        Ok(false)
    } else {
        begin_configuration_action(path, key, intent, retry_safe)?;
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
    action["status"] = json!("verified");
    action["readback"] = evidence;
    save_config_stack(path, &state)
}

fn record_configuration_message_start(
    path: &Path,
    action_key: &str,
    target: ActorId,
    payload: &[u8],
    gas_limit: GasUnit,
    value: ValueUnit,
    submission: Value,
) -> Result<()> {
    let mut state = read_config_stack(path)?;
    let action = &mut state["configuration"]["actions"][action_key];
    ensure!(
        action["status"] == "pending",
        "configuration action {action_key} is not pending"
    );
    action["messages"]
        .as_array_mut()
        .ok_or_else(|| anyhow!("configuration action {action_key} message journal is invalid"))?
        .push(json!({
            "status": "submitting",
            "target": format!("0x{}", hex::encode(target.into_bytes())),
            "payload": format!("0x{}", hex::encode(payload)),
            "gasLimit": gas_limit,
            "value": value,
            "submission": submission,
        }));
    save_config_stack(path, &state)
}

fn update_last_configuration_message(path: &Path, action_key: &str, fields: Value) -> Result<()> {
    let mut state = read_config_stack(path)?;
    let messages = state["configuration"]["actions"][action_key]["messages"]
        .as_array_mut()
        .ok_or_else(|| anyhow!("configuration action {action_key} message journal is invalid"))?;
    let last = messages
        .last_mut()
        .ok_or_else(|| anyhow!("configuration action {action_key} has no message intent"))?;
    for (key, value) in fields
        .as_object()
        .ok_or_else(|| anyhow!("invalid configuration message update"))?
    {
        last[key] = value.clone();
    }
    save_config_stack(path, &state)
}

#[derive(Clone)]
struct JournaledRemoting {
    api: GearApi,
    stack_path: PathBuf,
    action_key: String,
}

impl JournaledRemoting {
    fn new(api: GearApi, stack_path: &Path, action_key: &str) -> Self {
        Self {
            api,
            stack_path: stack_path.to_path_buf(),
            action_key: action_key.to_owned(),
        }
    }
}

impl Remoting for JournaledRemoting {
    type Args = GClientArgs;

    async fn activate(
        self,
        code_id: CodeId,
        salt: impl AsRef<[u8]>,
        payload: impl AsRef<[u8]>,
        gas_limit: Option<GasUnit>,
        value: ValueUnit,
        args: Self::Args,
    ) -> SailsResult<impl Future<Output = SailsResult<(ActorId, Vec<u8>)>>> {
        let remoting = GClientRemoting::new(self.api);
        remoting
            .activate(code_id, salt, payload, gas_limit, value, args)
            .await
    }
    async fn message(
        self,
        target: ActorId,
        payload: impl AsRef<[u8]>,
        gas_limit: Option<GasUnit>,
        value: ValueUnit,
        _args: Self::Args,
    ) -> SailsResult<impl Future<Output = SailsResult<Vec<u8>>>> {
        let payload = payload.as_ref();
        let gas_limit = gas_limit.unwrap_or(self.api.block_gas_limit().map_err(config_error)?);
        let mut listener = self.api.subscribe().await.map_err(config_error)?;
        let signer: gsdk::signer::Signer = self.api.clone().into();
        let nonce = signer
            .api()
            .tx()
            .account_nonce(signer.account_id())
            .await
            .map_err(config_error)?;
        let call = gear::tx().gear().send_message(
            target.into(),
            payload.to_vec(),
            gas_limit,
            value,
            false,
        );
        let signed = signer
            .api()
            .tx()
            .create_signed(
                &call,
                signer.signer(),
                PolkadotExtrinsicParamsBuilder::new().nonce(nonce).build(),
            )
            .await
            .map_err(config_error)?;
        record_configuration_message_start(
            &self.stack_path,
            &self.action_key,
            target,
            payload,
            gas_limit,
            value,
            json!({"nonce":nonce,"extrinsicHash":format!("{:#x}",signed.hash()),
                "rawExtrinsic":format!("0x{}",hex::encode(signed.encoded())),
                "sourceGenesis":format!("{:#x}",signer.api().genesis_hash())}),
        )
        .map_err(config_error)?;
        let finalized = signed
            .submit_and_watch()
            .await
            .map_err(config_error)?
            .wait_for_finalized()
            .await
            .map_err(config_error)?;
        let block_hash = finalized.block_hash();
        let events = finalized.wait_for_success().await.map_err(config_error)?;
        let mut queued = None;
        for event in events.iter() {
            if let RuntimeEvent::Gear(GearEvent::MessageQueued {
                id,
                source,
                destination,
                entry: MessageEntry::Handle,
            }) = event
                .map_err(config_error)?
                .as_gear()
                .map_err(config_error)?
            {
                if source.0 == <[u8; 32]>::from(signer.account_id().clone())
                    && destination.as_ref() == target.into_bytes().as_slice()
                {
                    if queued.is_some() {
                        return Err(config_error(
                            "configuration extrinsic queued multiple matching messages",
                        ));
                    }
                    let bytes: [u8; 32] = id.as_ref().try_into().map_err(config_error)?;
                    queued = Some(gear_core::ids::MessageId::from(bytes));
                }
            }
        }
        let message_id = queued.ok_or_else(|| {
            config_error(
                "finalized original configuration extrinsic has no matching queued message",
            )
        })?;
        update_last_configuration_message(
            &self.stack_path,
            &self.action_key,
            json!({
                "status": "submitted",
                "messageId": format!("0x{}", hex::encode(message_id.clone().into_bytes())),
                "enqueueBlockHash": format!("{block_hash:#x}"),
            }),
        )
        .map_err(config_error)?;
        let stack_path = self.stack_path;
        let action_key = self.action_key;
        Ok(async move {
            let (reply_id, result, reply_value) = listener
                .reply_bytes_on(message_id)
                .await
                .map_err(config_error)?;
            match result {
                Ok(reply) => {
                    update_last_configuration_message(
                        &stack_path,
                        &action_key,
                        json!({
                            "status": "reply_received",
                            "replyMessageId": format!("0x{}", hex::encode(reply_id.into_bytes())),
                            "replyValue": reply_value,
                            "replyPayload": format!("0x{}", hex::encode(&reply)),
                        }),
                    )
                    .map_err(config_error)?;
                    Ok(reply)
                }
                Err(error) => {
                    let _ = update_last_configuration_message(
                        &stack_path,
                        &action_key,
                        json!({
                            "status": "reply_error",
                            "replyMessageId": format!("0x{}", hex::encode(reply_id.into_bytes())),
                            "replyValue": reply_value,
                            "replyError": error.clone(),
                        }),
                    );
                    Err(RtlError::ReplyHasErrorString(error).into())
                }
            }
        })
    }

    async fn query(
        self,
        target: ActorId,
        payload: impl AsRef<[u8]>,
        gas_limit: Option<GasUnit>,
        value: ValueUnit,
        args: Self::Args,
    ) -> SailsResult<Vec<u8>> {
        let remoting = GClientRemoting::new(self.api);
        remoting
            .query(target, payload, gas_limit, value, args)
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

async fn inspect_ethereum_configuration(
    ethereum: &crate::ethereum::Ethereum,
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
    let mut registered_managers: Vec<String> =
        vft_managers.iter().map(|id| format!("{id:#x}")).collect();
    registered_managers.sort();
    Ok(json!({
        "manager": format!("{:#x}", ethereum.receiver_address),
        "managerCodeHash": format!("{:#x}", alloy::primitives::keccak256(manager_code.as_ref())),
        "governanceAdmin": format!("{governance_admin:#x}"),
        "governanceAdminCodeHash": format!("{:#x}", alloy::primitives::keccak256(admin_code.as_ref())),
        "governancePauser": format!("{governance_pauser:#x}"),
        "governancePauserCodeHash": format!("{:#x}", alloy::primitives::keccak256(pauser_code.as_ref())),
        "messageQueue": format!("{:#x}", ethereum.queue_address),
        "vftManagers": registered_managers,
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
) -> Result<()> {
    ensure!(
        crate::local_source_rpc(source_rpc)
            && crate::local_source_rpc(witness_rpc)
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
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    ensure!(
        deployment["mode"] == "hoodi-token-stack"
            && deployment["localRehearsal"] == false
            && deployment["ethereum"]["chainId"] == HOODI_CHAIN_ID,
        "tokens-configure only supports a fresh real-Hoodi token deployment"
    );
    let stack_path = fs::canonicalize(token_stack)?;
    let mut stack = read_config_stack(&stack_path)?;
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
        beefy_relay::bridge_domain(source_domain, HOODI_CHAIN_ID, queue.into_array())?
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

    let ethereum_manifest = deployment["ethereum"].clone();
    let ethereum =
        crate::ethereum::Ethereum::connect_hoodi(ethereum_rpc, wallet, &ethereum_manifest).await?;
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
        config.gas_for_token_ops == wanted_config.gas_for_token_ops
            && config.gas_for_reply_deposit == wanted_config.gas_for_reply_deposit
            && config.gas_to_send_request_to_builtin
                == wanted_config.gas_to_send_request_to_builtin
            && config.gas_for_swap_token_maps == wanted_config.gas_for_swap_token_maps
            && config.reply_timeout == wanted_config.reply_timeout
            && config.fee_bridge == wanted_config.fee_bridge
            && config.fee_incoming == wanted_config.fee_incoming,
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
            && payment_state.priority_fee == 2_000_000_000_000,
        "Gear normal/priority payment identity or fee differs from fresh setup"
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
        let admin = vft_client::VftAdmin::new(remoting.clone());
        let minter = admin
            .minter()
            .at_block(finalized_sails_hash)
            .recv(peer)
            .await?;
        let burner = admin
            .burner()
            .at_block(finalized_sails_hash)
            .recv(peer)
            .await?;
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
    let identity = json!({
        "sourceRpc": source_rpc,
        "sourceGenesis": format!("0x{}", hex::encode(source.source_genesis)),
        "bridgeDomain": format!("0x{}", hex::encode(source.bridge_domain)),
        "setupAccount": format!("0x{}", hex::encode(setup.into_bytes())),
        "gearManager": format!("0x{}", hex::encode(manager_id.into_bytes())),
        "gearPrograms": programs,
        "checkpoint": {"id": format!("0x{}", hex::encode(checkpoint_id.into_bytes())), "slot": checkpoint_slot, "hash": format!("0x{}", hex::encode(checkpoint_hash))},
        "gearPayment": {"id": format!("0x{}", hex::encode(payment_id.into_bytes())), "fee": payment_state.fee.to_string(), "priorityFee": payment_state.priority_fee.to_string(), "admin": format!("0x{}", hex::encode(setup.into_bytes()))},
        "ethereum": evm_configuration.clone(),
        "gearTokenPeers": pairs.iter().map(|(spec, peer, address)| json!({
            "component": spec.component,
            "id": format!("0x{}", hex::encode(peer.into_bytes())),
            "erc20": format!("{address:#x}"),
            "supply": if spec.supply == TokenSupply::Gear { "gear" } else { "ethereum" },
        })).collect::<Vec<_>>(),
    });
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
            true,
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
        false,
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
            let current = if role == "minter" {
                admin
                    .minter()
                    .at_block(sails_block_hash(current_at))
                    .recv(*peer)
                    .await?
            } else {
                admin
                    .burner()
                    .at_block(sails_block_hash(current_at))
                    .recv(*peer)
                    .await?
            };
            let should_change = authority_handoff(current, setup, manager_id)?;
            let send_handoff =
                prepare_configuration_action(&stack_path, &key, intent, !should_change, false)?;
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
            let admin = vft_client::VftAdmin::new(remoting.clone());
            let readback = if role == "minter" {
                admin
                    .minter()
                    .at_block(sails_block_hash(at))
                    .recv(*peer)
                    .await?
            } else {
                admin
                    .burner()
                    .at_block(sails_block_hash(at))
                    .recv(*peer)
                    .await?
            };
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
            false,
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
        if prepare_configuration_action(
            &stack_path,
            key,
            native_identity.clone(),
            native_paused,
            false,
        )? {
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
            false,
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
    let native_paused = native_admin
        .is_paused()
        .at_block(sails_block_hash(at))
        .recv(native_id)
        .await?;
    let key = "native.resume";
    if prepare_configuration_action(&stack_path, key, native_identity, !native_paused, false)? {
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
    for (spec, peer, _) in &pairs {
        let admin = vft_client::VftAdmin::new(remoting.clone());
        ensure!(
            admin
                .minter()
                .at_block(sails_block_hash(at))
                .recv(*peer)
                .await?
                == if spec.supply == TokenSupply::Ethereum {
                    manager_id
                } else {
                    setup
                }
                && admin
                    .burner()
                    .at_block(sails_block_hash(at))
                    .recv(*peer)
                    .await?
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
        false,
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
            json!({"finalized": ready_evidence.clone(), "paused": false}),
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

fn ensure_source_inventory_cadence(source: (u64, u64), witness: (u64, u64)) -> Result<()> {
    ensure!(
        source == witness && matches!(source, (64 | 2400, 1500)),
        "source inventory requires matching 3000-ms normal or fast source cadence"
    );
    Ok(())
}

#[cfg(test)]
#[test]
fn source_inventory_cadence_requires_supported_matching_peers() {
    for epoch in [64, 2400] {
        assert!(ensure_source_inventory_cadence((epoch, 1500), (epoch, 1500)).is_ok());
    }
    for (source, witness) in [
        ((64, 1500), (2400, 1500)),
        ((2400, 1500), (64, 1500)),
        ((64, 1499), (64, 1500)),
        ((64, 1500), (64, 1499)),
        ((64, 1499), (64, 1499)),
        ((65, 1500), (65, 1500)),
    ] {
        assert!(ensure_source_inventory_cadence(source, witness).is_err());
    }
}

/// Fund fresh local source fixtures; native inventory must be backed by real value.
#[allow(clippy::too_many_arguments)]
pub async fn provision_source_inventory(
    source_rpc: &str,
    witness_rpc: &str,
    expected_genesis: &str,
    setup_suri: &str,
    campaign_suri: &str,
    deployment_manifest: &Path,
    token_stack: &Path,
) -> Result<()> {
    ensure!(
        crate::local_source_rpc(source_rpc)
            && crate::local_source_rpc(witness_rpc)
            && source_rpc != witness_rpc,
        "source inventory requires independent loopback RPCs"
    );
    let _lock = lock_token_deployment(deployment_manifest)?;
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    let stack_path = fs::canonicalize(token_stack)?;
    ensure!(
        stack_path
            .file_name()
            .is_some_and(|name| name == "token-stack.json"),
        "--token-stack must point to token-stack.json"
    );
    let stack = read_config_stack(&stack_path)?;
    ensure!(
        deployment["mode"] == "hoodi-token-stack"
            && deployment["localRehearsal"] == false
            && deployment["ethereum"]["chainId"] == HOODI_CHAIN_ID
            && stack["lane"] == "beefy-token-hoodi"
            && stack["configuration"]["status"] == "ready",
        "fresh configured Hoodi stack required"
    );
    let (source, witness) = Source::connect_pair(
        gear_rpc_client::GearApi::new(source_rpc, 3).await?,
        gear_rpc_client::GearApi::new(witness_rpc, 3).await?,
    )
    .await?;
    source.validate_attachment(&deployment["anchor"]).await?;
    ensure!(
        source.source_genesis == decode_hash(&json!(expected_genesis), "expected genesis")?
            && stack["sourceGenesis"] == format!("0x{}", hex::encode(source.source_genesis)),
        "source inventory genesis differs"
    );
    ensure_source_identity(
        &deployment["anchor"],
        source.source_genesis,
        source.bridge_domain,
        witness.source_genesis,
        witness.bridge_domain,
    )?;
    let constant = |pallet, name| gsdk::ext::subxt::dynamic::constant(pallet, name);
    let cadence = |chain: &Source| -> Result<(u64, u64)> {
        Ok((
            chain
                .api
                .api
                .constants()
                .at(&constant("Babe", "EpochDuration"))?
                .as_type::<u64>()?,
            chain
                .api
                .api
                .constants()
                .at(&constant("Timestamp", "MinimumPeriod"))?
                .as_type::<u64>()?,
        ))
    };
    ensure_source_inventory_cadence(cadence(&source)?, cadence(&witness)?)?;
    let ed = source
        .api
        .api
        .constants()
        .at(&constant("Balances", "ExistentialDeposit"))?
        .as_type::<u128>()?;
    ensure!(
        ed == 1_000_000_000_000
            && ed
                == witness
                    .api
                    .api
                    .constants()
                    .at(&constant("Balances", "ExistentialDeposit"))?
                    .as_type::<u128>()?,
        "native existential deposit differs"
    );
    let setup_api = GearApi::builder()
        .suri(setup_suri)
        .uri(source_rpc)
        .build()
        .await?;
    let campaign_api = GearApi::builder()
        .suri(campaign_suri)
        .uri(source_rpc)
        .build()
        .await?;
    let setup = ActorId::from(<[u8; 32]>::from(setup_api.account_id().clone()));
    let campaign = ActorId::from(<[u8; 32]>::from(campaign_api.account_id().clone()));
    ensure!(
        setup != campaign,
        "setup and campaign signing roles must differ"
    );
    let manager_id = actor(
        stack["programs"]["vftManager"]["id"]
            .as_str()
            .context("manager ID missing")?,
    )?;
    ensure!(
        deployment["gearManager"] == format!("0x{}", hex::encode(manager_id.into_bytes())),
        "source inventory manager differs from deployment"
    );
    let native_id = actor(
        stack["programs"]["nativeVft"]["id"]
            .as_str()
            .context("native ID missing")?,
    )?;
    let remoting = GClientRemoting::new(setup_api.clone());
    let witness_token = vft_client::Vft::new(GClientRemoting::new(
        GearApi::builder()
            .suri(setup_suri)
            .uri(witness_rpc)
            .build()
            .await?,
    ));
    let manager = vft_manager_client::VftManager::new(remoting.clone());
    let native = vft_vara_client::NativeEscrow::new(remoting.clone());
    let gas_limit = setup_api.block_gas_limit()? * 95 / 100;
    let multiplier = source
        .api
        .api
        .constants()
        .at(&gear::constants().gear_bank().gas_multiplier())?;
    ensure!(
        multiplier
            == witness
                .api
                .api
                .constants()
                .at(&gear::constants().gear_bank().gas_multiplier())?,
        "source gas multiplier differs from witness"
    );
    let gsdk::gear::runtime_types::gear_common::GasMultiplier::ValuePerGas(value_per_gas) =
        multiplier
    else {
        return Err(anyhow!("unsupported source gas multiplier"));
    };
    let gas_reserve = u128::from(gas_limit)
        .checked_mul(value_per_gas)
        .context("gas reserve overflow")?;
    let mut assets = serde_json::Map::new();
    for (component, symbol, amount) in [
        ("gearOriginVft", "GOT", 100u128),
        ("nativeVft", "WTVARA", 2 * ed),
    ] {
        let peer = actor(
            stack["programs"][component]["id"]
                .as_str()
                .context("source token ID missing")?,
        )?;
        let key = format!("source-inventory-{component}");
        let is_native = peer == native_id;
        let token = vft_client::Vft::new(remoting.clone());
        let admin = vft_client::VftAdmin::new(remoting.clone());
        let (at, _, _) = finalized_evidence(&source).await?;
        let mappings = manager
            .vara_to_eth_addresses()
            .at_block(sails_block_hash(at))
            .recv(manager_id)
            .await?;
        ensure!(
            mappings.len() == 6
                && mappings
                    .iter()
                    .any(|(id, _, supply)| *id == peer && *supply == TokenSupply::Gear),
            "source token is not a configured Gear-origin mapping"
        );
        ensure!(
            admin
                .minter()
                .at_block(sails_block_hash(at))
                .recv(peer)
                .await?
                == setup
                && !admin
                    .is_paused()
                    .at_block(sails_block_hash(at))
                    .recv(peer)
                    .await?,
            "source token mint authority or pause differs"
        );
        ensure!(
            native
                .manager()
                .at_block(sails_block_hash(at))
                .recv(native_id)
                .await?
                == Some(manager_id)
                && manager
                    .native_wrapper()
                    .at_block(sails_block_hash(at))
                    .recv(manager_id)
                    .await?
                    == Some(native_id),
            "native wrapper/manager binding differs"
        );
        let current = read_config_stack(&stack_path)?;
        let previous = &current["configuration"]["actions"][&key];
        let expected = sails_rs::prelude::U256::from(amount);
        let balance_before = token
            .balance_of(campaign)
            .at_block(sails_block_hash(at))
            .recv(peer)
            .await?;
        let supply_before = token
            .total_supply()
            .at_block(sails_block_hash(at))
            .recv(peer)
            .await?;
        let reserve_before = if previous.is_null() {
            ensure!(
                balance_before.is_zero() && supply_before.is_zero(),
                "source inventory already exists without its original journal; HOLD"
            );
            if is_native {
                source_free_balance(&source, peer, at).await?
            } else {
                0
            }
        } else {
            previous["intent"]["reserveBefore"]
                .as_str()
                .context("inventory baseline missing")?
                .parse()?
        };
        let intent = json!({"component":component,"program":format!("0x{}",hex::encode(peer.into_bytes())),
            "campaign":format!("0x{}",hex::encode(campaign.into_bytes())),"amountRaw":amount.to_string(),
            "sourceGenesis":format!("0x{}",hex::encode(source.source_genesis)),"native":is_native,
            "reserveBefore":reserve_before.to_string()});
        let funded = balance_before == expected && supply_before == expected;
        ensure!(
            funded || (balance_before.is_zero() && supply_before.is_zero()),
            "source inventory changed before its original mint; HOLD"
        );
        if prepare_configuration_action(&stack_path, &key, intent, funded, false)? {
            if is_native {
                let free = source_free_balance(&source, campaign, at).await?;
                ensure!(
                    free >= amount
                        .checked_add(gas_reserve)
                        .and_then(|value| value.checked_add(ed))
                        .context("native funding requirement overflow")?,
                    "native inventory would consume the campaign gas reserve"
                );
                let mut native_mint = vft_vara_client::VftNativeExchange::new(
                    JournaledRemoting::new(campaign_api.clone(), &stack_path, &key),
                );
                native_mint
                    .mint()
                    .with_value(amount)
                    .with_gas_limit(gas_limit)
                    .send_recv(peer)
                    .await?;
            } else {
                let mut mint = vft_client::VftAdmin::new(JournaledRemoting::new(
                    setup_api.clone(),
                    &stack_path,
                    &key,
                ));
                mint.mint(campaign, sails_rs::prelude::U256::from(amount))
                    .with_gas_limit(gas_limit)
                    .send_recv(peer)
                    .await?;
            }
        }
        let journal = read_config_stack(&stack_path)?;
        let messages = journal["configuration"]["actions"][&key]["messages"]
            .as_array()
            .context("inventory messages missing")?;
        ensure!(messages.len() == 1 && messages[0]["status"] == "reply_received"
            && messages[0]["submission"]["rawExtrinsic"].is_string(),
            "original inventory message/reply is unresolved; preserve its signed transaction and HOLD");
        let reply = hex::decode(
            messages[0]["replyPayload"]
                .as_str()
                .context("inventory reply missing")?
                .trim_start_matches("0x"),
        )?;
        // Sails unit actions acknowledge with no payload, not a service/method envelope.
        ensure!(
            reply.is_empty(),
            "inventory mint acknowledgement contains unexpected bytes"
        );
        let (source_at, source_height, _) = finalized_evidence(&source).await?;
        let (witness_at, witness_height, _) = finalized_evidence(&witness).await?;
        let at = if source_height <= witness_height {
            source_at
        } else {
            witness_at
        };
        let height = source_height.min(witness_height);
        for chain in [&source, &witness] {
            ensure!(
                chain
                    .api
                    .api
                    .legacy()
                    .chain_get_block_hash(Some(height.into()))
                    .await?
                    == Some(at),
                "source inventory finality differs from witness"
            );
        }
        let balance = token
            .balance_of(campaign)
            .at_block(sails_block_hash(at))
            .recv(peer)
            .await?;
        let supply = token
            .total_supply()
            .at_block(sails_block_hash(at))
            .recv(peer)
            .await?;
        ensure!(
            balance == expected
                && supply == expected
                && witness_token
                    .balance_of(campaign)
                    .at_block(sails_block_hash(at))
                    .recv(peer)
                    .await?
                    == expected
                && witness_token
                    .total_supply()
                    .at_block(sails_block_hash(at))
                    .recv(peer)
                    .await?
                    == expected,
            "source inventory balance/supply is not exact at common finality; HOLD"
        );
        let reserve_after = if is_native {
            source_free_balance(&source, peer, at).await?
        } else {
            0
        };
        if is_native {
            ensure!(
                reserve_after == source_free_balance(&witness, peer, at).await?,
                "source reserve differs from witness"
            );
            ensure!(
                reserve_after
                    >= reserve_before
                        .checked_add(amount)
                        .context("native reserve overflow")?
                    && source_free_balance(&source, campaign, at).await? >= ed,
                "native inventory is not backed or campaign gas reserve is exhausted"
            );
        }
        let evidence = json!({"finalized":{"height":height,"hash":format!("{at:#x}")},
            "balanceRaw":balance.to_string(),"totalSupplyRaw":supply.to_string(),
            "nativeReserveRaw":reserve_after.to_string(),"originalMessageId":messages[0]["messageId"]});
        mark_configuration_action_verified(&stack_path, &key, evidence.clone())?;
        assets.insert(
            symbol.to_owned(),
            json!({"program":format!("0x{}",hex::encode(peer.into_bytes())),
            "amountRaw":amount.to_string(),"evidence":evidence}),
        );
    }
    let mut final_stack = read_config_stack(&stack_path)?;
    final_stack["sourceInventory"] = json!({"status":"ready","testOnly":true,
        "campaign":format!("0x{}",hex::encode(campaign.into_bytes())),"assets":assets});
    save_config_stack(&stack_path, &final_stack)?;
    println!(
        "Fresh source inventory verified: GOT 100 raw; WTVARA {} raw backed by native value",
        2 * ed
    );
    Ok(())
}

async fn source_free_balance(source: &Source, account: ActorId, at: GearHash) -> Result<u128> {
    let address = gear::storage()
        .system()
        .account(gsdk::ext::subxt::utils::AccountId32::from(
            account.into_bytes(),
        ));
    Ok(source
        .api
        .api
        .storage()
        .at(at)
        .fetch(&address)
        .await?
        .map_or(0, |info| info.data.free))
}

/// Publish a finalized token registration root and persist enough evidence to reconcile retries.
fn state_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub(crate) fn lock_token_deployment(manifest_path: &Path) -> Result<File> {
    let manifest_path = fs::canonicalize(manifest_path)?;
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let chain_id = manifest["ethereum"]["chainId"]
        .as_u64()
        .ok_or_else(|| anyhow!("deployment chain id is missing"))?;
    let queue: alloy::primitives::Address = manifest["ethereum"]["queue"]
        .as_str()
        .ok_or_else(|| anyhow!("deployment queue is missing"))?
        .parse()?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state_directory(&manifest_path).join(format!("beefy-{chain_id}-{queue:x}.lock")))?;
    lock.try_lock().map_err(|error| {
        anyhow!("another BEEFY actor or maintenance publisher owns this deployment: {error}")
    })?;
    Ok(lock)
}

fn persist_root_intent(path: &Path, intent: &Value) -> Result<()> {
    let parent = state_directory(path);
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, intent)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn decode_hash(value: &Value, field: &str) -> Result<[u8; 32]> {
    let text = value
        .as_str()
        .ok_or_else(|| anyhow!("{field} is missing"))?;
    hex::decode(text.strip_prefix("0x").unwrap_or(text))?
        .try_into()
        .map_err(|_| anyhow!("{field} must be 32 bytes"))
}

fn ensure_source_identity(
    manifest: &Value,
    source_genesis: [u8; 32],
    bridge_domain: [u8; 32],
    witness_genesis: [u8; 32],
    witness_domain: [u8; 32],
) -> Result<()> {
    ensure!(
        source_genesis == witness_genesis
            && bridge_domain == witness_domain
            && manifest["sourceGenesis"] == format!("0x{}", hex::encode(source_genesis))
            && manifest["bridgeDomain"] == format!("0x{}", hex::encode(bridge_domain)),
        "source genesis or destination-bound bridge domain differs between authorities and manifest"
    );
    Ok(())
}

fn ensure_recaptured_anchor(
    registration_block: u32,
    (recorded_block, recorded_hash, recorded_raw): (u32, [u8; 32], &str),
    (source_block, source_hash, source_raw): (u32, [u8; 32], &[u8]),
    (witness_block, witness_hash, witness_raw): (u32, [u8; 32], &[u8]),
    supplied_raw: &[u8],
) -> Result<()> {
    ensure!(
        source_block == recorded_block
            && witness_block == recorded_block
            && recorded_block > registration_block,
        "accepted BEEFY anchor does not cover the registered queue root"
    );
    ensure!(
        recorded_hash == source_hash && source_hash == witness_hash,
        "accepted source anchor was reorged or differs between authorities"
    );
    let encoded_raw = format!("0x{}", hex::encode(supplied_raw));
    ensure!(
        recorded_raw == encoded_raw && source_raw == supplied_raw && witness_raw == supplied_raw,
        "recaptured signed commitment differs from publisher journal"
    );
    Ok(())
}

fn ensure_separate_evm_signers(
    follower: alloy::primitives::Address,
    publisher: alloy::primitives::Address,
) -> Result<()> {
    ensure!(
        follower != publisher,
        "root publisher and follower must use separate EVM signer accounts"
    );
    Ok(())
}

fn saved_publication_nonce(intent: &Value) -> Result<u64> {
    intent["nonce"]
        .as_str()
        .ok_or_else(|| anyhow!("saved root publication nonce is invalid"))?
        .parse::<u64>()
        .map_err(|_| anyhow!("saved root publication nonce is invalid"))
}

fn ensure_saved_nonce_available(nonce: u64, latest: u64, pending: u64) -> Result<()> {
    ensure!(
        latest == nonce && pending == nonce,
        "root publication nonce is uncertain; refusing a replacement nonce"
    );
    Ok(())
}

fn ensure_immutable_publication(saved: &Value, expected: &Value) -> Result<()> {
    for field in [
        "schemaVersion",
        "sourceBlock",
        "root",
        "kind",
        "localRehearsal",
        "sourceIdentity",
        "acceptedAnchorTx",
        "acceptedAnchorClient",
        "acceptedCheckpoint",
        "sender",
        "proof",
        "queueProof",
    ] {
        ensure!(
            saved[field] == expected[field],
            "saved root publication {field} differs from independently recaptured evidence; hold original intent"
        );
    }
    Ok(())
}

async fn root_account_nonces(
    ethereum: &crate::ethereum::Ethereum,
    address: alloy::primitives::Address,
) -> Result<(u64, u64)> {
    use alloy::providers::Provider;

    let provider = ethereum.api.raw_provider();
    let latest = provider.get_transaction_count(address).await?;
    let pending = provider.get_transaction_count(address).pending().await?;
    Ok((latest, pending))
}

async fn finalized_ethereum_head(ethereum: &crate::ethereum::Ethereum) -> Result<(u64, B256)> {
    let head = ethereum.api.verified_finalized_view().await?;
    Ok((head.block_number(), head.block_hash()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReceiptFinality {
    Mined,
    Accepted,
    Reverted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootPublicationState {
    Pending,
    Mined,
    Accepted,
}

fn classify_receipt_finality(
    included_block: u64,
    included_hash: B256,
    succeeded: bool,
    finalized_block: u64,
    canonical_hash: Option<B256>,
) -> Result<ReceiptFinality> {
    ensure!(
        canonical_hash == Some(included_hash),
        "root transaction inclusion is no longer canonical; hold original signed intent"
    );
    Ok(if !succeeded {
        ReceiptFinality::Reverted
    } else if included_block > finalized_block {
        ReceiptFinality::Mined
    } else {
        ReceiptFinality::Accepted
    })
}

fn signed_publication_identity(
    intent: &Value,
    queue: Address,
) -> Result<ethereum_client::TransactionIdentity> {
    let hash = B256::from(decode_hash(
        &intent["txHash"],
        "saved root transaction hash",
    )?);
    let raw = hex::decode(
        intent["rawTransaction"]
            .as_str()
            .context("saved root transaction bytes missing")?
            .trim_start_matches("0x"),
    )?;
    let identity = crate::ethereum::signed_transaction_identity(
        &raw,
        hash,
        saved_publication_nonce(intent)?,
        queue,
    )?;
    let sender: Address = intent["sender"]
        .as_str()
        .context("saved root publisher missing")?
        .parse()?;
    ensure!(
        identity.from == sender,
        "saved root signature differs from original publisher"
    );
    Ok(identity)
}

fn persist_root_inclusion(
    path: &Path,
    intent: &mut Value,
    (block, hash): (u64, B256),
    kind: &str,
    finalized: Option<(u64, B256)>,
) -> Result<()> {
    ensure!(
        intent["publicationReceipt"]["finalized"] != true || finalized.is_some(),
        "previously finalized root lost finality; hold original signed intent"
    );
    if !intent["publicationReceipt"].is_null() {
        ensure!(
            intent["publicationReceipt"]["block"].as_u64() == Some(block)
                && intent["publicationReceipt"]["blockHash"].as_str()
                    == Some(format!("{hash:#x}").as_str())
                && intent["publicationReceipt"]["kind"].as_str() == Some(kind),
            "original root inclusion/kind changed; hold original signed intent"
        );
        if intent["publicationReceipt"]["finalized"] == true {
            return Ok(());
        }
    }
    let mut receipt = json!({
        "block": block, "blockHash": format!("{hash:#x}"),
        "finalized": finalized.is_some(), "kind": kind,
    });
    if let Some((finalized_block, finalized_hash)) = finalized {
        receipt["finalizedBlock"] = json!(finalized_block);
        receipt["finalizedBlockHash"] = json!(format!("{finalized_hash:#x}"));
    }
    intent["status"] = json!(if finalized.is_some() {
        "accepted"
    } else {
        "mined"
    });
    intent["finalityStatus"] = json!(if finalized.is_some() {
        "finalized"
    } else {
        "mined"
    });
    intent["publicationReceipt"] = receipt;
    persist_root_intent(path, intent)
}

fn ensure_pinned_latest_anchor(
    checkpoint: &crate::ethereum::DestinationCheckpoint,
    anchor_block: u32,
    anchor_root: [u8; 32],
) -> Result<()> {
    ensure!(
        checkpoint.block == u64::from(anchor_block) && checkpoint.root == anchor_root,
        "pinned BEEFY anchor is no longer latest; hold the original root intent"
    );
    Ok(())
}

async fn canonical_root_inclusion(
    ethereum: &crate::ethereum::Ethereum,
    intent: &Value,
    source_block: u32,
    kind: &str,
    expected_root: [u8; 32],
) -> Result<Option<(u64, B256)>> {
    use alloy::{
        providers::Provider,
        rpc::types::{BlockId, BlockNumberOrTag},
    };

    let signed = signed_publication_identity(intent, ethereum.queue_address)?;
    let hash = signed.hash;
    let provider = ethereum.api.raw_provider();
    let Some(receipt) = provider.get_transaction_receipt(hash).await? else {
        ensure!(
            intent["publicationReceipt"].is_null(),
            "previously mined root receipt disappeared; hold original signed intent"
        );
        return Ok(None);
    };
    let block = receipt
        .block_number
        .context("root receipt has no block number")?;
    let block_hash = receipt
        .block_hash
        .context("root receipt has no block hash")?;
    ensure!(
        receipt.transaction_hash == hash && receipt.from == signed.from && receipt.to == signed.to,
        "root receipt identity changed"
    );
    let canonical = provider
        .get_block_by_number(BlockNumberOrTag::Number(block))
        .await?
        .context("mined root receipt block is unavailable")?;
    let finality = classify_receipt_finality(
        block,
        block_hash,
        receipt.status(),
        0,
        Some(canonical.header.hash),
    )?;
    ensure!(
        finality != ReceiptFinality::Reverted,
        "original root transaction reverted in a canonical block"
    );
    let recorded = &intent["publicationReceipt"];
    ensure!(
        recorded.is_null()
            || (recorded["block"] == block
                && recorded["blockHash"] == format!("{block_hash:#x}")
                && recorded["kind"] == kind),
        "saved root receipt moved or changed; hold original signed intent"
    );
    let event = if kind == "emptyProgress" {
        B256::from(beefy_relay::keccak256(b"EmptyQueueProgress(uint256)"))
    } else {
        B256::from(beefy_relay::keccak256(b"MerkleRoot(uint256,bytes32)"))
    };
    let block_bytes = U256::from(source_block).to_be_bytes::<32>();
    ensure!(
        receipt.as_ref().logs().iter().any(|log| {
            let topics = log.topics();
            let data = log.data().data.as_ref();
            log.address() == ethereum.queue_address
                && topics.first() == Some(&event)
                && if kind == "emptyProgress" {
                    topics.len() == 2 && topics[1] == B256::from(block_bytes) && data.is_empty()
                } else {
                    topics.len() == 1
                        && data.len() == 64
                        && data[..32] == block_bytes
                        && data[32..] == expected_root
                }
        }),
        "canonical root receipt does not record the expected queue progress"
    );
    let queue = ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, provider.clone());
    let at = BlockId::hash(block_hash);
    let stored = queue
        .getMerkleRoot(U256::from(source_block))
        .block(at)
        .call()
        .await?;
    ensure!(
        stored == B256::from(expected_root)
            && (kind != "emptyProgress"
                || queue.maxBlockNumber().block(at).call().await? >= U256::from(source_block)),
        "canonical root receipt does not store the expected queue progress"
    );
    Ok(Some((block, block_hash)))
}

// Called under the deployment lock before advancing to another BEEFY commitment.
// An unsigned or unmined saved intent holds; a missing intent can wait for a covering anchor.
pub(crate) async fn ensure_mined_root_publications(
    ethereum: &crate::ethereum::Ethereum,
    roots: &Value,
) -> Result<()> {
    let roots = roots
        .as_object()
        .context("root registration journal is missing")?;
    for root in roots.values() {
        let status = root["status"].as_str().context("root status missing")?;
        if status == "accepted" {
            continue;
        }
        ensure!(matches!(status, "pending" | "mined"), "unknown root status");
        let path = Path::new(
            root["publication"]
                .as_str()
                .context("root publication path missing")?,
        );
        if !path.exists() && status == "pending" {
            continue;
        }
        let intent: Value = serde_json::from_slice(&fs::read(path)?)?;
        let block: u32 = root["block"]
            .as_u64()
            .context("root source block missing")?
            .try_into()?;
        let kind = root["kind"].as_str().unwrap_or("merkleRoot");
        let expected_root = decode_hash(&root["queueRoot"], "root registration")?;
        let saved_client: Address = intent["acceptedAnchorClient"]
            .as_str()
            .context("pinned BEEFY client is missing")?
            .parse()?;
        let saved_queue: Address = intent["sourceIdentity"]["destinationQueue"]
            .as_str()
            .context("pinned destination queue is missing")?
            .parse()?;
        ensure!(
            intent["schemaVersion"] == 3
                && intent["sourceBlock"] == block
                && intent["root"] == root["queueRoot"]
                && intent["kind"] == kind
                && saved_client == ethereum.client_address
                && saved_queue == ethereum.queue_address,
            "saved root publication differs from registration or active deployment"
        );
        ensure!(
            !intent["txHash"].is_null() && !intent["publicationReceipt"].is_null(),
            "root publication has not been canonically mined; hold next handover"
        );
        ensure!(
            canonical_root_inclusion(ethereum, &intent, block, kind, expected_root)
                .await?
                .is_some(),
            "root publication receipt is missing; hold next handover"
        );
    }
    Ok(())
}

fn hold_root_reconciliation(output: &Path, intent: &mut Value) -> Result<RootPublicationState> {
    let state = if intent["publicationReceipt"].is_null() {
        intent["status"] = json!("held: original signed root remains unmined");
        intent["finalityStatus"] = json!("pending");
        RootPublicationState::Pending
    } else {
        RootPublicationState::Mined
    };
    intent["lastError"] =
        json!("reconciliation held; retain original signed hash, nonce, anchor and inclusion");
    persist_root_intent(output, intent)?;
    Ok(state)
}

async fn wait_for_root_receipt(
    ethereum: &crate::ethereum::Ethereum,
    output: &Path,
    intent: &mut Value,
    source_block: u32,
    kind: &str,
    expected_root: [u8; 32],
    anchor_block: u32,
    anchor_root: [u8; 32],
    anchor_tx: B256,
    anchor_inclusion: (u64, B256),
) -> Result<RootPublicationState> {
    use alloy::providers::Provider;
    use std::time::{Duration, Instant};

    let signed = signed_publication_identity(intent, ethereum.queue_address)?;
    let transaction_hash = signed.hash;
    let provider = ethereum.api.raw_provider();
    let deadline = Instant::now() + Duration::from_secs(30);
    let observation = tokio::time::timeout_at(deadline.into(), async {
        loop {
            if let Some((block, hash)) =
                canonical_root_inclusion(ethereum, intent, source_block, kind, expected_root)
                    .await?
            {
                if intent["publicationReceipt"]["finalized"] != true {
                    persist_root_inclusion(output, intent, (block, hash), kind, None)?;
                }
                let (finalized_block, _) = finalized_ethereum_head(ethereum).await?;
                if classify_receipt_finality(block, hash, true, finalized_block, Some(hash))?
                    == ReceiptFinality::Mined
                {
                    persist_root_inclusion(output, intent, (block, hash), kind, None)?;
                    return Ok(RootPublicationState::Mined);
                }
                if let Some(receipt) = ethereum.api.get_finalized_receipt(transaction_hash).await? {
                    ensure!(
                        receipt.receipt.transaction_hash == transaction_hash
                            && receipt.receipt.from == signed.from
                            && receipt.receipt.to == signed.to
                            && receipt.receipt.status()
                            && receipt.included_block_number == block
                            && receipt.included_block_hash == hash
                            && receipt.receipt.block_number == Some(block)
                            && receipt.receipt.block_hash == Some(hash)
                            && receipt.finalized_block_number >= block,
                        "finalized root receipt differs from original canonical inclusion"
                    );
                    if let Some(anchor) = ethereum.api.get_finalized_receipt(anchor_tx).await? {
                        ensure!(
                            anchor.receipt.transaction_hash == anchor_tx
                                && anchor.receipt.status()
                                && anchor.receipt.block_number == Some(anchor_inclusion.0)
                                && anchor.receipt.block_hash == Some(anchor_inclusion.1)
                                && anchor.included_block_number == anchor_inclusion.0
                                && anchor.included_block_hash == anchor_inclusion.1
                                && anchor.finalized_block_number >= anchor_inclusion.0,
                            "finalized BEEFY anchor differs from pinned canonical inclusion"
                        );
                        ensure!(
                            provider
                                .get_block_by_number(alloy::rpc::types::BlockNumberOrTag::Number(
                                    anchor_inclusion.0
                                ))
                                .await?
                                .is_some_and(
                                    |canonical| canonical.header.hash == anchor_inclusion.1
                                ),
                            "pinned BEEFY anchor receipt block is no longer canonical"
                        );
                        if intent["publicationReceipt"]["finalized"] == true {
                            let saved_block = intent["publicationReceipt"]["finalizedBlock"]
                                .as_u64()
                                .context("original root finalized anchor height missing")?;
                            let saved_hash: B256 = intent["publicationReceipt"]
                                ["finalizedBlockHash"]
                                .as_str()
                                .context("original root finalized anchor hash missing")?
                                .parse()?;
                            ensure!(
                                ethereum
                                    .api
                                    .is_finalized_block(saved_block, saved_hash.0.into())
                                    .await?,
                                "original root finalized anchor changed; HOLD"
                            );
                        }
                        persist_root_inclusion(
                            output,
                            intent,
                            (block, hash),
                            kind,
                            Some((receipt.finalized_block_number, receipt.finalized_block_hash)),
                        )?;
                        return Ok(RootPublicationState::Accepted);
                    }
                }
                return Ok(RootPublicationState::Mined);
            }
            ensure_pinned_latest_anchor(&ethereum.checkpoint().await?, anchor_block, anchor_root)?;
            intent["finalityStatus"] = json!("pending");
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
    .await;
    match observation {
        Ok(Ok(state)) => Ok(state),
        Ok(Err(error))
            if !matches!(
                error.downcast_ref::<ethereum_client::Error>(),
                Some(ethereum_client::Error::FinalizedAncestryPending)
            ) =>
        {
            Err(error)
        }
        _ => hold_root_reconciliation(output, intent),
    }
}

/// Require the operator-pinned recovery Safe for any fresh deployment or recovery operation.
pub(crate) fn require_recovery_wallet_identity(expected: &str) -> Result<()> {
    let expected: Address = expected
        .parse()
        .map_err(|_| anyhow!("deployment recovery wallet identity is invalid"))?;
    let configured = std::env::var("BEEFY_RECOVERY_WALLET")
        .map_err(|_| anyhow!("BEEFY_RECOVERY_WALLET is required"))?;
    let configured: Address = configured
        .parse()
        .map_err(|_| anyhow!("BEEFY_RECOVERY_WALLET must be a valid address"))?;
    ensure!(
        expected != Address::ZERO && configured == expected,
        "BEEFY_RECOVERY_WALLET differs from the immutable deployment recovery wallet"
    );
    Ok(())
}

fn recovery_address(plan: &Value, field: &str) -> Result<Address> {
    plan[field]
        .as_str()
        .ok_or_else(|| anyhow!("recovery plan {field} is missing"))?
        .parse()
        .map_err(Into::into)
}

fn recovery_address_topic(address: Address) -> B256 {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address.as_slice());
    B256::from(word)
}

fn recovery_number_topic(value: u64) -> B256 {
    B256::from(U256::from(value).to_be_bytes::<32>())
}

async fn recovery_code_hash(
    ethereum: &crate::ethereum::Ethereum,
    address: Address,
    finalized_hash: B256,
) -> Result<B256> {
    use alloy::eips::BlockId;
    let code = ethereum
        .api
        .raw_provider()
        .get_code_at(address)
        .block_id(BlockId::hash(finalized_hash))
        .await?;
    ensure!(
        !code.is_empty(),
        "pinned recovery identity has no finalized runtime code"
    );
    Ok(B256::from(alloy::primitives::keccak256(code.as_ref())))
}

/// Verify an inactive recovery candidate without changing the immutable active deployment.
pub(crate) async fn verify_recovery_candidate(
    ethereum_rpc: &str,
    deployment: &Value,
    ethereum_manifest: &Value,
    plan: &Value,
) -> Result<Value> {
    use crate::ethereum::Ethereum;
    use alloy::{eips::BlockId, primitives::B256, providers::Provider};

    ensure!(
        deployment["mode"] == "hoodi-token-stack"
            && deployment["localRehearsal"] == false
            && deployment["ethereum"]["chainId"] == HOODI_CHAIN_ID,
        "candidate recovery requires the pinned real-Hoodi token deployment"
    );
    ensure!(
        plan["schemaVersion"] == 1,
        "unsupported recovery plan version"
    );
    let queue_address: Address = ethereum_manifest["queue"]
        .as_str()
        .context("deployment queue missing")?
        .parse()?;
    let old_verifier: Address = ethereum_manifest["verifier"]
        .as_str()
        .context("deployment verifier missing")?
        .parse()?;
    let old_client: Address = ethereum_manifest["client"]
        .as_str()
        .context("deployment client missing")?
        .parse()?;
    let controller = recovery_address(plan, "controller")?;
    let recovery_wallet = recovery_address(plan, "recoveryWallet")?;
    let expected_old = recovery_address(plan, "expectedOldVerifier")?;
    let expected_old_client = recovery_address(plan, "expectedOldClient")?;
    let candidate_verifier = recovery_address(plan, "candidateVerifier")?;
    let candidate_client = recovery_address(plan, "candidateClient")?;
    let manifest_controller: Address = ethereum_manifest["recoveryController"]
        .as_str()
        .context("deployment recovery controller missing")?
        .parse()?;
    let manifest_wallet: Address = ethereum_manifest["recoveryWallet"]
        .as_str()
        .context("deployment recovery wallet missing")?
        .parse()?;
    let source_domain = B256::from(decode_hash(
        &ethereum_manifest["sourceDomain"],
        "sourceDomain",
    )?);
    let bridge_domain = B256::from(decode_hash(
        &ethereum_manifest["bridgeDomain"],
        "bridgeDomain",
    )?);
    let mmr_start = number(&ethereum_manifest["mmrStartBlock"])?;
    ensure!(
        expected_old == old_verifier
            && expected_old_client == old_client
            && controller == manifest_controller
            && recovery_wallet == manifest_wallet,
        "recovery plan verifier/client/controller/wallet differs from immutable deployment"
    );
    require_recovery_wallet_identity(&format!("{manifest_wallet:#x}"))?;
    ensure!(
        plan["queue"]
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(&format!("{queue_address:#x}")))
            && number(&plan["destinationChainId"])? == HOODI_CHAIN_ID
            && number(&plan["mmrStartBlock"])? == mmr_start
            && B256::from(decode_hash(
                &plan["sourceDomain"],
                "recovery plan sourceDomain"
            )?) == source_domain
            && B256::from(decode_hash(
                &plan["bridgeDomain"],
                "recovery plan bridgeDomain"
            )?) == bridge_domain,
        "recovery plan queue/source/destination/MMR identity differs from immutable deployment"
    );

    let ethereum = Ethereum::connect_hoodi_readonly(ethereum_rpc, ethereum_manifest).await?;
    let (finalized_number, finalized_hash) = finalized_ethereum_head(&ethereum).await?;
    let provider = ethereum.api.raw_provider().clone();
    let at = BlockId::hash(finalized_hash);
    let queue = RecoveryQueueBinding::new(queue_address, provider.clone());
    let controller_contract = RecoveryControllerBinding::new(controller, provider.clone());
    ensure!(queue.verifier().block(at).call().await? == old_verifier
        && queue.recoveryController().block(at).call().await? == controller
        && controller_contract.messageQueue().block(at).call().await? == queue_address
        && controller_contract.recoveryWallet().block(at).call().await? == recovery_wallet
        && controller_contract.RECOVERY_DELAY().block(at).call().await? == U256::from(24 * 60 * 60u64),
        "finalized queue/controller/wallet binding or 24-hour recovery delay differs from deployment");

    let old_root = RecoveryVerifierBinding::new(old_verifier, provider.clone());
    let candidate_root = RecoveryVerifierBinding::new(candidate_verifier, provider.clone());
    ensure!(
        old_root.beefyClient().block(at).call().await? == old_client
            && candidate_root.beefyClient().block(at).call().await? == candidate_client
            && old_root.messageQueue().block(at).call().await? == queue_address
            && candidate_root.messageQueue().block(at).call().await? == queue_address
            && old_root.destinationChainId().block(at).call().await? == U256::from(HOODI_CHAIN_ID)
            && candidate_root.destinationChainId().block(at).call().await?
                == U256::from(HOODI_CHAIN_ID),
        "old/candidate verifier code bindings differ from the pinned Hoodi lane"
    );
    let old = RecoveryClientBinding::new(old_client, provider.clone());
    let candidate = RecoveryClientBinding::new(candidate_client, provider.clone());
    let old_block = old.latestBeefyBlock().block(at).call().await?;
    let candidate_block = candidate.latestBeefyBlock().block(at).call().await?;
    let candidate_root_hash = candidate.latestMMRRoot().block(at).call().await?;
    let old_live = old.isLive().block(at).call().await?;
    let candidate_live = candidate.isLive().block(at).call().await?;
    ensure!(
        !old_live && candidate_live,
        "recovery requires an expired old client and a live candidate"
    );
    let candidate_ready = candidate_root_hash != B256::ZERO && candidate_block > old_block;
    for client in [&old, &candidate] {
        ensure!(
            client.sourceDomain().block(at).call().await? == source_domain
                && client.bridgeDomain().block(at).call().await? == bridge_domain
                && client.destinationChainId().block(at).call().await?
                    == U256::from(HOODI_CHAIN_ID)
                && client.destinationQueue().block(at).call().await? == queue_address
                && client.mmrStartBlock().block(at).call().await? == mmr_start,
            "old/candidate client source identity or MMR start differs from immutable deployment"
        );
    }

    let code_hashes = &plan["codeHashes"];
    let old_verifier_hash = recovery_code_hash(&ethereum, old_verifier, finalized_hash).await?;
    let old_client_hash = recovery_code_hash(&ethereum, old_client, finalized_hash).await?;
    ensure!(
        old_verifier_hash
            == B256::from(decode_hash(
                &code_hashes["oldVerifier"],
                "codeHashes.oldVerifier"
            )?)
            && old_client_hash
                == B256::from(decode_hash(
                    &code_hashes["oldClient"],
                    "codeHashes.oldClient"
                )?)
            && old_verifier_hash
                == B256::from(decode_hash(
                    &ethereum_manifest["bytecodeHashes"]["verifier"],
                    "deployment bytecodeHashes.verifier"
                )?)
            && old_client_hash
                == B256::from(decode_hash(
                    &ethereum_manifest["bytecodeHashes"]["client"],
                    "deployment bytecodeHashes.client"
                )?),
        "old client/verifier runtime hashes differ from the immutable deployment or recovery plan"
    );
    let candidate_verifier_hash =
        recovery_code_hash(&ethereum, candidate_verifier, finalized_hash).await?;
    let candidate_client_hash =
        recovery_code_hash(&ethereum, candidate_client, finalized_hash).await?;
    ensure!(
        candidate_verifier_hash
            == B256::from(decode_hash(
                &code_hashes["candidateVerifier"],
                "codeHashes.candidateVerifier"
            )?)
            && candidate_client_hash
                == B256::from(decode_hash(
                    &code_hashes["candidateClient"],
                    "codeHashes.candidateClient"
                )?),
        "candidate verifier/client runtime code hashes differ from the operator pins"
    );
    for address in [controller, recovery_wallet] {
        let code = provider
            .get_code_at(address)
            .block_id(BlockId::hash(finalized_hash))
            .await?;
        ensure!(
            !code.is_empty(),
            "pinned recovery controller/Safe has no finalized runtime code"
        );
    }

    let pending = controller_contract
        .pendingRecovery()
        .block(at)
        .call()
        .await?;
    let plan_has_proposal = !plan["proposalId"].is_null()
        || !plan["proposalNonce"].is_null()
        || !plan["executeAfter"].is_null();
    let plan_has_complete_proposal = !plan["proposalId"].is_null()
        && !plan["proposalNonce"].is_null()
        && !plan["executeAfter"].is_null();
    ensure!(
        !plan_has_proposal || plan_has_complete_proposal,
        "recovery plan proposal identity is only partially pinned"
    );
    ensure!(
        !plan_has_proposal || pending.exists,
        "pinned recovery proposal is not pending on the finalized chain"
    );
    let proposal = if pending.exists {
        ensure!(
            candidate_ready,
            "recovery proposal requires a finalized candidate MMR update ahead of the old client"
        );
        let onchain_nonce = controller_contract.proposalNonce().block(at).call().await?;
        let proposal_id = if plan_has_complete_proposal {
            number(&plan["proposalId"])?
        } else {
            pending
                .proposalId
                .try_into()
                .context("recovery proposal id exceeds supported range")?
        };
        let proposal_nonce = if plan_has_complete_proposal {
            number(&plan["proposalNonce"])?
        } else {
            onchain_nonce
                .try_into()
                .context("recovery proposal nonce exceeds supported range")?
        };
        let execute_after = if plan_has_complete_proposal {
            U256::from(number(&plan["executeAfter"])?)
        } else {
            pending.executeAfter
        };
        ensure!(
            proposal_id == proposal_nonce
                && pending.proposalId == U256::from(proposal_id)
                && pending.expectedOldVerifier == old_verifier
                && pending.candidateVerifier == candidate_verifier
                && pending.executeAfter == execute_after
                && pending.expectedOldVerifierCodeHash == old_verifier_hash
                && pending.expectedOldClient == old_client
                && pending.expectedOldClientCodeHash == old_client_hash
                && pending.candidateVerifierCodeHash == candidate_verifier_hash
                && pending.candidateClient == candidate_client
                && pending.candidateClientCodeHash == candidate_client_hash
                && onchain_nonce == U256::from(proposal_nonce),
            "finalized pending recovery tuple differs from the pinned plan"
        );
        json!({"proposalId":proposal_id,"proposalNonce":proposal_nonce,
            "executeAfter":execute_after.to_string()})
    } else {
        ensure!(
            pending.proposalId == U256::ZERO,
            "finalized recovery controller reports inconsistent pending proposal fields"
        );
        Value::Null
    };

    let mut candidate_manifest = ethereum_manifest.clone();
    candidate_manifest["verifier"] = json!(format!("{candidate_verifier:#x}"));
    candidate_manifest["client"] = json!(format!("{candidate_client:#x}"));
    candidate_manifest["bindings"]["beefyClient"] = json!(format!("{candidate_client:#x}"));
    candidate_manifest["bytecodeHashes"]["verifier"] =
        json!(format!("{candidate_verifier_hash:#x}"));
    candidate_manifest["bytecodeHashes"]["client"] = json!(format!("{candidate_client_hash:#x}"));
    Ok(json!({
        "activeEthereum": candidate_manifest,
        "candidateVerifier": format!("{candidate_verifier:#x}"),
        "candidateClient": format!("{candidate_client:#x}"),
        "candidateReady": candidate_ready,
        "candidateBlock": candidate_block,
        "candidateRoot": format!("{candidate_root_hash:#x}"),
        "finalizedBlock": finalized_number,
        "finalizedHash": format!("{finalized_hash:#x}"),
        "pendingProposal": proposal,
    }))
}

pub(crate) async fn verify_recovery_transition(
    ethereum_rpc: &str,
    deployment: &Value,
    ethereum_manifest: &Value,
    plan: &Value,
) -> Result<Value> {
    use crate::ethereum::Ethereum;
    use alloy::{
        eips::BlockId, primitives::B256, providers::Provider, rpc::types::BlockNumberOrTag,
    };

    ensure!(
        deployment["mode"] == "hoodi-token-stack"
            && deployment["localRehearsal"] == false
            && deployment["ethereum"]["chainId"] == HOODI_CHAIN_ID,
        "recovery activation requires the pinned real-Hoodi token deployment"
    );
    ensure!(
        plan["schemaVersion"] == 1,
        "unsupported recovery plan version"
    );
    let queue_address: Address = ethereum_manifest["queue"]
        .as_str()
        .context("deployment queue missing")?
        .parse()?;
    let old_verifier: Address = ethereum_manifest["verifier"]
        .as_str()
        .context("deployment verifier missing")?
        .parse()?;
    let old_client: Address = ethereum_manifest["client"]
        .as_str()
        .context("deployment client missing")?
        .parse()?;
    let controller = recovery_address(plan, "controller")?;
    let recovery_wallet = recovery_address(plan, "recoveryWallet")?;
    let expected_old = recovery_address(plan, "expectedOldVerifier")?;
    let expected_old_client = recovery_address(plan, "expectedOldClient")?;
    let candidate_verifier = recovery_address(plan, "candidateVerifier")?;
    let candidate_client = recovery_address(plan, "candidateClient")?;
    let manifest_controller: Address = ethereum_manifest["recoveryController"]
        .as_str()
        .context("deployment recovery controller missing")?
        .parse()?;
    let manifest_recovery_wallet: Address = ethereum_manifest["recoveryWallet"]
        .as_str()
        .context("deployment recovery wallet missing")?
        .parse()?;
    let source_domain = B256::from(decode_hash(
        &ethereum_manifest["sourceDomain"],
        "sourceDomain",
    )?);
    let bridge_domain = B256::from(decode_hash(
        &ethereum_manifest["bridgeDomain"],
        "bridgeDomain",
    )?);
    let mmr_start = number(&ethereum_manifest["mmrStartBlock"])?;
    let activation_hash = B256::from(decode_hash(&plan["activationTxHash"], "activationTxHash")?);
    let proposal_hash = B256::from(decode_hash(&plan["proposalTxHash"], "proposalTxHash")?);
    let proposal_id = number(&plan["proposalId"])?;
    let proposal_nonce = number(&plan["proposalNonce"])?;
    ensure!(
        proposal_id == proposal_nonce && proposal_id > 0,
        "recovery proposal nonce and id must match"
    );
    ensure!(
        expected_old == old_verifier && expected_old_client == old_client,
        "recovery plan old verifier/client differs from immutable deployment"
    );
    ensure!(
        controller == manifest_controller && recovery_wallet == manifest_recovery_wallet,
        "recovery plan controller/wallet differs from immutable deployment identity"
    );
    require_recovery_wallet_identity(&format!("{manifest_recovery_wallet:#x}"))?;
    ensure!(
        plan["queue"]
            .as_str()
            .is_some_and(|queue| queue.eq_ignore_ascii_case(&format!("{queue_address:#x}")))
            && number(&plan["destinationChainId"])? == HOODI_CHAIN_ID
            && number(&plan["mmrStartBlock"])? == mmr_start
            && B256::from(decode_hash(
                &plan["sourceDomain"],
                "recovery plan sourceDomain"
            )?) == source_domain
            && B256::from(decode_hash(
                &plan["bridgeDomain"],
                "recovery plan bridgeDomain"
            )?) == bridge_domain,
        "recovery plan queue/source/destination identity differs from immutable deployment"
    );
    let ethereum = Ethereum::connect_hoodi_readonly(ethereum_rpc, ethereum_manifest).await?;
    let finalized_receipt = ethereum
        .api
        .get_finalized_receipt(activation_hash)
        .await?
        .context("recovery activation transaction is not finalized")?;
    let finalized_number = finalized_receipt.included_block_number;
    let finalized_hash = finalized_receipt.included_block_hash;
    let provider = ethereum.api.raw_provider().clone();
    let at = BlockId::hash(finalized_hash);
    let queue = RecoveryQueueBinding::new(queue_address, provider.clone());
    let controller_contract = RecoveryControllerBinding::new(controller, provider.clone());
    ensure!(
        queue.verifier().block(at).call().await? == candidate_verifier
            && queue.recoveryController().block(at).call().await? == controller,
        "finalized queue pointer/controller differ from recovery plan"
    );
    ensure!(
        controller_contract.messageQueue().block(at).call().await? == queue_address
            && controller_contract
                .recoveryWallet()
                .block(at)
                .call()
                .await?
                == recovery_wallet
            && controller_contract.proposalNonce().block(at).call().await?
                == U256::from(proposal_nonce),
        "finalized recovery controller binding or proposal nonce differs from plan"
    );

    let old_root = RecoveryVerifierBinding::new(old_verifier, provider.clone());
    let candidate_root = RecoveryVerifierBinding::new(candidate_verifier, provider.clone());
    let old_client_from_verifier = old_root.beefyClient().block(at).call().await?;
    let candidate_client_from_verifier = candidate_root.beefyClient().block(at).call().await?;
    ensure!(
        old_client_from_verifier == old_client
            && candidate_client_from_verifier == candidate_client
            && old_root.messageQueue().block(at).call().await? == queue_address
            && candidate_root.messageQueue().block(at).call().await? == queue_address
            && old_root.destinationChainId().block(at).call().await? == U256::from(HOODI_CHAIN_ID)
            && candidate_root.destinationChainId().block(at).call().await?
                == U256::from(HOODI_CHAIN_ID),
        "old/candidate verifier code bindings differ from the pinned lane"
    );
    let old = RecoveryClientBinding::new(old_client, provider.clone());
    let candidate = RecoveryClientBinding::new(candidate_client, provider.clone());
    let candidate_root_hash = candidate.latestMMRRoot().block(at).call().await?;
    let candidate_block = candidate.latestBeefyBlock().block(at).call().await?;
    let old_block = old.latestBeefyBlock().block(at).call().await?;
    ensure!(
        !old.isLive().block(at).call().await?
            && candidate.isLive().block(at).call().await?
            && candidate_root_hash != B256::ZERO
            && candidate_block > old_block,
        "recovery clients are not expired/live/forward at finalized state"
    );
    for client in [&old, &candidate] {
        ensure!(
            client.sourceDomain().block(at).call().await? == source_domain
                && client.bridgeDomain().block(at).call().await? == bridge_domain
                && client.destinationChainId().block(at).call().await?
                    == U256::from(HOODI_CHAIN_ID)
                && client.destinationQueue().block(at).call().await? == queue_address
                && client.mmrStartBlock().block(at).call().await? == mmr_start,
            "old/candidate client source domain, bridge domain, queue, chain, or MMR start differs"
        );
    }

    let queue_code_hash = recovery_code_hash(&ethereum, queue_address, finalized_hash).await?;
    let old_verifier_code_hash =
        recovery_code_hash(&ethereum, old_verifier, finalized_hash).await?;
    let old_client_code_hash = recovery_code_hash(&ethereum, old_client, finalized_hash).await?;
    for (name, actual) in [
        ("queue", queue_code_hash),
        ("verifier", old_verifier_code_hash),
        ("client", old_client_code_hash),
    ] {
        let pinned = B256::from(decode_hash(
            &ethereum_manifest["bytecodeHashes"][name],
            &format!("deployment bytecodeHashes.{name}"),
        )?);
        ensure!(
            actual == pinned,
            "immutable deployment {name} runtime code hash changed"
        );
    }
    for address in [controller, recovery_wallet] {
        let code = provider
            .get_code_at(address)
            .block_id(BlockId::hash(finalized_hash))
            .await?;
        ensure!(
            !code.is_empty(),
            "pinned recovery controller/Safe has no finalized runtime code"
        );
    }
    let code_hashes = &plan["codeHashes"];
    ensure!(
        B256::from(decode_hash(
            &code_hashes["oldVerifier"],
            "codeHashes.oldVerifier"
        )?) == old_verifier_code_hash
            && B256::from(decode_hash(
                &code_hashes["oldClient"],
                "codeHashes.oldClient"
            )?) == old_client_code_hash,
        "recovery plan old verifier/client runtime hashes differ from the immutable deployment",
    );
    let mut observed = serde_json::Map::new();
    observed.insert(
        "oldVerifier".into(),
        json!(format!("{old_verifier_code_hash:#x}")),
    );
    observed.insert(
        "oldClient".into(),
        json!(format!("{old_client_code_hash:#x}")),
    );
    let candidate_verifier_code_hash = B256::from(decode_hash(
        &code_hashes["candidateVerifier"],
        "codeHashes.candidateVerifier",
    )?);
    let candidate_client_code_hash = B256::from(decode_hash(
        &code_hashes["candidateClient"],
        "codeHashes.candidateClient",
    )?);
    for (name, address, expected) in [
        (
            "candidateVerifier",
            candidate_verifier,
            candidate_verifier_code_hash,
        ),
        (
            "candidateClient",
            candidate_client,
            candidate_client_code_hash,
        ),
    ] {
        let actual = recovery_code_hash(&ethereum, address, finalized_hash).await?;
        ensure!(
            actual == expected,
            "recovery {name} runtime code hash differs from operator pin"
        );
        observed.insert(name.to_owned(), json!(format!("{actual:#x}")));
    }

    let proposal_receipt = ethereum
        .api
        .get_finalized_receipt(proposal_hash)
        .await?
        .context("recovery proposal transaction is not finalized")?;
    ensure!(
        proposal_receipt.receipt.status()
            && proposal_receipt.included_block_number < finalized_number,
        "recovery proposal must succeed before activation"
    );
    let proposal_topic = recovery_number_topic(proposal_id);
    let old_topic = recovery_address_topic(old_verifier);
    let candidate_topic = recovery_address_topic(candidate_verifier);
    let proposed_topics = [alloy::primitives::keccak256(
        b"RecoveryProposed(uint256,address,address,uint256,bytes32,address,bytes32,bytes32,address,bytes32)"),
        proposal_topic, old_topic, candidate_topic];
    let execute_after = U256::from(number(&plan["executeAfter"])?);
    let mut proposal_block_hash = None;
    for log in proposal_receipt.receipt.as_ref().logs() {
        if log.address() != controller || log.topics() != proposed_topics {
            continue;
        }
        let data = log.data().data.as_ref();
        if data.len() != 7 * 32
            || data[64..76].iter().any(|byte| *byte != 0)
            || data[160..172].iter().any(|byte| *byte != 0)
        {
            continue;
        }
        let event_matches = U256::from_be_slice(&data[0..32]) == execute_after
            && B256::from_slice(&data[32..64]) == old_verifier_code_hash
            && Address::from_slice(&data[76..96]) == old_client
            && B256::from_slice(&data[96..128]) == old_client_code_hash
            && B256::from_slice(&data[128..160]) == candidate_verifier_code_hash
            && Address::from_slice(&data[172..192]) == candidate_client
            && B256::from_slice(&data[192..224]) == candidate_client_code_hash;
        if !event_matches {
            continue;
        }
        let block_hash = proposal_receipt.included_block_hash;
        let pending = controller_contract
            .pendingRecovery()
            .block(BlockId::hash(block_hash))
            .call()
            .await?;
        ensure!(
            pending.exists
                && pending.proposalId == U256::from(proposal_id)
                && pending.expectedOldVerifier == old_verifier
                && pending.candidateVerifier == candidate_verifier
                && pending.executeAfter == execute_after
                && pending.expectedOldVerifierCodeHash == old_verifier_code_hash
                && pending.expectedOldClient == old_client
                && pending.expectedOldClientCodeHash == old_client_code_hash
                && pending.candidateVerifierCodeHash == candidate_verifier_code_hash
                && pending.candidateClient == candidate_client
                && pending.candidateClientCodeHash == candidate_client_code_hash,
            "proposal-block pendingRecovery snapshot differs from its finalized event and plan"
        );
        let proposal_block_number = proposal_receipt.included_block_number;
        let proposal_block = provider
            .get_block_by_number(BlockNumberOrTag::Number(proposal_block_number))
            .await?
            .context("finalized recovery proposal block is unavailable")?;
        ensure!(
            proposal_block.header.hash == block_hash,
            "recovery proposal block hash changed from finalized log"
        );
        let recovery_delay = controller_contract
            .RECOVERY_DELAY()
            .block(BlockId::hash(block_hash))
            .call()
            .await?;
        ensure!(
            U256::from(proposal_block.header.timestamp) + recovery_delay == execute_after,
            "proposal executeAfter does not match the controller's pinned recovery delay"
        );
        proposal_block_hash = Some(block_hash);
    }
    let proposal_block_hash = proposal_block_hash.context(
        "exact finalized recovery proposal snapshot is missing or differs from its plan",
    )?;
    let activation_block = provider
        .get_block_by_number(BlockNumberOrTag::Number(
            finalized_receipt.included_block_number,
        ))
        .await?
        .context("finalized recovery activation block is unavailable")?;
    ensure!(
        activation_block.header.hash == finalized_receipt.included_block_hash
            && U256::from(activation_block.header.timestamp) >= execute_after,
        "recovery activation is not canonical or precedes its pinned timelock"
    );
    let logs = finalized_receipt.receipt.as_ref().logs();
    let executed_topics = [
        alloy::primitives::keccak256(b"RecoveryExecuted(uint256,address,address)"),
        proposal_topic,
        old_topic,
        candidate_topic,
    ];
    let activated_topics = [
        alloy::primitives::keccak256(b"RecoveryVerifierActivated(address,address)"),
        old_topic,
        candidate_topic,
    ];
    ensure!(
        finalized_receipt.receipt.status()
            && logs
                .iter()
                .any(|log| log.address() == controller && log.topics() == executed_topics)
            && logs
                .iter()
                .any(|log| log.address() == queue_address && log.topics() == activated_topics),
        "finalized activation receipt does not contain matching controller and queue events"
    );

    let mut active = ethereum_manifest.clone();
    active["client"] = json!(format!("{candidate_client:#x}"));
    active["verifier"] = json!(format!("{candidate_verifier:#x}"));
    active["bytecodeHashes"]["client"] = json!(observed["candidateClient"]);
    active["bytecodeHashes"]["verifier"] = json!(observed["candidateVerifier"]);
    active["bindings"]["beefyClient"] = json!(format!("{candidate_client:#x}"));
    Ok(json!({
        "queue": format!("{queue_address:#x}"),
        "controller": format!("{controller:#x}"),
        "recoveryWallet": format!("{recovery_wallet:#x}"),
        "oldVerifier": format!("{old_verifier:#x}"),
        "oldClient": format!("{old_client:#x}"),
        "candidateVerifier": format!("{candidate_verifier:#x}"),
        "candidateClient": format!("{candidate_client:#x}"),
        "sourceDomain": format!("{source_domain:#x}"),
        "bridgeDomain": format!("{bridge_domain:#x}"),
        "proposalNonce": proposal_nonce,
        "proposalId": proposal_id,
        "executeAfter": execute_after.to_string(),
        "proposalBlockHash": format!("{proposal_block_hash:#x}"),
        "proposalTxHash": format!("{proposal_hash:#x}"),
        "activationTxHash": format!("{activation_hash:#x}"),
        "activationBlock": finalized_receipt.included_block_number,
        "activationBlockHash": format!("{:#x}", finalized_receipt.included_block_hash),
        "codeHashes": observed,
        "activeEthereum": active,
        "finalized": true,
    }))
}

pub(crate) fn ensure_no_unresolved_recovery_intents(directory: &Path) -> Result<()> {
    let state_path = directory.join("state.json");
    if state_path.try_exists()? {
        let state: Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        ensure!(state["submission"].is_null(),
            "old-client commitment submission is unresolved; preserve its hash/nonce and reconcile before cutover");
        if !state["roots"].is_null() {
            for root in state["roots"]
                .as_object()
                .context("old-client root registry is malformed")?
                .values()
            {
                ensure!(root["status"] == "accepted",
                    "old-client root registration is unresolved; preserve and reconcile its original publication before cutover");
            }
        }
    }
    let publications = directory.join("root-publications");
    if publications.try_exists()? {
        for entry in fs::read_dir(publications)? {
            let path = entry?.path();
            ensure!(
                path.extension().and_then(|extension| extension.to_str()) != Some("tmp"),
                "unfinished root publication write at {}; preserve and reconcile it before cutover",
                path.display()
            );
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let intent: Value = serde_json::from_slice(&fs::read(&path)?)?;
            ensure!(intent["status"] == "accepted" && intent["finalityStatus"] == "finalized",
                "old-client publication at {} is unresolved, including an unsigned prepared intent; retain its client/anchor/nonce/hash and reconcile before cutover",
                path.display());
        }
    }
    Ok(())
}

pub(crate) fn read_recovery_history(directory: &Path) -> Result<Value> {
    let path = directory.join("recovery-transition.json");
    ensure!(
        !path.with_extension("json.tmp").try_exists()?,
        "unfinished recovery history write; preserve and reconcile both journal files"
    );
    if !path.try_exists()? {
        return Ok(json!({"schemaVersion":2,"transitions":[]}));
    }
    let history: Value = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        history["schemaVersion"] == 2 && history["transitions"].is_array(),
        "unsupported or malformed recovery transition history"
    );
    Ok(history)
}

pub(crate) async fn verify_recovery_history<'a>(
    ethereum_rpc: &str,
    deployment: &'a Value,
    history: &'a Value,
) -> Result<&'a Value> {
    let mut active = &deployment["ethereum"];
    let mut nonce = 0;
    let mut activation_block = 0;
    for record in history["transitions"]
        .as_array()
        .context("recovery history is not an array")?
    {
        let verification =
            verify_recovery_transition(ethereum_rpc, deployment, active, &record["plan"]).await?;
        ensure!(
            verification == record["verification"],
            "recovery history differs from canonical activation evidence"
        );
        let next_nonce = number(&verification["proposalNonce"])?;
        let next_block = number(&verification["activationBlock"])?;
        ensure!(
            next_nonce > nonce && next_block > activation_block,
            "recovery history replays or reorders a consumed proposal"
        );
        nonce = next_nonce;
        activation_block = next_block;
        active = &record["verification"]["activeEthereum"];
    }
    Ok(active)
}

pub async fn activate_recovery_transition(
    ethereum_rpc: &str,
    deployment_path: &Path,
    follower_dir: &Path,
    recovery_plan_path: &Path,
) -> Result<()> {
    let _owner = lock_token_deployment(deployment_path)?;
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_path)?)?;
    let plan: Value = serde_json::from_slice(&fs::read(recovery_plan_path)?)?;
    let follower = crate::hoodi::read_state_file(&follower_dir.join("state.json"))?;
    crate::hoodi::validate_token_follower_journal(&follower, &deployment)?;
    ensure!(
        follower["localRehearsal"] == false && !follower["bootstrap"].is_null(),
        "recovery cutover requires the established real-Hoodi follower bootstrap"
    );
    let mut history = read_recovery_history(follower_dir)?;
    let active = verify_recovery_history(ethereum_rpc, &deployment, &history).await?;
    let records = history["transitions"]
        .as_array()
        .context("missing recovery history")?;
    if records.last().is_some_and(|record| record["plan"] == plan) {
        println!(
            "verified existing finalized recovery; restart tokens-follow with the same journal"
        );
        return Ok(());
    }
    ensure!(
        follower["activeEthereum"] == *active,
        "follower active client differs from its finalized recovery history"
    );
    ensure!(
        follower["recoveryCandidate"]["identity"]
            == crate::hoodi::recovery_candidate_identity(&plan),
        "activation plan differs from the follower's pinned recovery candidate"
    );
    let candidate_key = format!("{:#x}", recovery_address(&plan, "candidateClient")?);
    let bootstrap = &follower["recoveryBootstraps"][candidate_key];
    ensure!(
        bootstrap.is_object()
            && bootstrap["block"] == plan["bootstrap"]["block"]
            && bootstrap["blockHash"] == plan["bootstrap"]["blockHash"]
            && bootstrap["rawScale"] == plan["bootstrap"]["signedCommitmentScale"]
            && bootstrap["current"] == plan["bootstrap"]["current"]
            && bootstrap["next"] == plan["bootstrap"]["next"],
        "activation candidate has no matching independently authenticated bootstrap"
    );
    ensure_no_unresolved_recovery_intents(follower_dir)?;
    let ethereum = crate::ethereum::Ethereum::connect_hoodi_readonly(ethereum_rpc, active).await?;
    let publisher: Address = follower["rootPublisherSigner"]
        .as_str()
        .context("root publisher identity missing")?
        .parse()?;
    let (latest_nonce, pending_nonce) = root_account_nonces(&ethereum, publisher).await?;
    let (_, finalized_hash) = finalized_ethereum_head(&ethereum).await?;
    let finalized_nonce = ethereum
        .api
        .raw_provider()
        .get_transaction_count(publisher)
        .block_id(alloy::eips::BlockId::hash(finalized_hash))
        .await?;
    ensure!(
        latest_nonce == pending_nonce && latest_nonce == finalized_nonce,
        "root publisher nonce is pending or unfinalized; reconcile it before cutover"
    );
    let verification = verify_recovery_transition(ethereum_rpc, &deployment, active, &plan).await?;
    if let Some(previous) = records.last() {
        ensure!(
            number(&verification["proposalNonce"])?
                > number(&previous["verification"]["proposalNonce"])?
                && number(&verification["activationBlock"])?
                    > number(&previous["verification"]["activationBlock"])?,
            "recovery activation replays or reorders a previous proposal"
        );
    }
    history["transitions"]
        .as_array_mut()
        .expect("validated recovery history")
        .push(json!({"plan":plan,"verification":verification}));
    persist_root_intent(&follower_dir.join("recovery-transition.json"), &history)?;
    println!("verified finalized verifier recovery; restart tokens-follow to consume the appended transition");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn publish_root(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    deployment_manifest: &Path,
    publisher_state: &Path,
    registration: &Path,
    output: &Path,
) -> Result<()> {
    let _owner = lock_token_deployment(deployment_manifest)?;
    let registration: Value = serde_json::from_slice(&fs::read(registration)?)?;
    loop {
        if publish_root_locked(
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            wallet,
            deployment_manifest,
            publisher_state,
            &registration,
            output,
        )
        .await?
            == RootPublicationState::Accepted
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

// Caller owns the deployment lock across anchor selection and publication.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn publish_root_locked(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    wallet: &Path,
    deployment_manifest: &Path,
    publisher_state: &Path,
    registration: &Value,
    output: &Path,
) -> Result<RootPublicationState> {
    use crate::{ethereum::Ethereum, rehearsal::proof_evidence, source::Source};
    use alloy::{
        eips::Encodable2718,
        primitives::{B256, U256},
        providers::Provider,
        rpc::types::{BlockNumberOrTag, Filter},
    };
    use anyhow::Context;
    use beefy_relay::{encode_queue_proof, keccak256};
    use gear_rpc_client::GearApi as SourceApi;
    use gsdk::ext::subxt::utils::H256;

    ensure!(
        crate::local_source_rpc(source_rpc) && crate::local_source_rpc(witness_rpc),
        "public dev keys: source and witness RPCs must be loopback"
    );
    ensure!(
        source_rpc != witness_rpc,
        "independent Gear authorities required"
    );
    let manifest: Value = serde_json::from_slice(&fs::read(deployment_manifest)?)?;
    ensure!(
        manifest["mode"] == "hoodi-token-stack",
        "not the isolated token deployment"
    );
    let block: u32 = registration["block"]
        .as_u64()
        .ok_or_else(|| anyhow!("registration block missing"))?
        .try_into()?;
    let kind = registration["kind"].as_str().unwrap_or("merkleRoot");
    ensure!(
        matches!(kind, "merkleRoot" | "emptyProgress"),
        "unknown queue progress registration kind"
    );
    let root = decode_hash(&registration["queueRoot"], "registration root")?;
    ensure!(
        if kind == "emptyProgress" {
            root == [0; 32]
        } else {
            root != [0; 32]
        },
        "registration root does not match its progress kind"
    );
    let root_text = format!("0x{}", hex::encode(root));
    let existing_journal: Value = serde_json::from_slice(&fs::read(publisher_state)?)?;
    crate::hoodi::validate_token_follower_journal(&existing_journal, &manifest)?;
    let mut matches = existing_journal["roots"]
        .as_object()
        .context("root publication inventory missing; HOLD")?
        .values()
        .filter(|saved| {
            saved["block"] == block
                && saved["queueRoot"] == root_text
                && saved["kind"].as_str().unwrap_or("merkleRoot") == kind
        });
    let registered = matches
        .next()
        .context("publication has no original follower registration; HOLD")?;
    ensure!(
        matches.next().is_none(),
        "publication has ambiguous follower registrations; HOLD"
    );
    let registered_path = Path::new(
        registered["publication"]
            .as_str()
            .context("registered publication path missing; HOLD")?,
    );
    ensure!(
        output == registered_path
            && output.parent()
                == Some(
                    state_directory(publisher_state)
                        .join("root-publications")
                        .as_path()
                ),
        "publication output differs from its owned original registration; HOLD"
    );
    let local_rehearsal = existing_journal["localRehearsal"]
        .as_bool()
        .context("actor rehearsal mode missing")?;
    ensure!(
        manifest["localRehearsal"] == local_rehearsal,
        "deployment rehearsal mode differs from actor"
    );
    ensure!(
        if local_rehearsal {
            crate::local_source_rpc(ethereum_rpc)
        } else {
            ethereum_rpc.starts_with("wss://")
        },
        "Hoodi requires wss; local rehearsal requires a loopback websocket"
    );
    fs::create_dir_all(state_directory(output))?;
    ensure!(
        !output.exists() || output.is_file(),
        "publication evidence path is not a file"
    );
    let prior = if output.exists() {
        let intent: Value = serde_json::from_slice(&fs::read(output)?)?;
        ensure!(
            intent["schemaVersion"] == 3
                && intent["sourceBlock"] == block
                && intent["root"] == root_text
                && intent["kind"].as_str().unwrap_or("merkleRoot") == kind,
            "publication intent is incompatible with this root or schema"
        );
        Some(intent)
    } else {
        None
    };

    let active_ethereum = if existing_journal["activeEthereum"].is_object() {
        existing_journal["activeEthereum"].clone()
    } else {
        manifest["ethereum"].clone()
    };
    ensure!(
        active_ethereum["chainId"] == manifest["ethereum"]["chainId"]
            && active_ethereum["queue"] == manifest["ethereum"]["queue"]
            && active_ethereum["receiver"] == manifest["ethereum"]["receiver"]
            && active_ethereum["sourceDomain"] == manifest["ethereum"]["sourceDomain"]
            && active_ethereum["bridgeDomain"] == manifest["ethereum"]["bridgeDomain"]
            && active_ethereum["mmrStartBlock"] == manifest["ethereum"]["mmrStartBlock"],
        "actor active BEEFY identity differs from the immutable token deployment"
    );
    let ethereum = Ethereum::connect_hoodi(ethereum_rpc, wallet, &active_ethereum).await?;
    ethereum
        .verify_token_bindings(
            actor(
                manifest["gearManager"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Gear manager missing"))?,
            )?
            .into(),
        )
        .await?;
    let publisher_signer = crate::ethereum::hoodi_wallet_address(wallet)?;
    let publisher_address = format!("{publisher_signer:#x}");

    let (source, witness) = Source::connect_pair(
        SourceApi::new(source_rpc, 3).await?,
        SourceApi::new(witness_rpc, 3).await?,
    )
    .await?;
    source.validate_attachment(&manifest["anchor"]).await?;
    ensure_source_identity(
        &manifest["ethereum"],
        source.source_genesis,
        source.bridge_domain,
        witness.source_genesis,
        witness.bridge_domain,
    )?;
    ensure!(
        manifest["anchor"]["sourceGenesis"] == format!("0x{}", hex::encode(source.source_genesis))
            && manifest["anchor"]["bridgeDomain"]
                == format!("0x{}", hex::encode(source.bridge_domain)),
        "token deployment anchor differs from current source identity"
    );

    let active_client = active_ethereum["client"]
        .as_str()
        .context("active BEEFY client address missing")?;
    let prior_anchor = prior
        .as_ref()
        .map(|intent| {
            ensure!(
                intent["acceptedAnchorClient"] == active_client,
                "saved root client differs from active deployment; hold original intent"
            );
            intent["acceptedAnchorTx"]
                .as_str()
                .context("saved root anchor transaction is missing")
        })
        .transpose()?;
    let journal = crate::hoodi::read_state_file(publisher_state)?;
    crate::hoodi::validate_token_follower_journal(&journal, &manifest)?;
    let follower_signer = journal["followerSigner"]
        .as_str()
        .context("token follower signer is missing")?
        .parse()?;
    ensure_separate_evm_signers(follower_signer, publisher_signer)?;
    ensure!(
        journal["rootPublisherSigner"] == publisher_address,
        "root publisher signer differs from actor journal"
    );
    ensure!(
        journal["follower"]["status"] != "failed",
        "token actor is failed; reconcile it before maintenance publication"
    );
    if !journal["submission"].is_null() {
        let saved = prior
            .as_ref()
            .context("pending BEEFY commitment overlaps an unsigned root publication")?;
        ensure!(
            !saved["publicationReceipt"].is_null()
                && canonical_root_inclusion(&ethereum, saved, block, kind, root)
                    .await?
                    .is_some(),
            "pending BEEFY commitment overlaps an unmined root publication; hold original intents"
        );
    }
    let commitments = journal["commitments"]
        .as_array()
        .context("token follower journal has no accepted commitments")?;
    let legacy_client = manifest["ethereum"]["client"]
        .as_str()
        .context("immutable deployment client is missing")?
        .parse()?;
    let selected_client: Address = active_client.parse()?;
    let mut selected = None;
    for entry in commitments.iter().rev() {
        if crate::hoodi::journal_client(entry, legacy_client)? == selected_client
            && prior_anchor.is_none_or(|tx_hash| entry["txHash"].as_str() == Some(tx_hash))
        {
            selected = Some(entry);
            break;
        }
    }
    let last = selected.context("publication anchor is absent from active commitment history")?;
    ensure!(
        last["block"]
            .as_u64()
            .context("accepted anchor block missing")?
            > u64::from(block),
        "actor must accept a covering anchor before root publication"
    );

    let anchor_block: u32 = last["block"]
        .as_u64()
        .context("accepted anchor block missing")?
        .try_into()?;
    let raw = hex::decode(
        last["rawScale"]
            .as_str()
            .context("accepted signed commitment missing")?
            .trim_start_matches("0x"),
    )?;
    let (anchor, second_anchor) = tokio::try_join!(
        source.recapture(anchor_block, raw.clone()),
        witness.recapture(anchor_block, raw.clone()),
    )?;
    let source_finalized = source
        .api
        .block_hash_to_number(source.api.latest_finalized_block().await?)
        .await?;
    let witness_finalized = witness
        .api
        .block_hash_to_number(witness.api.latest_finalized_block().await?)
        .await?;
    ensure!(
        source_finalized >= anchor_block && witness_finalized >= anchor_block,
        "accepted source anchor is not finalized by both Gear authorities"
    );
    ensure!(
        source.api.block_number_to_hash(anchor_block).await? == H256(anchor.block_hash)
            && witness.api.block_number_to_hash(anchor_block).await? == H256(anchor.block_hash),
        "accepted source anchor was reorged at one of the Gear authorities"
    );
    let recorded_hash = decode_hash(&last["blockHash"], "accepted anchor block hash")?;
    ensure_recaptured_anchor(
        block,
        (
            anchor_block,
            recorded_hash,
            last["rawScale"]
                .as_str()
                .context("accepted commitment SCALE missing")?,
        ),
        (anchor.block, anchor.block_hash, &anchor.raw),
        (
            second_anchor.block,
            second_anchor.block_hash,
            &second_anchor.raw,
        ),
        &raw,
    )?;
    let (proof, second) = tokio::try_join!(
        source.proof(block, &anchor),
        witness.proof(block, &second_anchor)
    )?;
    ensure!(
        proof.snapshot == second.snapshot
            && proof.raw_leaf == second.raw_leaf
            && proof.source_hash == second.source_hash
            && proof.snapshot.bridge_domain == source.bridge_domain
            && registration["blockHash"] == format!("0x{}", hex::encode(proof.source_hash))
            && registration["queueId"] == proof.snapshot.queue_id
            && proof.snapshot.initialized
            && proof.snapshot.queue_root == root
            && proof.leaf.leaf_extra == proof.snapshot.hash(),
        "historical registration root or independently witnessed MMR proof differs"
    );
    let accepted_tx = decode_hash(&last["txHash"], "accepted anchor transaction hash")?;
    let accepted_anchor_tx = format!("0x{}", hex::encode(accepted_tx));
    let anchor_proof;
    let anchor_snapshot = if block == anchor.block - 1 {
        &proof.snapshot
    } else {
        let (first, second) = tokio::try_join!(
            source.proof(anchor.block - 1, &anchor),
            witness.proof(anchor.block - 1, &second_anchor)
        )?;
        ensure!(
            first.snapshot == second.snapshot
                && first.raw_leaf == second.raw_leaf
                && first.source_hash == second.source_hash,
            "accepted checkpoint snapshot differs between authorities"
        );
        anchor_proof = first;
        &anchor_proof.snapshot
    };
    let (destination_block, destination_hash) = ethereum
        .verify_accepted_commitment(B256::from(accepted_tx), &anchor, anchor_snapshot)
        .await?;
    let destination_hash_text = format!("{destination_hash:#x}");
    ensure!(
        last["destinationBlock"].as_u64() == Some(destination_block)
            && last["destinationHash"].as_str() == Some(destination_hash_text.as_str()),
        "accepted Hoodi checkpoint is noncanonical or differs from follower journal"
    );
    let encoded = encode_queue_proof(
        &proof.snapshot,
        u64::from(anchor.block),
        anchor.validated.mmr_root,
        &proof.leaf,
        &proof.simplified.items,
        proof.simplified.proof_order,
    )?;
    let proof_record = proof_evidence(&proof, &anchor, &encoded);
    let immutable_intent = json!({
        "schemaVersion": 3,
        "sourceBlock": block,
        "root": root_text,
        "kind": kind,
        "localRehearsal": journal["localRehearsal"],
        "sourceIdentity": {
            "sourceGenesis": manifest["ethereum"]["sourceGenesis"],
            "sourceDomain": manifest["ethereum"]["sourceDomain"],
            "bridgeDomain": manifest["ethereum"]["bridgeDomain"],
            "destinationChainId": manifest["ethereum"]["chainId"],
            "destinationQueue": manifest["ethereum"]["queue"],
        },
        "acceptedAnchorTx": accepted_anchor_tx,
        "acceptedAnchorClient": active_client,
        "acceptedCheckpoint": {
            "block": destination_block,
            "blockHash": format!("{destination_hash:#x}"),
        },
        "sender": publisher_address,
        "proof": proof_record,
        "queueProof": format!("0x{}", hex::encode(&encoded)),
    });
    let mut intent = prior.clone().unwrap_or_else(|| {
        json!({
            "schemaVersion": 3,
            "sourceBlock": block,
            "root": root_text,
            "kind": kind,
            "localRehearsal": immutable_intent["localRehearsal"],
            "sourceIdentity": immutable_intent["sourceIdentity"].clone(),
            "acceptedAnchorTx": accepted_anchor_tx,
            "acceptedAnchorClient": active_client,
            "acceptedCheckpoint": immutable_intent["acceptedCheckpoint"].clone(),
            "sender": publisher_address,
            "proof": proof_record,
            "queueProof": format!("0x{}", hex::encode(&encoded)),
            "nonce": null,
            "txHash": null,
            "rawTransaction": null,
            "status": "prepared",
        })
    });
    if let Some(saved) = prior.as_ref() {
        ensure_immutable_publication(saved, &immutable_intent)?;
    }
    ensure!(
        intent.get("nonce").is_some()
            && intent.get("txHash").is_some()
            && intent.get("rawTransaction").is_some(),
        "publication intent is missing durable nonce or transaction hash fields"
    );
    ensure!(
        intent["txHash"].is_null() == intent["rawTransaction"].is_null()
            && (intent["txHash"].is_null() || !intent["nonce"].is_null()),
        "saved root transaction hash has no reserved nonce"
    );
    ensure!(
        intent["txHash"].is_null() || intent["txHash"].as_str().is_some(),
        "saved root transaction hash is invalid"
    );
    persist_root_intent(output, &intent)?;

    let (finalized_block, finalized_hash) = finalized_ethereum_head(&ethereum).await?;
    let existing = if kind == "emptyProgress" {
        let provider = ethereum.api.raw_provider();
        let event_signature = B256::from(keccak256(b"EmptyQueueProgress(uint256)"));
        let block_topic = B256::from(U256::from(block).to_be_bytes::<32>());
        let logs = provider
            .get_logs(
                &Filter::new()
                    .address(ethereum.queue_address)
                    .event_signature(event_signature)
                    .topic1(block_topic)
                    .to_block(BlockNumberOrTag::Number(finalized_block)),
            )
            .await?;
        ensure!(
            logs.len() <= 1,
            "duplicate finalized empty queue progress event"
        );
        if let Some(log) = logs.first() {
            ensure!(
                log.topics().len() == 2,
                "malformed empty queue progress event"
            );
            let number = log
                .block_number
                .context("empty progress event block missing")?;
            let hash = log
                .block_hash
                .context("empty progress event hash missing")?;
            ensure!(
                number <= finalized_block,
                "empty progress event is not finalized"
            );
            ensure!(
                provider
                    .get_block_by_number(BlockNumberOrTag::Number(number))
                    .await?
                    .is_some_and(|canonical| canonical.header.hash == hash),
                "empty progress event block is not canonical"
            );
            let tx_hash = log
                .transaction_hash
                .context("empty progress transaction hash missing")?;
            let finalized = ethereum
                .api
                .get_finalized_receipt(tx_hash)
                .await?
                .context("empty progress event receipt is not finalized")?;
            ensure!(
                finalized.receipt.status()
                    && finalized.included_block_number == number
                    && finalized.included_block_hash == hash,
                "empty progress event receipt differs from the finalized log"
            );
            Some([0; 32])
        } else {
            None
        }
    } else {
        ethereum
            .api
            .read_finalized_merkle_root_at(block, finalized_block, finalized_hash)
            .await?
    };
    if let Some(existing) = existing {
        ensure!(
            existing == root,
            "conflicting finalized queue root already stored"
        );
    }
    let mined = if intent["txHash"].is_null() {
        None
    } else {
        canonical_root_inclusion(&ethereum, &intent, block, kind, root).await?
    };
    ensure!(
        existing.is_none() || mined.is_some(),
        "finalized queue progress has no canonical receipt for the original root transaction"
    );
    if mined.is_none() {
        ensure_pinned_latest_anchor(
            &ethereum.checkpoint().await?,
            anchor_block,
            anchor.validated.mmr_root,
        )?;
    }

    let saved_nonce = if intent["nonce"].is_null() {
        let (latest, pending) = root_account_nonces(&ethereum, publisher_signer).await?;
        ensure!(
            latest == pending,
            "publisher account has an outstanding transaction; reconcile it before root publication"
        );
        intent["nonce"] = json!(pending.to_string());
        persist_root_intent(output, &intent)?;
        pending
    } else {
        saved_publication_nonce(&intent)?
    };
    let provider = ethereum.api.raw_provider();
    if intent["txHash"].is_null() {
        ensure_pinned_latest_anchor(
            &ethereum.checkpoint().await?,
            anchor_block,
            anchor.validated.mmr_root,
        )?;
        let (latest, pending) = root_account_nonces(&ethereum, publisher_signer).await?;
        ensure_saved_nonce_available(saved_nonce, latest, pending)?;
        let request = if kind == "emptyProgress" {
            ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, provider.clone())
                .submitEmptyQueueProgress(
                    alloy::primitives::U256::from(block),
                    encoded.clone().into(),
                )
                .nonce(saved_nonce)
                .into_transaction_request()
        } else {
            ethereum_client::abi::IMessageQueue::new(ethereum.queue_address, provider.clone())
                .submitMerkleRoot(
                    alloy::primitives::U256::from(block),
                    B256::from(root),
                    encoded.clone().into(),
                )
                .nonce(saved_nonce)
                .into_transaction_request()
        };
        let filled = provider
            .fill(request)
            .await
            .context("sign original queue-root transaction")?;
        let raw = filled
            .as_envelope()
            .context("root transaction was not signed")?
            .encoded_2718();
        intent["rawTransaction"] = json!(format!("0x{}", hex::encode(&raw)));
        intent["txHash"] = json!(format!("0x{}", hex::encode(beefy_relay::keccak256(&raw))));
        intent["status"] =
            json!("signed original root transaction; broadcast outcome not yet known");
        persist_root_intent(output, &intent)?;
    }
    let signed = signed_publication_identity(&intent, ethereum.queue_address)?;
    let transaction_hash = signed.hash;
    ensure!(
        signed.from == publisher_signer && signed.nonce == saved_nonce,
        "saved root publisher or reserved nonce differs from signed intent"
    );
    let raw = hex::decode(
        intent["rawTransaction"]
            .as_str()
            .context("saved signed root transaction missing")?
            .trim_start_matches("0x"),
    )?;
    if mined.is_none() {
        if let Some(observed) =
            ethereum_client::transaction_identity(provider, transaction_hash).await?
        {
            crate::ethereum::ensure_same_submission(observed, signed)?;
        } else {
            ensure_pinned_latest_anchor(
                &ethereum.checkpoint().await?,
                anchor_block,
                anchor.validated.mmr_root,
            )?;
            let (latest, pending) = root_account_nonces(&ethereum, publisher_signer).await?;
            ensure_saved_nonce_available(saved_nonce, latest, pending)?;
            // Recovery can only rebroadcast these identical signed bytes, never replace the transaction.
            match provider.send_raw_transaction(&raw).await {
                Ok(transaction) => ensure!(
                    *transaction.tx_hash() == transaction_hash,
                    "RPC returned a different root transaction hash"
                ),
                Err(_) => {
                    intent["lastError"] = json!(
                        "send API returned an error; reconcile the original signed hash and nonce"
                    );
                    persist_root_intent(output, &intent)?;
                }
            }
        }
    }
    if mined.is_none() {
        intent["status"] =
            json!("broadcast outcome pending; reconcile the saved signed transaction");
        persist_root_intent(output, &intent)?;
    }
    let finalized = wait_for_root_receipt(
        &ethereum,
        output,
        &mut intent,
        block,
        kind,
        root,
        anchor_block,
        anchor.validated.mmr_root,
        B256::from(accepted_tx),
        (destination_block, destination_hash),
    )
    .await?;
    if finalized == RootPublicationState::Accepted {
        println!("accepted token queue root at Gear {block}, Hoodi tx {transaction_hash:#x}");
    }
    Ok(finalized)
}
#[cfg(test)]
mod configuration_recovery_tests {
    use super::*;

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

    #[test]
    fn finalized_role_and_mapping_actions_resume_without_resubmitting() -> Result<()> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("beefy-token-config-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory)?;
        let stack_path = directory.join("token-stack.json");
        fs::write(
            &stack_path,
            serde_json::to_vec(&json!({"configuration": {"actions": {}}}))?,
        )?;

        for (key, intent, readback) in [
            (
                "circle.minter",
                json!({"role": "minter", "authority": "0xmanager"}),
                json!({"authority": "0xmanager"}),
            ),
            (
                "circle.mapping",
                json!({"erc20": "0xtoken", "supply": "ethereum"}),
                json!({"erc20": "0xtoken", "supply": "ethereum"}),
            ),
        ] {
            assert!(prepare_configuration_action(
                &stack_path,
                key,
                intent.clone(),
                false,
                false
            )?);
            let mut state = read_config_stack(&stack_path)?;
            state["configuration"]["actions"][key]["messages"] =
                json!([{"messageId": "0xoriginal", "status": "submitted"}]);
            save_config_stack(&stack_path, &state)?;

            assert!(!prepare_configuration_action(
                &stack_path,
                key,
                intent.clone(),
                true,
                false
            )?);
            mark_configuration_action_verified(&stack_path, key, readback.clone())?;
            let action = read_config_stack(&stack_path)?["configuration"]["actions"][key].clone();
            assert_eq!(action["status"], "verified");
            assert_eq!(action["intent"], intent);
            assert_eq!(action["readback"], readback);
            assert_eq!(action["messages"][0]["messageId"], "0xoriginal");
        }

        let key = "source-inventory-nativeVft";
        let intent =
            json!({"native": true, "amountRaw": "2000000000000", "campaign": "0xoriginal"});
        assert!(prepare_configuration_action(
            &stack_path,
            key,
            intent.clone(),
            false,
            false
        )?);
        // A crash before signing may resume; once raw bytes are journaled it may not mint again.
        assert!(prepare_configuration_action(
            &stack_path,
            key,
            intent.clone(),
            false,
            false
        )?);
        record_configuration_message_start(
            &stack_path,
            key,
            ActorId::from([3; 32]),
            &[1, 2],
            100,
            2_000_000_000_000,
            json!({"nonce": 7, "rawExtrinsic": "0x010203"}),
        )?;
        let original = fs::read(&stack_path)?;
        assert!(
            prepare_configuration_action(&stack_path, key, intent.clone(), false, false).is_err()
        );
        assert_eq!(fs::read(&stack_path)?, original);
        assert!(prepare_configuration_action(
            &stack_path,
            key,
            json!({"native": true, "amountRaw": "2000000000000", "campaign": "0xreplacement"}),
            true,
            false
        )
        .is_err());
        assert_eq!(fs::read(&stack_path)?, original);
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}

#[cfg(test)]
mod root_publication_tests {
    use super::*;

    #[test]
    fn recovery_cutover_holds_unresolved_journal_writes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let state = directory.path().join("state.json");
        fs::write(&state, br#"{"submission":null}"#)?;
        let publications = directory.path().join("root-publications");
        fs::create_dir(&publications)?;
        let temporary = publications.join("root.json.tmp");
        fs::write(&temporary, br#"{"nonce":7,"status":"pending"}"#)?;
        assert!(ensure_no_unresolved_recovery_intents(directory.path()).is_err());
        let committed = publications.join("root.json");
        fs::rename(temporary, &committed)?;
        assert!(ensure_no_unresolved_recovery_intents(directory.path()).is_err());
        fs::write(
            &committed,
            br#"{"status":"prepared","nonce":null,"txHash":null,"rawTransaction":null}"#,
        )?;
        assert!(ensure_no_unresolved_recovery_intents(directory.path()).is_err());
        fs::write(
            &committed,
            br#"{"nonce":7,"status":"accepted","finalityStatus":"finalized"}"#,
        )?;
        ensure_no_unresolved_recovery_intents(directory.path())?;
        fs::write(
            &state,
            br#"{"submission":null,"roots":{"original":{"status":"pending"}}}"#,
        )?;
        assert!(ensure_no_unresolved_recovery_intents(directory.path()).is_err());
        fs::write(
            &state,
            br#"{"submission":null,"roots":{"original":{"status":"accepted"}}}"#,
        )?;
        ensure_no_unresolved_recovery_intents(directory.path())?;
        fs::write(state, br#"{"submission":{"nonce":8}}"#)?;
        assert!(ensure_no_unresolved_recovery_intents(directory.path()).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn mined_root_precedes_acceptance_and_keeps_the_original_signed_intent() -> Result<()> {
        use alloy::{
            consensus::{SignableTransaction, TxEip1559, TxEnvelope},
            eips::Encodable2718,
            network::TxSigner,
            primitives::TxKind,
            signers::local::PrivateKeySigner,
        };
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("root.json");
        let queue = Address::from([4; 20]);
        let wallet = PrivateKeySigner::from_bytes(&B256::from([7; 32]))?;
        let mut transaction = TxEip1559 {
            chain_id: HOODI_CHAIN_ID,
            nonce: 4,
            gas_limit: 21_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(queue),
            ..Default::default()
        };
        let signature = wallet.sign_transaction(&mut transaction).await?;
        let raw = TxEnvelope::Eip1559(transaction.into_signed(signature)).encoded_2718();
        let hash = alloy::primitives::keccak256(&raw);
        let mut intent = json!({
            "sourceBlock": 21, "acceptedAnchorTx": "0xanchor", "proof": {"anchorBlock": 25},
            "queueProof": "0x0102", "nonce": "4", "sender": wallet.address(),
            "rawTransaction": format!("0x{}", hex::encode(&raw)),
            "txHash": format!("{hash:#x}"),
        });
        let signed = intent.clone();
        assert_eq!(
            hold_root_reconciliation(&path, &mut intent)?,
            RootPublicationState::Pending
        );
        let held: Value = serde_json::from_slice(&fs::read(&path)?)?;
        for field in [
            "nonce",
            "txHash",
            "rawTransaction",
            "acceptedAnchorTx",
            "proof",
            "queueProof",
        ] {
            assert_eq!(held[field], signed[field]);
        }
        assert_eq!(signed_publication_identity(&held, queue)?.hash, hash);
        let inclusion = (41, B256::from([1; 32]));
        persist_root_inclusion(&path, &mut intent, inclusion, "merkleRoot", None)?;
        let mined: Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(mined["status"], "mined");
        assert_eq!(mined["publicationReceipt"]["finalized"], false);
        for field in [
            "nonce",
            "txHash",
            "rawTransaction",
            "acceptedAnchorTx",
            "proof",
            "queueProof",
        ] {
            assert_eq!(mined[field], signed[field]);
        }
        assert_eq!(signed_publication_identity(&mined, queue)?.hash, hash);
        assert_eq!(
            hold_root_reconciliation(&path, &mut intent)?,
            RootPublicationState::Mined
        );
        assert_eq!(intent["publicationReceipt"], mined["publicationReceipt"]);
        let finalized = (48, B256::from([9; 32]));
        persist_root_inclusion(&path, &mut intent, inclusion, "merkleRoot", Some(finalized))?;
        let accepted: Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(accepted["status"], "accepted");
        assert_eq!(accepted["finalityStatus"], "finalized");
        assert_eq!(accepted["publicationReceipt"]["finalized"], true);
        assert_eq!(accepted["publicationReceipt"]["block"], inclusion.0);
        assert_eq!(
            accepted["publicationReceipt"]["blockHash"],
            format!("{:#x}", inclusion.1)
        );
        assert_eq!(
            accepted["publicationReceipt"]["finalizedBlock"],
            finalized.0
        );
        assert_eq!(signed_publication_identity(&accepted, queue)?.hash, hash);
        for (field, replacement) in [
            ("sender", json!(Address::from([5; 20]))),
            ("nonce", json!("5")),
            ("txHash", json!(B256::ZERO)),
        ] {
            let mut changed = accepted.clone();
            changed[field] = replacement;
            assert!(
                signed_publication_identity(&changed, queue).is_err(),
                "{field}"
            );
        }
        assert!(signed_publication_identity(&accepted, Address::from([6; 20])).is_err());
        assert!(persist_root_inclusion(&path, &mut intent, inclusion, "merkleRoot", None).is_err());
        let original_finalized = intent["publicationReceipt"].clone();
        persist_root_inclusion(
            &path,
            &mut intent,
            inclusion,
            "merkleRoot",
            Some((60, B256::repeat_byte(8))),
        )?;
        assert_eq!(intent["publicationReceipt"], original_finalized);
        assert!(persist_root_inclusion(
            &path,
            &mut intent,
            (42, inclusion.1),
            "merkleRoot",
            Some(finalized)
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn signed_root_anchor_or_proof_cannot_change_on_resume() {
        let expected = json!({
            "schemaVersion": 3, "sourceBlock": 21, "root": "0xroot", "kind": "merkleRoot",
            "localRehearsal": false, "sourceIdentity": {"genesisHash": "0xgenesis"},
            "acceptedAnchorTx": "0xoriginal", "acceptedAnchorClient": "0xclient",
            "acceptedCheckpoint": {"block": 25}, "sender": "0xpublisher",
            "proof": {"anchorBlock": 25}, "queueProof": "0xproof",
        });
        let mut saved = expected.clone();
        saved["nonce"] = json!("4");
        saved["rawTransaction"] = json!("0xsigned");
        saved["txHash"] = json!("0xoriginal-hash");
        assert!(ensure_immutable_publication(&saved, &expected).is_ok());
        for (field, changed) in [
            ("acceptedAnchorTx", json!("0xreanchored")),
            ("acceptedCheckpoint", json!({"block": 26})),
            ("proof", json!({"anchorBlock": 26})),
            ("queueProof", json!("0xreplaced")),
        ] {
            let mut recaptured = expected.clone();
            recaptured[field] = changed;
            assert!(
                ensure_immutable_publication(&saved, &recaptured).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn actor_and_maintenance_share_deployment_owner_across_journal_paths() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("deployment.json");
        let second = directory.path().join("same-deployment.json");
        let manifest = br#"{"ethereum":{"chainId":560048,"queue":"0x1111111111111111111111111111111111111111"}}"#;
        fs::write(&first, manifest)?;
        fs::write(&second, manifest)?;
        let owner = lock_token_deployment(&first)?;
        assert!(lock_token_deployment(&second).is_err());
        drop(owner);
        let maintenance = lock_token_deployment(&second)?;
        assert!(lock_token_deployment(&first).is_err());
        drop(maintenance);
        assert!(lock_token_deployment(&first).is_ok());
        Ok(())
    }

    #[test]
    fn publisher_uses_a_distinct_follower_account() {
        let follower = alloy::primitives::Address::from([1; 20]);
        let publisher = alloy::primitives::Address::from([2; 20]);
        assert!(ensure_separate_evm_signers(follower, publisher).is_ok());
        assert!(ensure_separate_evm_signers(follower, follower).is_err());
    }

    #[test]
    fn publisher_rejects_changed_genesis_or_bridge_domain() {
        let genesis = [1; 32];
        let domain = [2; 32];
        let manifest = json!({
            "sourceGenesis": format!("0x{}", hex::encode(genesis)),
            "bridgeDomain": format!("0x{}", hex::encode(domain)),
        });
        assert!(ensure_source_identity(&manifest, genesis, domain, genesis, domain).is_ok());
        assert!(ensure_source_identity(&manifest, [3; 32], domain, [3; 32], domain).is_err());
        assert!(ensure_source_identity(&manifest, genesis, domain, genesis, [3; 32]).is_err());
        let wrong_genesis = json!({
            "sourceGenesis": format!("0x{}", hex::encode([3; 32])),
            "bridgeDomain": manifest["bridgeDomain"],
        });
        assert!(ensure_source_identity(&wrong_genesis, genesis, domain, genesis, domain).is_err());
        let wrong_domain = json!({
            "sourceGenesis": manifest["sourceGenesis"],
            "bridgeDomain": format!("0x{}", hex::encode([3; 32])),
        });
        assert!(ensure_source_identity(&wrong_domain, genesis, domain, genesis, domain).is_err());
    }

    #[test]
    fn reorged_or_changed_source_anchor_is_rejected() {
        let raw = [4, 5, 6];
        let hash = [8; 32];
        let raw_text = format!("0x{}", hex::encode(raw));
        assert!(ensure_recaptured_anchor(
            20,
            (25, hash, &raw_text),
            (25, hash, &raw),
            (25, hash, &raw),
            &raw,
        )
        .is_ok());
        assert!(ensure_recaptured_anchor(
            20,
            (25, hash, &raw_text),
            (25, hash, &raw),
            (25, [9; 32], &raw),
            &raw,
        )
        .is_err());
        assert!(ensure_recaptured_anchor(
            20,
            (25, hash, &raw_text),
            (25, hash, &raw),
            (25, hash, &raw),
            &[1, 2, 3],
        )
        .is_err());
    }
    #[test]
    fn provisional_receipts_and_reorgs_do_not_complete() {
        let hash = B256::from([1; 32]);
        assert_eq!(
            classify_receipt_finality(41, hash, true, 40, Some(hash)).unwrap(),
            ReceiptFinality::Mined
        );
        assert!(classify_receipt_finality(41, hash, true, 41, Some(B256::from([2; 32]))).is_err());
        assert!(classify_receipt_finality(41, hash, true, 41, None).is_err());
        assert_eq!(
            classify_receipt_finality(41, hash, true, 41, Some(hash)).unwrap(),
            ReceiptFinality::Accepted
        );
        assert_eq!(
            classify_receipt_finality(41, hash, false, 41, Some(hash)).unwrap(),
            ReceiptFinality::Reverted
        );
    }

    #[test]
    fn ambiguous_send_reconciles_only_the_reserved_nonce() {
        let intent = json!({
            "nonce": "4",
            "txHash": null,
            "status": "ambiguous send; reconcile root state and saved nonce before retry",
        });
        let nonce = saved_publication_nonce(&intent).unwrap();
        assert_eq!(nonce, 4);
        assert!(ensure_saved_nonce_available(nonce, 4, 4).is_ok());
        assert!(ensure_saved_nonce_available(nonce, 5, 5).is_err());
        assert!(ensure_saved_nonce_available(nonce, 4, 5).is_err());
        assert_eq!(intent["nonce"], "4");
        assert!(intent["txHash"].is_null());
    }
}
