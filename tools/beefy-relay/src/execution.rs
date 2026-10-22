//! Independently approved write authority. RPC getters are diagnostics, never approval.
use alloy::{primitives::{Address, Bytes, B256, U256}, providers::Provider, rpc::types::BlockId};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};

pub const APPROVAL_ENV: &str = "BEEFY_APPROVED_MANIFEST_SHA256";

#[cfg(test)]
pub(crate) mod fixture;

/// Read-only pre-CREATE gate used by Foundry; stdout is only the approved digest.
pub async fn authorize_deployment(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    finality: &cli_utils::EthereumFinalityArgs,
    manifest: &Path,
    plan_hash: B256,
) -> Result<B256> {
    let approval = ExecutionAuthority::load(manifest, finality)?
        .context("fresh deployment has no independent execution approval; HOLD")?;
    ensure!(approval.manifest.execution.deployment_plan_hash == Some(plan_hash) && plan_hash != B256::ZERO,
        "deployment constructor/action plan differs from approved manifest; HOLD");
    let chain = approval.manifest.ethereum["chainId"].as_u64().context("approved chain missing; HOLD")?;
    ensure!(chain != 1, "fresh mainnet deployment forbidden; HOLD");
    if let Some(fixture) = &approval.manifest.owned_fixture_network {
        fixture.validate_endpoints(source_rpc, witness_rpc, ethereum_rpc)?;
    }
    ensure!(crate::local_source_rpc(ethereum_rpc) || ethereum_rpc.starts_with("https://") || ethereum_rpc.starts_with("wss://"),
        "remote deployment evidence requires HTTPS/WSS; HOLD");
    let config = ethereum_client::BeaconFinalityConfig::load(
        &finality.ethereum_finality_bootstrap,
        finality.ethereum_finality_bootstrap_sha256.parse()?,
        &finality.ethereum_finality_beacon_rpc,
    )?.with_proof_sources(finality.ethereum_finality_proof_sources.clone())?;
    ensure!(config.identity().0 == chain, "approved Beacon/deployment chain differs; HOLD");
    if let Some(fixture) = &approval.manifest.owned_fixture_network {
        ensure!(config.identity().1 == fixture.ethereum_genesis_hash, "owned fixture Beacon genesis differs; HOLD");
    }
    let api = ethereum_client::EthApi::new(ethereum_rpc,
        &format!("{:#x}", address(&approval.manifest.ethereum, "queue")?), None, None, None).await?
        .with_beacon_finality(config, &finality.ethereum_finality_store).await?;
    let (mut source, mut witness) = crate::source::Source::connect_pair(
        gear_rpc_client::GearApi::new(source_rpc, 3).await?,
        gear_rpc_client::GearApi::new(witness_rpc, 3).await?,
    ).await?;
    approval.verify_source(&source).await?;
    approval.verify_source(&witness).await?;
    crate::follower::authenticated_token_anchor(&mut source, &mut witness, &approval.manifest.anchor).await?;
    approval.verify_api(&api, true).await?;
    Ok(approval.digest)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    execution_authorized: bool,
    mode: String,
    local_rehearsal: bool,
    manager: Address,
    gear_manager: B256,
    anchor: Value,
    ethereum: Value,
    #[serde(default)]
    owned_fixture_network: Option<crate::ethereum::OwnedFixtureNetwork>,
    #[serde(default)]
    ethereum_configuration: Option<Value>,
    execution: Approval,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Approval {
    source_identity_report_sha256: B256,
    beacon_approval_sha256: B256,
    bootstrap_sha256: B256,
    token_inventory_sha256: B256,
    token_inventory: Vec<TokenIdentity>,
    authorities: BTreeMap<String, Authority>,
    recovery: Recovery,
    // These are independent release approvals, not results inferred from this tool.
    runtime_replay_safety_report_sha256: B256,
    runtime_heartbeat_report_sha256: B256,
    custody_authority_report_sha256: B256,
    gear_manager_code_id: B256,
    deployment_plan_hash: Option<B256>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TokenIdentity {
    actor_id: B256,
    code_id: B256,
    ethereum_token: Address,
    native_wrapper: Option<WrapperIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WrapperIdentity {
    mode: WrapperMode,
    idl_sha256: B256,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum WrapperMode { EscrowV1, HeldEscrowV2 }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Authority {
    address: Address,
    code_hash: B256,
    artifact_sha256: B256,
    runtime_code: Bytes,
    storage_layout_sha256: B256,
    // ponytail: a complete governance root also fences unrelated state changes.
    // Reapprove after any change; selective non-enumerable role proofs need a qualified layout.
    storage_root: Option<B256>,
    storage: BTreeMap<String, StorageWord>,
    implementation: Option<Implementation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StorageWord { slot: B256, value: B256, offset: u8, width: u8 }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Implementation { slot: B256, address: Address, code_hash: B256, artifact_sha256: B256, runtime_code: Bytes }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Recovery {
    address: Address,
    min_delay: u64,
    proposers: Vec<Address>,
    executors: Vec<Address>,
    admins: Vec<Address>,
    cancellers: Vec<Address>,
    // Independent review of exact runtime/layout and complete role storage.
    complete_state_report_sha256: B256,
    safes: Vec<Safe>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Safe {
    address: Address,
    singleton: Address,
    owners: Vec<Address>,
    threshold: u64,
    modules: Vec<Address>,
    guard: Address,
    complete_state_report_sha256: B256,
}

#[derive(Clone, Debug)]
pub(crate) struct ExecutionAuthority {
    pub(crate) digest: B256,
    manifest: Manifest,
}

fn sha256(bytes: &[u8]) -> B256 { B256::from(sp_core::hashing::sha2_256(bytes)) }
fn json_sha256(value: &impl Serialize) -> Result<B256> {
    Ok(sha256(&serde_json::to_vec(&serde_json::to_value(value)?)?))
}
fn nonzero(hash: B256, what: &str) -> Result<()> {
    ensure!(hash != B256::ZERO, "missing approved {what}; HOLD");
    Ok(())
}
fn unique_addresses(values: &[Address], what: &str, allow_empty: bool) -> Result<()> {
    ensure!((allow_empty || !values.is_empty()) && values.iter().all(|v| *v != Address::ZERO)
        && values.iter().collect::<BTreeSet<_>>().len() == values.len(),
        "invalid or incomplete approved {what}; HOLD");
    Ok(())
}
fn address(value: &Value, name: &str) -> Result<Address> {
    value[name].as_str().with_context(|| format!("missing {name}; HOLD"))?.parse().map_err(Into::into)
}
fn hash(value: &Value, name: &str) -> Result<B256> {
    value[name].as_str().with_context(|| format!("missing {name}; HOLD"))?.parse().map_err(Into::into)
}

impl ExecutionAuthority {
    pub(crate) fn load(path: &Path, finality: &cli_utils::EthereumFinalityArgs) -> Result<Option<Self>> {
        let approved = match std::env::var(APPROVAL_ENV) {
            Ok(value) => Some(value.parse::<B256>().context("invalid approved deployment SHA-256; HOLD")?),
            Err(std::env::VarError::NotPresent) => None,
            Err(error) => return Err(error.into()),
        };
        Self::parse(&fs::read(path)?, approved, finality.ethereum_finality_bootstrap_sha256.parse()?)
    }

    fn parse(raw: &[u8], approved: Option<B256>, beacon: B256) -> Result<Option<Self>> {
        let value = serde_json::from_slice::<UniqueJson>(raw)?.0;
        if let Some(approved) = approved {
            ensure!(sha256(raw) == approved, "deployment manifest differs from independently approved SHA-256; HOLD");
        }
        // Historical manifests remain readable, but can never acquire write authority.
        if value["schemaVersion"].as_u64() != Some(2) {
            ensure!(value["executionAuthorized"] != true, "legacy deployment schema cannot authorize execution; HOLD");
            return Ok(None);
        }
        let manifest: Manifest = serde_json::from_value(value).context("invalid strict deployment schema 2; HOLD")?;
        validate_shape(&manifest)?;
        if !manifest.execution_authorized || approved.is_none() { return Ok(None); }
        let authority = Self { digest: approved.expect("checked"), manifest };
        authority.validate(beacon)?;
        Ok(Some(authority))
    }

    fn validate(&self, beacon: B256) -> Result<()> {
        let m = &self.manifest;
        let a = &m.execution;
        if let Some(fixture) = &m.owned_fixture_network { fixture.validate()?; }
        validate_shape(m)?;
        let message_only = m.mode == "hoodi-message-stack";
        if message_only {
            ensure!(m.gear_manager == B256::ZERO
                && a.gear_manager_code_id == B256::ZERO && a.token_inventory.is_empty(),
                "message-only approval cannot acquire token/native-manager authority; HOLD");
        } else { nonzero(a.gear_manager_code_id, "native manager CodeId")?; }
        ensure!(m.schema_version == 2 && m.execution_authorized && (!m.local_rehearsal || m.owned_fixture_network.is_some())
            && matches!(m.mode.as_str(), "mainnet-token-stack" | "hoodi-token-stack" | "hoodi-message-stack"),
            "deployment is not authorized for this execution mode; HOLD");
        ensure!(m.manager == address(&m.ethereum, "receiver")? && (message_only || m.gear_manager != B256::ZERO),
            "approved manager identity differs; HOLD");
        for (digest, name) in [
            (a.source_identity_report_sha256, "source identity report"),
            (a.beacon_approval_sha256, "Beacon approval"),
            (a.bootstrap_sha256, "bootstrap"),
            (a.token_inventory_sha256, "token inventory"),
            (a.runtime_replay_safety_report_sha256, "runtime replay safety report"),
            (a.runtime_heartbeat_report_sha256, "runtime heartbeat report"),
            (a.custody_authority_report_sha256, "custody authority report"),
        ] { nonzero(digest, name)?; }
        ensure!(a.beacon_approval_sha256 == beacon && a.bootstrap_sha256 == json_sha256(&m.anchor)?
            && a.source_identity_report_sha256 == json_sha256(&m.anchor["sourceIdentity"])?
            && a.token_inventory_sha256 == json_sha256(&a.token_inventory)?,
            "approved source/bootstrap/Beacon/inventory binding differs; HOLD");
        let source = &m.anchor["sourceIdentity"];
        ensure!(source["sourceTimestampMs"].as_u64().is_some_and(|t| t > 0),
            "native source identity lacks its authenticated timestamp; HOLD");
        ensure!(source.is_object() && m.anchor["block"].as_u64().is_some_and(|b| b > 0)
            && m.anchor["sourceTimestampMs"].as_u64().is_some_and(|t| t > 0)
            && m.anchor["beefyActivationBlock"].as_u64().zip(m.anchor["mmrStartBlock"].as_u64()).is_some_and(|(first, start)| first >= start),
            "approved native source bootstrap is incomplete; HOLD");
        for field in ["sourceGenesis", "bridgeDomain", "mmrStartBlock"] {
            ensure!(m.anchor[field] == m.ethereum[field] && !m.anchor[field].is_null(), "approved source {field} differs; HOLD");
        }
        let chain = m.ethereum["chainId"].as_u64().context("missing approved chain; HOLD")?;
        let expected_chain = m.owned_fixture_network.as_ref().map_or(
            if m.mode == "mainnet-token-stack" { 1 } else { 560_048 }, |fixture| fixture.ethereum_chain_id);
        ensure!(chain == expected_chain, "approved deployment mode and destination chain differ; HOLD");
        ensure!(crate::bridge_domain(hash(&m.ethereum, "sourceDomain")?.0, chain, address(&m.ethereum, "queue")?.into())?
            == hash(&m.anchor, "bridgeDomain")?.0, "approved source/destination domain differs; HOLD");
        let bootstrap = &m.ethereum["checkpoint"];
        ensure!(bootstrap["block"] == m.anchor["block"] && bootstrap["sourceTimestampMs"] == m.anchor["sourceTimestampMs"],
            "approved initial block/timestamp differs; HOLD");
        for (set, prefix) in [("current", "current"), ("next", "next")] {
            ensure!(bootstrap[format!("{prefix}Id")] == m.anchor[set]["id"]
                && bootstrap[format!("{prefix}Length")].as_u64() == m.anchor[set]["keys"].as_array().map(|v| v.len() as u64)
                && hash(bootstrap, &format!("{prefix}Root"))?.as_slice() == decode_hash_array(&m.anchor[set]["root"])?,
                "approved bootstrap authority tuple differs; HOLD");
        }
        ensure!(message_only || !a.token_inventory.is_empty(), "complete unchanged token inventory required; HOLD");
        let mut tokens = BTreeSet::new();
        let mut peers = BTreeSet::new();
        for token in &a.token_inventory {
            nonzero(token.actor_id, "token ActorId")?;
            nonzero(token.code_id, "token CodeId")?;
            ensure!(token.ethereum_token != Address::ZERO && tokens.insert(token.actor_id) && peers.insert(token.ethereum_token),
                "duplicate or zero approved token identity; HOLD");
            if let Some(wrapper) = &token.native_wrapper { nonzero(wrapper.idl_sha256, "deployed wrapper IDL")?; }
        }
        for name in ["client", "verifier", "queue", "receiver"] {
            let authority = a.authorities.get(name).with_context(|| format!("missing {name} artifact approval; HOLD"))?;
            ensure!(authority.address == address(&m.ethereum, name)? && authority.code_hash == hash(&m.ethereum["bytecodeHashes"], name)?,
                "approved {name} code/address binding differs; HOLD");
            ensure!((name != "queue" && (name != "receiver" || message_only)) || authority.implementation.is_some(),
                "proxy implementation/layout approval is required; HOLD");
        }
        let r = &a.recovery;
        ensure!(r.address != Address::ZERO && r.address == address(&m.ethereum["recovery"], "address")?
            && r.min_delay > 0 && m.ethereum["recovery"]["minDelay"].as_str().and_then(|v| v.parse::<u64>().ok()) == Some(r.min_delay),
            "approved recovery address/delay differs; HOLD");
        nonzero(r.complete_state_report_sha256, "complete timelock state report")?;
        for (members, name, empty) in [(&r.proposers, "proposers", false), (&r.executors, "executors", false),
            (&r.admins, "admins", false), (&r.cancellers, "cancellers", true)] {
            unique_addresses(members, name, empty)?;
        }
        ensure!(r.admins == [r.address], "recovery timelock must be its sole administrator; HOLD");
        let timelock = a.authorities.get("timelock").context("missing approved timelock artifact; HOLD")?;
        ensure!(timelock.address == r.address && timelock.storage_root.is_some()
            && timelock.code_hash == hash(&m.ethereum["recovery"], "codeHash")?, "timelock complete state/code approval missing; HOLD");
        require_word(timelock, "minDelay", B256::from(U256::from(r.min_delay).to_be_bytes::<32>()))?;
        for (role, members) in [("proposer", &r.proposers), ("executor", &r.executors),
            ("admin", &r.admins), ("canceller", &r.cancellers)] {
            for member in members {
                require_word(timelock, &format!("{role}:{member:#x}"), B256::from(U256::from(1).to_be_bytes::<32>()))?;
            }
        }
        let mut safes = BTreeSet::new();
        for safe in &r.safes {
            ensure!(safes.insert(safe.address) && safe.singleton != Address::ZERO && safe.threshold > 0 && safe.threshold <= safe.owners.len() as u64,
                "invalid approved Safe singleton/threshold; HOLD");
            unique_addresses(&safe.owners, "Safe owners", false)?;
            unique_addresses(&safe.modules, "Safe modules", true)?;
            nonzero(safe.complete_state_report_sha256, "complete Safe state report")?;
            let identity = a.authorities.values().find(|v| v.address == safe.address).context("missing Safe authority proof; HOLD")?;
            ensure!(identity.storage_root.is_some() && identity.implementation.as_ref().is_some_and(|i| i.address == safe.singleton),
                "Safe complete storage/singleton approval missing; HOLD");
            require_word(identity, "threshold", B256::from(U256::from(safe.threshold).to_be_bytes::<32>()))?;
            require_word(identity, "ownerCount", B256::from(U256::from(safe.owners.len()).to_be_bytes::<32>()))?;
            require_word(identity, "guard", address_word(safe.guard))?;
            let sentinel = Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
            for (label, members) in [("owner", &safe.owners), ("module", &safe.modules)] {
                let mut previous = sentinel;
                for member in members.iter().copied().chain(std::iter::once(sentinel)) {
                    require_word(identity, &format!("{label}:{previous:#x}"), address_word(member))?;
                    previous = member;
                }
            }
        }
        for safe in &r.safes {
            for member in safe.modules.iter().copied().chain((safe.guard != Address::ZERO).then_some(safe.guard)) {
                ensure!(a.authorities.values().any(|identity| identity.address == member),
                    "Safe module/guard lacks an approved code authority; HOLD");
            }
        }
        ensure!(r.proposers.iter().chain(&r.executors).chain(&r.cancellers).all(|v| safes.contains(v)),
            "recovery roles require independently approved Safe authorities, not hot keys; HOLD");
        let mut addresses = BTreeSet::new();
        for authority in a.authorities.values() {
            ensure!(authority.address != Address::ZERO && addresses.insert(authority.address), "duplicate/zero authority address; HOLD");
            nonzero(authority.code_hash, "authority code hash")?;
            nonzero(authority.artifact_sha256, "authority artifact")?;
            nonzero(authority.storage_layout_sha256, "authority storage layout")?;
            ensure!(!authority.runtime_code.is_empty()
                && sha256(&authority.runtime_code) == authority.artifact_sha256
                && alloy::primitives::keccak256(&authority.runtime_code) == authority.code_hash,
                "approved runtime artifact preimage differs; HOLD");
            let layout: BTreeMap<_, _> = authority.storage.iter()
                .map(|(name, word)| (name, (word.slot, word.offset, word.width))).collect();
            ensure!(json_sha256(&(layout, authority.implementation.as_ref().map(|i| i.slot)))? == authority.storage_layout_sha256,
                "approved storage layout preimage differs; HOLD");
            for word in authority.storage.values() { validate_storage_word(word)?; }
            if let Some(root) = authority.storage_root { nonzero(root, "complete authority storage root")?; }
            if let Some(implementation) = &authority.implementation {
                ensure!(implementation.address != Address::ZERO, "zero approved implementation; HOLD");
                nonzero(implementation.code_hash, "implementation code hash")?;
                nonzero(implementation.artifact_sha256, "implementation artifact")?;
                ensure!(!implementation.runtime_code.is_empty()
                    && sha256(&implementation.runtime_code) == implementation.artifact_sha256
                    && alloy::primitives::keccak256(&implementation.runtime_code) == implementation.code_hash,
                    "approved implementation artifact preimage differs; HOLD");
            }
        }
        // The approved layout selects slots. Their values are not learned from eth_call.
        let governance = a.authorities.get("governanceAdmin").context("missing approved governance admin artifact; HOLD")?;
        let pauser = a.authorities.get("governancePauser").context("missing approved governance pauser artifact; HOLD")?;
        for authority in [governance, pauser] {
            nonzero(authority.storage.get("governance").context("missing approved native governance storage; HOLD")?.value,
                "native governance authority")?;
        }
        let manager = if message_only {
            if let Some(manager) = a.authorities.get("erc20Manager") {
                ensure!(manager.implementation.is_some(), "auxiliary EVM manager needs its approved proxy implementation; HOLD");
                for (field, expected) in [("messageQueue", address(&m.ethereum, "queue")?),
                    ("governanceAdmin", governance.address), ("governancePauser", pauser.address)] {
                    require_word(manager, field, address_word(expected))?;
                }
                manager.address
            } else { Address::ZERO }
        } else { m.manager };
        for (name, field, expected) in [("queue", "verifier", address(&m.ethereum, "verifier")?),
            ("queue", "governanceAdmin", governance.address), ("queue", "beefyRecoveryTimelock", r.address),
            ("queue", "governancePauser", pauser.address),
            ("receiver", "messageQueue", address(&m.ethereum, "queue")?),
            ("receiver", "governanceAdmin", governance.address),
            ("receiver", "governancePauser", pauser.address),
            ("governancePauser", "erc20Manager", manager),
            ("governancePauser", "messageQueue", address(&m.ethereum, "queue")?),
            ("governanceAdmin", "erc20Manager", manager),
            ("governanceAdmin", "messageQueue", address(&m.ethereum, "queue")?)] {
            if message_only && name == "receiver" { continue; }
            let word = a.authorities[name].storage.get(field).with_context(|| format!("missing approved {name}.{field} layout slot; HOLD"))?;
            ensure!(word.value == address_word(expected), "approved {name}.{field} storage identity differs; HOLD");
        }
        Ok(())
    }

    pub(crate) fn matches_ethereum(&self, ethereum: &crate::ethereum::Ethereum) -> Result<()> {
        for (name, actual) in [("client", ethereum.client_address), ("verifier", ethereum.verifier_address),
            ("queue", ethereum.queue_address), ("receiver", ethereum.receiver_address)] {
            ensure!(address(&self.manifest.ethereum, name)? == actual, "write authority changed {name}; HOLD");
        }
        ensure!(self.manifest.ethereum["chainId"].as_u64() == Some(ethereum.api.finality_identity()?.0), "write authority changed chain; HOLD");
        if let Some(fixture) = &self.manifest.owned_fixture_network {
            ensure!(ethereum.api.finality_identity()?.1 == fixture.ethereum_genesis_hash,
                "owned fixture execution genesis changed; HOLD");
        }
        Ok(())
    }
    pub(crate) async fn verify_source(&self, source: &crate::source::Source) -> Result<()> {
        source.validate_attachment(&self.manifest.anchor).await?;
        ensure!(source.source_genesis == hash(&self.manifest.anchor, "sourceGenesis")?.0
            && source.bridge_domain == hash(&self.manifest.anchor, "bridgeDomain")?.0
            && Some(source.mmr_start_block) == self.manifest.anchor["mmrStartBlock"].as_u64(),
            "approved source identity changed; HOLD");
        let at = gsdk::ext::subxt::utils::H256(source.identity.finalized_hash);
        if self.manifest.gear_manager != B256::ZERO {
            let manager = source.api.api.gprog_at(self.manifest.gear_manager.0.into(), Some(at)).await?;
            ensure!(matches!(manager.state, gsdk::ext::gear_core::program::ProgramState::Initialized)
                && manager.code_id.into_bytes() == self.manifest.execution.gear_manager_code_id.0,
                "deployed native manager differs from independently approved CodeId; HOLD");
        }
        for token in &self.manifest.execution.token_inventory {
            let program = source.api.api.gprog_at(token.actor_id.0.into(), Some(at)).await?;
            ensure!(matches!(program.state, gsdk::ext::gear_core::program::ProgramState::Initialized)
                && program.code_id.into_bytes() == token.code_id.0,
                "deployed token/wrapper CodeId differs from unchanged approved inventory; HOLD");
        }
        Ok(())
    }

    pub(crate) async fn verify(&self, ethereum: &crate::ethereum::Ethereum) -> Result<()> {
        self.matches_ethereum(ethereum)?;
        self.verify_api(&ethereum.api, false).await
    }

    async fn verify_api(&self, api: &ethereum_client::EthApi, fresh_deployment: bool) -> Result<()> {
        let provider = api.raw_provider();
        let view = api.verified_finalized_view().await?;
        let header = view.checkpoint().common_header().clone();
        for (name, authority) in &self.manifest.execution.authorities {
            if fresh_deployment && matches!(name.as_str(), "client" | "verifier" | "queue" | "receiver" | "governanceAdmin" | "governancePauser") {
                let proof = ethereum_client::inclusion::prove_account(provider, &view, header.clone(), authority.address).await?;
                ensure!(proof.verify(&view, authority.address)?.is_none(), "planned fresh authority address already exists; HOLD");
                continue;
            }
            let account = authenticate_code(provider, &view, header.clone(), authority.address, authority.code_hash).await?;
            if let Some(root) = authority.storage_root {
                ensure!(account.storage_root == root, "approved complete governance state changed; HOLD");
            }
            let mut words: Vec<_> = authority.storage.values().cloned().collect();
            if let Some(implementation) = &authority.implementation {
                words.push(StorageWord { slot: implementation.slot, value: address_word(implementation.address), offset: 0, width: 20 });
                authenticate_code(provider, &view, header.clone(), implementation.address, implementation.code_hash).await?;
            }
            if !words.is_empty() {
                let keys = words.iter().map(|word| word.slot).collect();
                let response = tokio::time::timeout(std::time::Duration::from_secs(10), provider.get_proof(authority.address, keys)
                    .block_id(BlockId::hash_canonical(view.block_hash()))).await.context("authority storage request timed out; HOLD")??;
                ensure!(response.address == authority.address && response.storage_proof.len() == words.len(), "authority storage response changed identity; HOLD");

                for word in &words {
                    let proof = response.storage_proof.iter().find(|p| p.key.as_b256() == word.slot)
                        .context("required authority storage proof missing; HOLD")?;
                    verify_storage(account.storage_root, word, &proof.proof.iter().map(|p| p.to_vec()).collect::<Vec<_>>())?;
                }
            }
        }
        view.recheck(provider).await?;
        Ok(())
    }
}

fn decode_hash_array(value: &Value) -> Result<[u8; 32]> {
    serde_json::from_value(value.clone()).context("invalid native source hash; HOLD")
}
fn address_word(address: Address) -> B256 {
    let mut word = [0; 32]; word[12..].copy_from_slice(address.as_slice()); B256::from(word)
}

pub(crate) async fn authenticate_code(provider: &impl Provider, view: &ethereum_client::FinalizedView,
    header: alloy::consensus::Header, address: Address, expected: B256) -> Result<alloy::consensus::TrieAccount> {
    let proof = ethereum_client::inclusion::prove_account(provider, view, header.clone(), address).await?;
    let account = proof.verify(view, address)?.context("authority account absent; HOLD")?;
    ensure!(account.code_hash == expected, "authority code differs from independently approved artifact; HOLD");
    let code = tokio::time::timeout(std::time::Duration::from_secs(10), provider.get_code_at(address)
        .block_id(BlockId::hash_canonical(header.hash_slow()))).await.context("authority code request timed out; HOLD")??;
    ensure!(!code.is_empty() && alloy::primitives::keccak256(&code) == account.code_hash,
        "authority raw code differs from proven account; HOLD");
    Ok(account)
}
fn require_word(authority: &Authority, name: &str, expected: B256) -> Result<()> {
    ensure!(authority.storage.get(name).is_some_and(|word| word.value == expected),
        "approved artifact layout lacks exact {name} authority state; HOLD");
    Ok(())
}

fn validate_storage_word(word: &StorageWord) -> Result<()> {
    ensure!(word.width > 0 && word.width <= 32 && u16::from(word.offset) + u16::from(word.width) <= 32,
        "invalid approved storage packing; HOLD");
    ensure!(word.width == 32 || U256::from_be_slice(word.value.as_slice()) >> (usize::from(word.width) * 8) == U256::ZERO,
        "approved storage value exceeds its field width; HOLD");
    Ok(())
}

fn verify_storage(root: B256, word: &StorageWord, proof: &[Vec<u8>]) -> Result<()> {
    use ethereum_common::{hash_db, memory_db, patricia_trie::TrieDB, trie_db::{HashDB, Trie}};
    use alloy::rlp::Decodable;
    validate_storage_word(word)?;
    let mut database = memory_db::new();
    for node in proof { database.insert(hash_db::EMPTY_PREFIX, node); }
    let root = ethereum_common::H256(root.0);
    let trie = TrieDB::new(&database, &root).map_err(|_| anyhow::anyhow!("invalid authority storage trie; HOLD"))?;
    let value = trie.get(alloy::primitives::keccak256(word.slot).as_slice())
        .map_err(|_| anyhow::anyhow!("incomplete authority storage proof; HOLD"))?;

    let actual = match value {
        None => U256::ZERO,
        Some(value) => {
            let mut input = value.as_slice();
            let decoded = U256::decode(&mut input)?;
            ensure!(input.is_empty() && alloy::rlp::encode(decoded) == value, "noncanonical authority storage value; HOLD");
            decoded
        }
    };
    let shifted = actual >> (usize::from(word.offset) * 8);
    let mask = U256::MAX >> ((32 - usize::from(word.width)) * 8);
    ensure!((shifted & mask) == U256::from_be_slice(word.value.as_slice()), "proven authority storage differs from approved layout/value; HOLD");
    Ok(())
}

fn exact_fields(value: &Value, fields: &[&str]) -> Result<()> {
    let object = value.as_object().context("required deployment object missing; HOLD")?;
    ensure!(object.len() == fields.len() && fields.iter().all(|key| object.contains_key(*key)),
        "deployment schema has missing or unknown fields; HOLD");
    Ok(())
}

fn validate_shape(m: &Manifest) -> Result<()> {
    exact_fields(&m.ethereum, &["chainId", "client", "verifier", "queue", "receiver", "recovery",
        "bindings", "sourceGenesis", "sourceDomain", "bridgeDomain", "mmrStartBlock", "checkpoint", "policy", "bytecodeHashes"])?;
    exact_fields(&m.anchor, &["sourceGenesis", "bridgeDomain", "mmrStartBlock", "beefyActivationBlock",
        "domainBindingBlock", "sourceIdentity", "block", "blockHash", "sourceTimestampMs", "freshnessSourceBlock",
        "mmrRoot", "signedCommitmentScale", "current", "next"])?;
    exact_fields(&m.ethereum["bindings"], &["beefyClient", "messageQueue", "destinationChainId"])?;
    exact_fields(&m.ethereum["recovery"], &["address", "codeHash", "minDelay"])?;
    exact_fields(&m.ethereum["bytecodeHashes"], &["client", "verifier", "queue", "receiver"])?;
    exact_fields(&m.ethereum["checkpoint"], &["block", "root", "sourceTimestampMs", "currentId", "currentLength",
        "currentRoot", "nextId", "nextLength", "nextRoot"])?;
    exact_fields(&m.ethereum["policy"], &["randaoCommitDelay", "randaoCommitExpiration", "minNumRequiredSignatures",
        "fiatShamirRequiredSignatures", "maxValidators", "maxSourceAgeMs", "maxFutureSourceSkewMs", "isLive"])
}

// Values retained from historical manifests must not hide duplicate nested keys.
struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("JSON without duplicate keys") }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<UniqueJson,E> { Ok(UniqueJson(v.into())) }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<UniqueJson,E> { Ok(UniqueJson(v.into())) }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<UniqueJson,E> { Ok(UniqueJson(v.into())) }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<UniqueJson,E> {
                serde_json::Number::from_f64(v).map(|v| UniqueJson(Value::Number(v))).ok_or_else(|| E::custom("nonfinite JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<UniqueJson,E> { Ok(UniqueJson(v.into())) }
            fn visit_string<E: serde::de::Error>(self, v: String) -> std::result::Result<UniqueJson,E> { Ok(UniqueJson(v.into())) }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<UniqueJson,E> { Ok(UniqueJson(Value::Null)) }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<UniqueJson,A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueJson>()? { values.push(value.0); }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<UniqueJson,A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, UniqueJson>()? {
                    if values.insert(key, value.0).is_some() { return Err(serde::de::Error::custom("duplicate JSON key")); }
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn approved_manifest() -> Result<Value> {
        use serde_json::json;
        let beacon = B256::repeat_byte(9);
        let genesis = B256::repeat_byte(8);
        let queue = Address::repeat_byte(3);
        let domain = B256::from(crate::bridge_domain(genesis.0, 560_048, queue.into())?);
        let current = json!({"id":1,"keys":[vec![2u8;33]],"root":vec![4u8;32]});
        let next = json!({"id":2,"keys":[vec![3u8;33]],"root":vec![5u8;32]});
        let anchor = json!({"sourceGenesis":genesis,"bridgeDomain":domain,"mmrStartBlock":10,
            "beefyActivationBlock":12,"domainBindingBlock":12,"sourceIdentity":{"sourceTimestampMs":1000},
            "block":20,"blockHash":B256::repeat_byte(1),"sourceTimestampMs":1000,"freshnessSourceBlock":19,
            "mmrRoot":B256::repeat_byte(2),"signedCommitmentScale":"0x01","current":current,"next":next});
        let mut identities = BTreeMap::new();
        for (name, byte) in [("client",1),("verifier",2),("queue",3),("receiver",4),("timelock",5),("safe",6),("governanceAdmin",7),("governancePauser",8)] {
            identities.insert(name.to_owned(), Authority {
                address: Address::repeat_byte(byte), code_hash: alloy::primitives::keccak256([0x60,0]),
                artifact_sha256: sha256(&[0x60,0]), runtime_code: vec![0x60,0].into(),
                storage_layout_sha256: B256::ZERO, storage_root: Some(B256::repeat_byte(byte)),
                storage: BTreeMap::new(), implementation: None,
            });
        }
        let mut slot = 0u64;
        let mut set = |name: &str, field: String, value: B256| {
            slot += 1;
            identities.get_mut(name).unwrap().storage.insert(field, StorageWord {
                slot: B256::from(U256::from(slot).to_be_bytes::<32>()), value, offset:0, width:32,
            });
        };
        for (contract, field, byte) in [("queue","verifier",2),("queue","governanceAdmin",7),
            ("queue","beefyRecoveryTimelock",5),("receiver","messageQueue",3),("receiver","governanceAdmin",7),
            ("governanceAdmin","erc20Manager",4),("governanceAdmin","messageQueue",3),
            ("queue","governancePauser",8),("receiver","governancePauser",8),
            ("governancePauser","erc20Manager",4),("governancePauser","messageQueue",3)] {
            set(contract, field.to_owned(), address_word(Address::repeat_byte(byte)));
        }
        let one = B256::from(U256::from(1).to_be_bytes::<32>());
        let safe = Address::repeat_byte(6);
        let timelock = Address::repeat_byte(5);
        let owner = Address::repeat_byte(10);
        let sentinel = Address::from_slice(&one.as_slice()[12..]);
        set("timelock", "minDelay".into(), B256::from(U256::from(3600).to_be_bytes::<32>()));
        for (role, member) in [("proposer", safe), ("executor", safe), ("admin", timelock)] {
            set("timelock", format!("{role}:{member:#x}"), one);
        }
        set("safe", "threshold".into(), one);
        set("safe", "ownerCount".into(), one);
        set("safe", "guard".into(), B256::ZERO);
        set("safe", format!("owner:{sentinel:#x}"), address_word(owner));
        set("safe", format!("owner:{owner:#x}"), address_word(sentinel));
        set("safe", format!("module:{sentinel:#x}"), address_word(sentinel));
        set("governanceAdmin", "governance".into(), B256::repeat_byte(20));
        set("governancePauser", "governance".into(), B256::repeat_byte(21));
        identities.get_mut("safe").unwrap().implementation = Some(Implementation {
            slot:B256::ZERO, address:Address::repeat_byte(11), code_hash:alloy::primitives::keccak256([0x60,0]),
            artifact_sha256:sha256(&[0x60,0]), runtime_code:vec![0x60,0].into(),
        });
        for (name, byte) in [("queue",12),("receiver",13)] {
            identities.get_mut(name).unwrap().implementation = Some(Implementation {
                slot:B256::repeat_byte(50), address:Address::repeat_byte(byte), code_hash:alloy::primitives::keccak256([0x60,0]),
                artifact_sha256:sha256(&[0x60,0]), runtime_code:vec![0x60,0].into(),
            });
        }
        for authority in identities.values_mut() {
            let layout: BTreeMap<_,_> = authority.storage.iter().map(|(name, word)| (name,(word.slot,word.offset,word.width))).collect();
            authority.storage_layout_sha256 = json_sha256(&(layout, authority.implementation.as_ref().map(|i| i.slot)))?;
        }
        let inventory = vec![TokenIdentity { actor_id:B256::repeat_byte(12), code_id:B256::repeat_byte(13),
            ethereum_token:Address::repeat_byte(12), native_wrapper:Some(WrapperIdentity { mode:WrapperMode::EscrowV1, idl_sha256:B256::repeat_byte(14) }) }];
        Ok(json!({
            "schemaVersion":2,"executionAuthorized":true,"mode":"hoodi-token-stack","localRehearsal":false,
            "manager":Address::repeat_byte(4),"gearManager":B256::repeat_byte(7),"anchor":anchor,
            "ethereum":{"chainId":560048,"client":Address::repeat_byte(1),"verifier":Address::repeat_byte(2),
                "queue":queue,"receiver":Address::repeat_byte(4),"sourceGenesis":genesis,"sourceDomain":genesis,
                "bridgeDomain":domain,"mmrStartBlock":10,
                "bindings":{"beefyClient":Address::repeat_byte(1),"messageQueue":queue,"destinationChainId":"560048"},
                "recovery":{"address":timelock,"codeHash":identities["timelock"].code_hash,"minDelay":"3600"},
                "bytecodeHashes":{"client":identities["client"].code_hash,"verifier":identities["verifier"].code_hash,
                    "queue":identities["queue"].code_hash,"receiver":identities["receiver"].code_hash},
                "checkpoint":{"block":20,"root":B256::ZERO,"sourceTimestampMs":1000,"currentId":1,"currentLength":1,
                    "currentRoot":B256::repeat_byte(4),"nextId":2,"nextLength":1,"nextRoot":B256::repeat_byte(5)},
                "policy":{"randaoCommitDelay":"1","randaoCommitExpiration":"2","minNumRequiredSignatures":"86",
                    "fiatShamirRequiredSignatures":"86","maxValidators":"256","maxSourceAgeMs":"86400000",
                    "maxFutureSourceSkewMs":"120000","isLive":false}},
            "execution":{"sourceIdentityReportSha256":json_sha256(&anchor["sourceIdentity"])?,
                "bootstrapSha256":json_sha256(&anchor)?,"beaconApprovalSha256":beacon,
                "tokenInventorySha256":json_sha256(&inventory)?,"tokenInventory":inventory,"authorities":identities,
                "recovery":{"address":timelock,"minDelay":3600,"proposers":[safe],"executors":[safe],"admins":[timelock],
                    "cancellers":[],"completeStateReportSha256":beacon,"safes":[{"address":safe,"singleton":Address::repeat_byte(11),
                        "owners":[owner],"threshold":1,"modules":[],"guard":Address::ZERO,"completeStateReportSha256":beacon}]},
                "runtimeReplaySafetyReportSha256":beacon,"runtimeHeartbeatReportSha256":beacon,"custodyAuthorityReportSha256":beacon,
                "gearManagerCodeId":beacon,"deploymentPlanHash":null}
        }))
    }

    #[test]
    fn approval_binds_source_inventory_roles_artifacts_and_exact_file_bytes() -> Result<()> {
        use serde_json::json;
        let manifest = approved_manifest()?;
        let raw = serde_json::to_vec(&manifest)?;
        assert!(ExecutionAuthority::parse(&raw, Some(sha256(&raw)), B256::repeat_byte(9))?.is_some());
        assert!(ExecutionAuthority::parse(&raw, None, B256::repeat_byte(9))?.is_none());
        for (pointer, value) in [
            ("/execution/recovery/proposers/0",json!(Address::repeat_byte(99))),
            ("/execution/recovery/safes/0/threshold",json!(2)),
            ("/execution/recovery/safes/0/guard",json!(Address::repeat_byte(99))),
            ("/execution/authorities/verifier/runtimeCode",json!("0x6001")),
            ("/execution/authorities/queue/storage/verifier/slot",json!(B256::repeat_byte(99))),
            ("/execution/tokenInventory/0/nativeWrapper/mode",json!("held-escrow-v2")),
            ("/anchor/sourceTimestampMs",json!(1001)),
            ("/ethereum/checkpoint/nextId",json!(3)),
        ] {
            let mut changed = manifest.clone();
            *changed.pointer_mut(pointer).context("fixture pointer missing")? = value;
            let raw = serde_json::to_vec(&changed)?;
            assert!(ExecutionAuthority::parse(&raw, Some(sha256(&raw)), B256::repeat_byte(9)).is_err(), "{pointer}");
        }
        let mut changed = manifest.clone();
        changed["executionAuthorized"] = json!(false);
        let raw_false = serde_json::to_vec(&changed)?;
        assert!(ExecutionAuthority::parse(&raw_false, Some(sha256(&raw_false)), B256::repeat_byte(9))?.is_none());
        let mut extra = manifest;
        extra["ethereum"]["expectedQueueId"] = json!(4);
        let raw_extra = serde_json::to_vec(&extra)?;
        assert!(ExecutionAuthority::parse(&raw_extra, Some(sha256(&raw_extra)), B256::repeat_byte(9)).is_err());
        assert!(ExecutionAuthority::parse(&raw_false, Some(sha256(&raw)), B256::repeat_byte(9)).is_err());
        let duplicate = String::from_utf8(raw)?.replacen("\"chainId\":560048", "\"chainId\":1,\"chainId\":560048", 1);
        assert!(ExecutionAuthority::parse(duplicate.as_bytes(), Some(sha256(duplicate.as_bytes())), B256::repeat_byte(9)).is_err());
        Ok(())
    }
    #[test]
    fn legacy_or_unapproved_manifest_never_authorizes_writes() -> Result<()> {
        let legacy = br#"{"mode":"hoodi-token-stack"}"#;
        assert!(ExecutionAuthority::parse(legacy, None, B256::ZERO)?.is_none());
        assert!(ExecutionAuthority::parse(legacy, Some(sha256(legacy)), B256::ZERO)?.is_none());
        assert!(ExecutionAuthority::parse(legacy, Some(B256::repeat_byte(1)), B256::ZERO).is_err());
        assert!(ExecutionAuthority::parse(br#"{"executionAuthorized":true}"#, None, B256::ZERO).is_err());
        assert!(ExecutionAuthority::parse(br#"{"schemaVersion":2,"executionAuthorized":true,"unknown":1}"#, None, B256::ZERO).is_err());
        Ok(())
    }
    #[test]
    fn storage_proof_requires_exact_key_value_and_complete_nodes() -> Result<()> {
        use ethereum_common::{memory_db, patricia_trie::TrieDBMut, trie_db::TrieMut};
        let mut db = memory_db::new();
        let mut root = ethereum_common::H256::zero();
        let slot = B256::repeat_byte(4);
        let value = U256::from(27);
        {
            let mut trie = TrieDBMut::new(&mut db, &mut root);
            trie.insert(alloy::primitives::keccak256(slot).as_slice(), &alloy::rlp::encode(value)).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        use ethereum_common::{patricia_trie::TrieDB, trie_db::{Recorder, Trie}};
        let mut recorder = Recorder::new();
        TrieDB::new(&db, &root).map_err(|e| anyhow::anyhow!("{e:?}"))?
            .get_with(alloy::primitives::keccak256(slot).as_slice(), &mut recorder).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let nodes: Vec<_> = recorder.drain().into_iter().map(|n| n.data).collect();
        let word = StorageWord { slot, value: B256::from(value.to_be_bytes::<32>()), offset: 0, width: 32 };
        verify_storage(B256::from(root.0), &word, &nodes)?;
        assert!(verify_storage(B256::from(root.0), &StorageWord { value: B256::repeat_byte(1), ..word.clone() }, &nodes).is_err());
        assert!(verify_storage(B256::from(root.0), &word, &[]).is_err());
        Ok(())
    }
}
