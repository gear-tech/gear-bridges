//! Local-only fixture approvals: real native/Beacon/MPT evidence, never production authorization.
use super::*;
use crate::{ethereum::{Ethereum, OwnedFixtureNetwork}, source::{CapturedCommitment, Source, SourceProof}};
use alloy::primitives::keccak256;
use gear_common::durable::{committed_bytes, immutable_json};
use serde_json::json;
use std::path::PathBuf;

pub(crate) struct FixtureTopology {
    pub governance_admin: Address,
    pub governance_pauser: Address,
    pub evm_manager: Option<Address>,
    pub recovery: Address,
    pub safe: Address,
    pub singleton: Address,
    /// Linked-list order, not sorted address order.
    pub safe_owners: Vec<Address>,
    pub min_delay: u64,
    pub native: Option<NativeInventory>,
    pub owned_network: OwnedFixtureNetwork,
    pub beacon_approval_sha256: B256,
    pub beacon_approval_path: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeInventory {
    pub manager: B256,
    pub manager_code_id: B256,
    pub tokens: Vec<NativeToken>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeToken {
    pub actor_id: B256,
    pub code_id: B256,
    pub ethereum_token: Address,
    pub native_wrapper: Option<NativeWrapper>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeWrapper {
    pub mode: NativeWrapperMode,
    pub idl_sha256: B256,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NativeWrapperMode { EscrowV1, HeldEscrowV2 }

fn slot(value: u64) -> B256 { B256::from(U256::from(value).to_be_bytes::<32>()) }
fn mapping(key: B256, base: B256) -> B256 {
    let mut preimage = [0; 64];
    preimage[..32].copy_from_slice(key.as_slice());
    preimage[32..].copy_from_slice(base.as_slice());
    keccak256(preimage)
}
fn word(slot: B256, offset: u8, width: u8) -> StorageWord {
    StorageWord { slot, offset, width, value: B256::ZERO }
}

/// Rebuild the entire freshly initialized governance trie, not just selected getters.
/// Any unlisted role, module, approved hash, queued operation, or storage mutation fails.
fn require_complete_initial_state(authority: &Authority) -> Result<()> {
    use ethereum_common::{memory_db, patricia_trie::TrieDBMut, trie_db::TrieMut};
    let mut words = BTreeMap::<B256, U256>::new();
    for word in authority.storage.values() {
        *words.entry(word.slot).or_default() |=
            U256::from_be_slice(word.value.as_slice()) << (usize::from(word.offset) * 8);
    }
    if let Some(implementation) = &authority.implementation {
        words.insert(implementation.slot, U256::from_be_slice(implementation.address.as_slice()));
    }
    let mut database = memory_db::new();
    let mut root = ethereum_common::H256::zero();
    {
        let mut trie = TrieDBMut::new(&mut database, &mut root);
        for (slot, value) in words {
            if value != U256::ZERO {
                trie.insert(keccak256(slot).as_slice(), &alloy::rlp::encode(value))
                    .map_err(|error| anyhow::anyhow!("fixture governance trie reconstruction failed: {error:?}"))?;
            }
        }
        let _ = trie.root();
    }
    ensure!(authority.storage_root == Some(B256::from(root.0)),
        "fixture governance has additional/unapproved state outside its complete initialized layout; HOLD");
    Ok(())
}

struct Artifact { name: String, raw: Vec<u8>, value: Value }
impl Artifact {
    fn compiled(name: &str) -> Result<Self> {
        ensure!(matches!(name, "BeefyClient" | "VaraQueueRootVerifier" | "MessageQueue" |
            "ERC20Manager" | "ERC1967Proxy" | "MessageHandlerMock" | "GovernanceAdmin" |
            "GovernancePauser" | "TimelockController"), "unknown fixture artifact; HOLD");
        Self::read(name, &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ethereum/out")
            .join(format!("{name}.sol/{name}.json")))
    }
    fn read(name: &str, path: &Path) -> Result<Self> {
        let raw = committed_bytes(path)?;
        let value = serde_json::from_slice::<UniqueJson>(&raw)?.0;
        Ok(Self { name: name.into(), raw, value })
    }
    fn field(&self, label: &str) -> Result<StorageWord> {
        let layout = &self.value["storageLayout"];
        let field = layout["storage"].as_array().context("full artifact storageLayout missing; HOLD")?
            .iter().find(|entry| entry["label"] == label)
            .with_context(|| format!("{} storage field {label} missing; HOLD", self.name))?;
        let kind = &layout["types"][field["type"].as_str().context("storage type missing")?];
        ensure!(kind["encoding"] == "inplace", "fixture scalar layout is not inplace; HOLD");
        Ok(word(B256::from(U256::from_str_radix(field["slot"].as_str().context("slot missing")?, 10)?.to_be_bytes::<32>()),
            field["offset"].as_u64().context("offset missing")?.try_into()?,
            kind["numberOfBytes"].as_str().context("width missing")?.parse()?))
    }
    fn fields(&self, labels: &[(&str, &str)]) -> Result<BTreeMap<String, StorageWord>> {
        labels.iter().map(|(name, label)| Ok(((*name).into(), self.field(label)?))).collect()
    }
    fn runtime(&self, address: Address, expected: &BTreeMap<String, B256>) -> Result<Vec<u8>> {
        let deployed = &self.value["deployedBytecode"];
        let encoded = deployed.as_str().or_else(|| deployed["object"].as_str()).context("runtime artifact missing; HOLD")?;
        let mut runtime = hex::decode(encoded.strip_prefix("0x").unwrap_or(encoded))?;
        ensure!(!runtime.is_empty(), "empty runtime artifact; HOLD");
        if let Some(references) = deployed["immutableReferences"].as_object() {
            fn declaration(value: &Value, id: u64) -> Option<&str> {
                if value["id"].as_u64() == Some(id) && value["nodeType"] == "VariableDeclaration" {
                    return value["name"].as_str();
                }
                match value {
                    Value::Object(object) => object.values().find_map(|v| declaration(v, id)),
                    Value::Array(array) => array.iter().find_map(|v| declaration(v, id)),
                    _ => None,
                }
            }
            for (id, ranges) in references {
                let value = if matches!(self.name.as_str(), "MessageQueue" | "ERC20Manager") {
                    ensure!(references.len() == 1, "unknown UUPS immutable layout; HOLD");
                    address_word(address) // OpenZeppelin UUPSUpgradeable.__self.
                } else {
                    let name = declaration(&self.value["ast"], id.parse()?).context("unknown immutable declaration; HOLD")?;
                    *expected.get(name).with_context(|| format!("unknown immutable {name}; HOLD"))?
                };
                for range in ranges.as_array().context("immutable ranges missing")? {
                    let start: usize = range["start"].as_u64().context("immutable start missing")?.try_into()?;
                    ensure!(range["length"] == 32, "unsupported immutable width; HOLD");
                    runtime.get_mut(start..start.checked_add(32).context("immutable overflow")?)
                        .context("immutable outside runtime; HOLD")?.copy_from_slice(value.as_slice());
                }
            }
        }
        Ok(runtime)
    }
}

async fn capture_authority(
    ethereum: &Ethereum, view: &ethereum_client::FinalizedView, directory: &Path,
    name: &str, address: Address, artifact: &Artifact, layout: &Artifact,
    mut storage: BTreeMap<String, StorageWord>, implementation: Option<(B256, Address, &Artifact)>,
    complete: bool, immutables: &BTreeMap<String, B256>,
) -> Result<Authority> {
    let provider = ethereum.api.raw_provider();
    let header = view.checkpoint().common_header().clone();
    let proof = ethereum_client::inclusion::prove_account(provider, view, header.clone(), address).await?;
    let account = proof.verify(view, address)?.context("fixture authority account absent; HOLD")?;
    let runtime = artifact.runtime(address, immutables)?;
    let raw_code = provider.get_code_at(address).block_id(BlockId::hash_canonical(view.block_hash())).await?;
    ensure!(raw_code.as_ref() == runtime && keccak256(&runtime) == account.code_hash,
        "fixture {name} runtime differs from known artifact; HOLD");
    let implementation = if let Some((slot, implementation_address, artifact)) = implementation {
        let runtime = artifact.runtime(implementation_address, immutables)?;
        let code_hash = keccak256(&runtime);
        let implementation_proof = ethereum_client::inclusion::prove_account(provider, view, header.clone(), implementation_address).await?;
        let implementation_account = implementation_proof.verify(view, implementation_address)?.context("fixture implementation absent; HOLD")?;
        ensure!(implementation_account.code_hash == code_hash, "fixture implementation differs from known artifact; HOLD");
        let code = provider.get_code_at(implementation_address).block_id(BlockId::hash_canonical(view.block_hash())).await?;
        ensure!(code.as_ref() == runtime, "fixture implementation raw code differs; HOLD");
        immutable_json(&directory.join(format!("{name}-implementation.json")), &json!({
            "scope":"local-only-no-production-authorization", "accountProof":implementation_proof,
            "rawCode":code,"artifactSha256":sha256(&artifact.raw),"artifactRaw":Bytes::copy_from_slice(&artifact.raw)}))?;
        Some(Implementation { slot, address: implementation_address, code_hash,
            artifact_sha256: sha256(&runtime), runtime_code: runtime.into() })
    } else { None };
    let mut keys: BTreeSet<_> = storage.values().map(|word| word.slot).collect();
    if let Some(implementation) = &implementation { keys.insert(implementation.slot); }
    let response = provider.get_proof(address, keys.iter().copied().collect())
        .block_id(BlockId::hash_canonical(view.block_hash())).await?;
    ensure!(response.address == address && response.storage_proof.len() == keys.len(), "fixture storage response identity differs; HOLD");
    for word in storage.values_mut() {
        let entry = response.storage_proof.iter().find(|p| p.key.as_b256() == word.slot).context("fixture storage proof missing; HOLD")?;
        let mask = U256::MAX >> ((32 - usize::from(word.width)) * 8);
        word.value = B256::from(((entry.value >> (usize::from(word.offset) * 8)) & mask).to_be_bytes::<32>());
        verify_storage(account.storage_root, word, &entry.proof.iter().map(|p| p.to_vec()).collect::<Vec<_>>())?;
    }
    if let Some(implementation) = &implementation {
        let entry = response.storage_proof.iter().find(|p| p.key.as_b256() == implementation.slot).context("implementation slot proof missing; HOLD")?;
        verify_storage(account.storage_root, &StorageWord { slot: implementation.slot, value: address_word(implementation.address), offset: 0, width: 20 },
            &entry.proof.iter().map(|p| p.to_vec()).collect::<Vec<_>>())?;
    }
    let selected: BTreeMap<_, _> = storage.iter().map(|(name, word)| (name, (word.slot, word.offset, word.width))).collect();
    let storage_layout_sha256 = json_sha256(&(selected, implementation.as_ref().map(|i| i.slot)))?;
    immutable_json(&directory.join(format!("{name}.json")), &json!({
        "scope":"local-only-no-production-authorization", "accountProof":proof,"storageProof":response,"rawCode":raw_code,
        "artifactSha256":sha256(&artifact.raw),"artifactRaw":Bytes::copy_from_slice(&artifact.raw),
        "layoutArtifactSha256":sha256(&layout.raw),"layoutArtifactRaw":Bytes::copy_from_slice(&layout.raw),
        "selectedStorage":storage,"storageLayoutSha256":storage_layout_sha256}))?;
    Ok(Authority { address, code_hash: account.code_hash, artifact_sha256: sha256(&runtime), runtime_code: runtime.into(),
        storage_layout_sha256, storage_root: complete.then_some(account.storage_root), storage, implementation })
}

/// The digest is returned separately from the immutable JSON; no process-global environment is changed.
pub(crate) async fn generate(
    ethereum: &Ethereum, source: &Source, initial: &CapturedCommitment, proof: &SourceProof,
    output: &Path, topology: &FixtureTopology,
) -> Result<(Value, B256)> {
    topology.owned_network.validate()?;
    ensure!(ethereum.api.finality_identity()? == (topology.owned_network.ethereum_chain_id, topology.owned_network.ethereum_genesis_hash),
        "fixture EL network differs from independent approval; HOLD");
    let beacon_raw = committed_bytes(&topology.beacon_approval_path)?;
    ensure!(sha256(&beacon_raw) == topology.beacon_approval_sha256, "fixture Beacon approval digest differs; HOLD");
    let beacon: ethereum_client::beacon_finality::BootstrapApproval = serde_json::from_slice(&beacon_raw)?;
    ensure!(beacon.chain_id == topology.owned_network.ethereum_chain_id && beacon.genesis_hash == topology.owned_network.ethereum_genesis_hash
        && beacon.genesis_validators_root == topology.owned_network.beacon_genesis_validators_root
        && beacon.genesis_time == topology.owned_network.beacon_genesis_time, "fixture Beacon/network approval differs; HOLD");
    let captured = source.recapture(initial.block, initial.raw.clone()).await?;
    ensure!(captured.block_hash == initial.block_hash && captured.validated.mmr_root == initial.validated.mmr_root
        && captured.current == initial.current && captured.next == initial.next, "fixture native initial commitment changed; HOLD");
    ensure!(proof.source.checked_add(1) == Some(initial.block), "fixture initial proof is not its freshness leaf; HOLD");
    let authenticated = source.proof(proof.source, &captured).await?;
    ensure!(authenticated.source_hash == proof.source_hash && authenticated.raw_leaf == proof.raw_leaf && authenticated.snapshot == proof.snapshot
        && crate::evidence::freshness_evidence(&authenticated)? == crate::evidence::freshness_evidence(proof)?, "fixture native initial proof changed; HOLD");
    let anchor = json!({"sourceGenesis":B256::from(source.source_genesis),"bridgeDomain":B256::from(source.bridge_domain),
        "mmrStartBlock":source.mmr_start_block,"beefyActivationBlock":source.beefy_activation_block,
        "domainBindingBlock":source.identity.domain_binding_block,"sourceIdentity":source.identity,
        "block":captured.block,"blockHash":B256::from(captured.block_hash),"sourceTimestampMs":authenticated.snapshot.source_timestamp_ms,
        "freshnessSourceBlock":authenticated.source,"mmrRoot":B256::from(captured.validated.mmr_root),
        "signedCommitmentScale":Bytes::copy_from_slice(&captured.raw),"current":captured.current,"next":captured.next});
    source.validate_attachment(&anchor).await?;
    let (gear_manager, manager_code, tokens) = match &topology.native {
        None => (B256::ZERO, B256::ZERO, Vec::new()),
        Some(native) => {
            nonzero(native.manager, "actual fixture native manager")?;
            nonzero(native.manager_code_id, "actual fixture native manager CodeId")?;
            ensure!(!native.tokens.is_empty() && topology.evm_manager == Some(ethereum.receiver_address), "token fixture requires actual inventory and manager receiver; HOLD");
            let tokens = native.tokens.iter().map(|token| TokenIdentity {
                actor_id: token.actor_id, code_id: token.code_id, ethereum_token: token.ethereum_token,
                native_wrapper: token.native_wrapper.as_ref().map(|wrapper| WrapperIdentity {
                    mode: match wrapper.mode { NativeWrapperMode::EscrowV1 => WrapperMode::EscrowV1, NativeWrapperMode::HeldEscrowV2 => WrapperMode::HeldEscrowV2 },
                    idl_sha256: wrapper.idl_sha256,
                }),
            }).collect();
            (native.manager, native.manager_code_id, tokens)
        }
    };
    let directory = output.join("local-fixture-approval-evidence");
    fs::create_dir_all(&directory)?;
    immutable_json(&directory.join("native-bootstrap.json"), &json!({"scope":"local-only-no-production-authorization",
        "anchor":anchor,"proof":crate::evidence::proof_evidence(&authenticated, &captured, &[]),"inventory":tokens,"manager":gear_manager,"managerCodeId":manager_code}))?;
    immutable_json(&directory.join("beacon-approval.json"), &json!({"scope":"local-only-no-production-authorization",
        "raw":Bytes::copy_from_slice(&beacon_raw),"sha256":topology.beacon_approval_sha256,"ownedNetwork":topology.owned_network}))?;
    let mut ethereum_manifest = ethereum.manifest().await?;
    ethereum_manifest["sourceGenesis"] = json!(B256::from(source.source_genesis));
    // A later token approval retains the deployed bootstrap, not the client's
    // mutable latest checkpoint. The constructor initializes its MMR root to zero.
    ethereum_manifest["checkpoint"] = json!({
        "block":captured.block,"root":B256::ZERO,
        "sourceTimestampMs":authenticated.snapshot.source_timestamp_ms,
        "currentId":captured.current.id,"currentLength":captured.current.keys.len(),
        "currentRoot":B256::from(captured.current.root),
        "nextId":captured.next.id,"nextLength":captured.next.keys.len(),
        "nextRoot":B256::from(captured.next.root)
    });
    let view = ethereum.api.verified_finalized_view().await?;
    ensure!(matches!(view.checkpoint(), ethereum_client::beacon_finality::FinalityCheckpoint::Beacon(checkpoint)
        if checkpoint.beacon.bootstrap_approval_hash == topology.beacon_approval_sha256
            && checkpoint.beacon.genesis_validators_root == topology.owned_network.beacon_genesis_validators_root),
        "fixture view is not authenticated by the supplied Beacon approval; HOLD");
    immutable_json(&directory.join("beacon-checkpoint.json"), view.checkpoint())?;
    let mut immutables = BTreeMap::from([
        ("sourceDomain".into(), hash(&ethereum_manifest, "sourceDomain")?),
        ("bridgeDomain".into(), B256::from(source.bridge_domain)),
        ("destinationChainId".into(), slot(topology.owned_network.ethereum_chain_id)),
        ("destinationQueue".into(), address_word(ethereum.queue_address)),
        ("mmrStartBlock".into(), slot(source.mmr_start_block)),
        ("beefyClient".into(), address_word(ethereum.client_address)),
        ("messageQueue".into(), address_word(ethereum.queue_address)),
    ]);
    // Runtime templates supply every immutable range; only these known constructor identities may fill it.
    let mut authorities = BTreeMap::new();
    for (name, address, artifact_name) in [("client", ethereum.client_address, "BeefyClient"),
        ("verifier", ethereum.verifier_address, "VaraQueueRootVerifier"),
        ("governanceAdmin", topology.governance_admin, "GovernanceAdmin"),
        ("governancePauser", topology.governance_pauser, "GovernancePauser")] {
        let artifact = Artifact::compiled(artifact_name)?;
        let fields = if name.starts_with("governance") {
            artifact.fields(&[("governance","governance"),("messageQueue","messageQueue"),("erc20Manager","erc20Manager")])?
        } else { BTreeMap::new() };
        authorities.insert(name.into(), capture_authority(ethereum, &view, &directory, name, address,
            &artifact, &artifact, fields, None, name.starts_with("governance"), &immutables).await?);
    }
    let proxy = Artifact::compiled("ERC1967Proxy")?;
    let implementation_slot: B256 = "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc".parse()?;
    let mut proxies = vec![("queue", ethereum.queue_address, "MessageQueue")];
    if let Some(manager) = topology.evm_manager { proxies.push((if topology.native.is_some() { "receiver" } else { "erc20Manager" }, manager, "ERC20Manager")); }
    for (name, address, artifact_name) in proxies {
        let artifact = Artifact::compiled(artifact_name)?;
        let fields = if name == "queue" {
            artifact.fields(&[("verifier","_verifier"),("governanceAdmin","_governanceAdmin"),
                ("governancePauser","_governancePauser"),("beefyRecoveryTimelock","beefyRecoveryTimelock")])?
        } else { artifact.fields(&[("messageQueue","_messageQueue"),("governanceAdmin","_governanceAdmin"),("governancePauser","_governancePauser")])? };
        let raw = ethereum.api.raw_provider().get_storage_at(address, U256::from_be_slice(implementation_slot.as_slice()))
            .block_id(BlockId::hash_canonical(view.block_hash())).await?;
        let encoded = raw.to_be_bytes::<32>();
        ensure!(encoded[..12] == [0; 12], "invalid fixture implementation address; HOLD");
        let implementation = Address::from_slice(&encoded[12..]);
        authorities.insert(name.into(), capture_authority(ethereum, &view, &directory, name, address,
            &proxy, &artifact, fields, Some((implementation_slot, implementation, &artifact)), false, &immutables).await?);
    }
    if topology.native.is_none() {
        let artifact = Artifact::compiled("MessageHandlerMock")?;
        authorities.insert("receiver".into(), capture_authority(ethereum, &view, &directory, "receiver", ethereum.receiver_address,
            &artifact, &artifact, BTreeMap::new(), None, false, &immutables).await?);
    }
    let timelock = Artifact::compiled("TimelockController")?;
    let mut timelock_fields = timelock.fields(&[("minDelay","_minDelay")])?;
    let layout = &timelock.value["storageLayout"];
    let roles = layout["storage"].as_array().context("timelock layout missing")?.iter()
        .find(|entry| entry["label"] == "_roles").context("timelock role layout missing; HOLD")?;
    let role_type = &layout["types"][roles["type"].as_str().context("role type missing")?];
    let members = &layout["types"][role_type["value"].as_str().context("role struct missing")?]["members"];
    ensure!(role_type["encoding"] == "mapping" && members.as_array().is_some_and(|entries| entries.iter().any(|entry|
        entry["label"] == "hasRole" && entry["slot"] == "0" && entry["offset"] == 0)), "unknown timelock role layout; HOLD");
    let roles_slot = B256::from(U256::from_str_radix(roles["slot"].as_str().context("role slot missing")?, 10)?.to_be_bytes::<32>());
    for (name, role, member) in [("proposer",keccak256("PROPOSER_ROLE"),topology.safe),
        ("executor",keccak256("EXECUTOR_ROLE"),topology.safe),("canceller",keccak256("CANCELLER_ROLE"),topology.safe),
        ("admin",B256::ZERO,topology.recovery)] {
        timelock_fields.insert(format!("{name}:{member:#x}"), word(mapping(address_word(member),mapping(role,roles_slot)),0,1));
    }
    authorities.insert("timelock".into(), capture_authority(ethereum, &view, &directory, "timelock", topology.recovery,
        &timelock, &timelock, timelock_fields, None, true, &immutables).await?);
    require_complete_initial_state(&authorities["timelock"])?;
    let (safe_proxy, safe_singleton) = safe_artifacts()?;
    let sentinel = Address::from_slice(&slot(1).as_slice()[12..]);
    unique_addresses(&topology.safe_owners, "fixture Safe owners", false)?;
    ensure!(topology.safe_owners.len() == 2 && !topology.safe_owners.contains(&sentinel), "fixture requires two real Safe owners; HOLD");
    let mut safe_fields = BTreeMap::from([
        ("ownerCount".into(),word(slot(3),0,32)), ("threshold".into(),word(slot(4),0,32)),
        ("guard".into(),word("0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8".parse()?,0,20)),
        ("fallbackHandler".into(),word("0x6c9a6c4a39284e37ed1cf53d337577d14212a4870fb976a4366c693b939918d5".parse()?,0,20)),
        ("nonce".into(),word(slot(5),0,32)),
        (format!("module:{sentinel:#x}"),word(mapping(address_word(sentinel),slot(1)),0,20)),
    ]);
    for previous in std::iter::once(sentinel).chain(topology.safe_owners.iter().copied()) {
        safe_fields.insert(format!("owner:{previous:#x}"),word(mapping(address_word(previous),slot(2)),0,20));
    }
    immutables.clear();
    authorities.insert("safe".into(), capture_authority(ethereum, &view, &directory, "safe", topology.safe,
        &safe_proxy, &safe_singleton, safe_fields, Some((slot(0),topology.singleton,&safe_singleton)), true, &immutables).await?);
    require_word(&authorities["safe"], "fallbackHandler", B256::ZERO)?;
    require_word(&authorities["safe"], "nonce", B256::ZERO)?;
    require_complete_initial_state(&authorities["safe"])?;
    immutable_json(&directory.join("safe-provenance.json"), &serde_json::from_slice::<Value>(
        &committed_bytes(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ethereum/test/fixtures/safe-1.4.1/provenance.json"))?)?)?;
    let safe_report = json_sha256(&json!({"scope":"local-only-no-production-authorization","authority":authorities["safe"],
        "owners":topology.safe_owners,"threshold":2,"modules":[],"guard":Address::ZERO}))?;
    let recovery_report = json_sha256(&json!({"scope":"local-only-no-production-authorization","authority":authorities["timelock"],
        "soleAdmin":topology.recovery,"proposerExecutorCanceller":topology.safe}))?;
    let report = json!({"scope":"local-only-no-production-authorization", "nativeBootstrap":anchor,
        "beaconApprovalSha256":topology.beacon_approval_sha256,"authorities":authorities,"safeStateReport":safe_report,
        "recoveryStateReport":recovery_report,"qualification":"owned fixture evidence; not a production safety review"});
    immutable_json(&directory.join("approval-source.json"), &report)?;
    let report_hash = json_sha256(&report)?;
    let manifest = Manifest { schema_version:2,execution_authorized:true,
        mode:if topology.native.is_some() { "hoodi-token-stack" } else { "hoodi-message-stack" }.into(),
        local_rehearsal:false,manager:ethereum.receiver_address,gear_manager,anchor:anchor.clone(),ethereum:ethereum_manifest,
        owned_fixture_network:Some(topology.owned_network.clone()),ethereum_configuration:None,
        execution:Approval { source_identity_report_sha256:json_sha256(&anchor["sourceIdentity"])?,
            beacon_approval_sha256:topology.beacon_approval_sha256,bootstrap_sha256:json_sha256(&anchor)?,
            token_inventory_sha256:json_sha256(&tokens)?,token_inventory:tokens,authorities,
            recovery:Recovery { address:topology.recovery,min_delay:topology.min_delay,proposers:vec![topology.safe],
                executors:vec![topology.safe],admins:vec![topology.recovery],cancellers:vec![topology.safe],
                complete_state_report_sha256:recovery_report,safes:vec![Safe { address:topology.safe,singleton:topology.singleton,
                    owners:topology.safe_owners.clone(),threshold:2,modules:Vec::new(),guard:Address::ZERO,complete_state_report_sha256:safe_report }] },
            runtime_replay_safety_report_sha256:report_hash,runtime_heartbeat_report_sha256:report_hash,
            custody_authority_report_sha256:report_hash,gear_manager_code_id:manager_code,deployment_plan_hash:None } };
    let value = serde_json::to_value(manifest)?;
    let raw = serde_json::to_vec(&value)?;
    let digest = sha256(&raw);
    let approval = ExecutionAuthority::parse(&raw, Some(digest), topology.beacon_approval_sha256)?.context("fixture approval missing; HOLD")?;
    approval.verify_source(source).await?;
    approval.verify(ethereum).await?;
    view.recheck(ethereum.api.raw_provider()).await?;
    immutable_json(&output.join("local-fixture-manifest.json"), &value)?;
    immutable_json(&directory.join("manifest-digest.json"), &json!({"scope":"local-only-no-production-authorization","sha256":digest}))?;
    Ok((value, digest))
}

/// Loads only the separately approved exact bytes and exercises the unmodified production gate.
pub(crate) async fn load(path: &Path, digest: B256, beacon: B256, ethereum: &Ethereum, source: &Source) -> Result<ExecutionAuthority> {
    let authority = ExecutionAuthority::parse(&committed_bytes(path)?, Some(digest), beacon)?.context("fixture approval absent; HOLD")?;
    ensure!(authority.manifest.owned_fixture_network.is_some(), "fixture loader requires an owned local approval; HOLD");
    authority.verify_source(source).await?;
    authority.verify(ethereum).await?;
    Ok(authority)
}

fn safe_artifacts() -> Result<(Artifact, Artifact)> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ethereum/test/fixtures/safe-1.4.1");
    let proxy = Artifact::read("SafeProxy", &directory.join("SafeProxy.json"))?;
    let singleton = Artifact::read("Safe", &directory.join("Safe.json"))?;
    // The exact official artifact hashes bind the inspected 1.4.1 slots above.
    // A caller-supplied replacement provenance file cannot approve unknown Safe code.
    let provenance: Value = serde_json::from_slice(&committed_bytes(&directory.join("provenance.json"))?)?;
    ensure!(provenance["version"] == "1.4.1" && provenance["fixtureOnly"] == true,
        "unknown Safe artifact version; HOLD");
    for (artifact, digest) in [
        (&proxy, "b05eaeaf7278097e52a9e9b38410de2a812c23fa3622373473e73eaa19646ecd"),
        (&singleton, "c36a5e99bc3f25c2d75ac5f7920b679200b3ce617c0d26e674b1321dbc6b0bdf"),
    ] {
        ensure!(sha256(&artifact.raw) == format!("0x{digest}").parse::<B256>()?
            && provenance["artifacts"][&artifact.name]["sha256"] == digest,
            "Safe artifact differs from pinned official 1.4.1 artifact; HOLD");
    }
    Ok((proxy, singleton))
}
