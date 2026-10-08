use alloy::{
    consensus::Transaction as _,
    eips::{Decodable2718, Encodable2718},
    primitives::{Address, B256, U256 as EthU256},
    providers::Provider,
    rpc::types::{BlockId, BlockNumberOrTag, Filter},
    signers::{local::PrivateKeySigner, Signer},
    sol,
    sol_types::{SolCall, SolEvent},
};
use anyhow::{anyhow, bail, ensure, Context as AnyhowContext, Result};
use bridging_payment_client::{
    bridging_payment::events::BridgingPaymentEvents,
    traits::BridgingPayment as BridgingPaymentCalls, BridgingPayment,
};
use checkpoint_light_client_client::{
    traits::{ServiceCheckpointFor as _, ServiceState as _},
    CheckpointError, Order, ServiceCheckpointFor,
};
use clap::ValueEnum;
use eth_events_electra_client::EthToVaraEvent;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::PollingEthApi;
use ethereum_common::{tree_hash::TreeHash, SECONDS_PER_SLOT};
use futures::{stream::FuturesUnordered, Future, StreamExt};
use gear_core::ids::ActorId;
use gsdk::{
    ext::{
        sp_core::{sr25519, Pair},
        subxt::utils::H256 as GearHash,
    },
    gear::{gear::Event as GearEvent, Event as RuntimeEvent},
    AsGear,
};
use historical_proxy_client::historical_proxy::io::Redirect;
use parity_scale_codec::{Decode, Encode};
use relayer::message_relayer::eth_to_gear::{
    storage::{Block as InboundBlock, InboundRuntimeIdentity},
    tx_manager::Transaction as InboundTransaction,
};
use sails_rs::{
    calls::{Action as SailsAction, *},
    events::EventIo,
    gclient::calls::{GClientRemoting, QueryExtGClient},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sp_core::{H160, H256, U256 as GearU256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subxt_rpcs::rpc_params;
use tokio::time::{sleep, sleep_until, timeout_at};
use url::Url;
use vft_client::{
    traits::{Vft as VftCalls, VftAdmin as VftAdminCalls, VftMetadata as VftMetadataCalls},
    Vft, VftAdmin, VftMetadata,
};
use vft_manager_client::{
    traits::VftManager as VftManagerCalls,
    vft_manager::{events::VftManagerEvents, io::SubmitReceipt},
    ReceiptStatus as GearReceiptStatus, TokenSupply, VftManager,
};

use crate::{
    ethereum::{hoodi_wallet_address, DestinationCheckpoint, Ethereum},
    hoodi::{self, number},
    source::{self, AuthoritySet, Source},
};
use beefy_relay::keccak256;
use vft_vara_client::traits::NativeEscrow as _;

const SCHEMA_VERSION: u32 = 3;
const HOODI_CHAIN_ID: u64 = 560_048;
const HOODI_EXECUTION_GENESIS: &str =
    "0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b";
const HOODI_GENESIS_VALIDATORS_ROOT: &str =
    "212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f";
const HOUR_MS: u64 = 60 * 60 * 1_000;
const QUALIFICATION_HOURS: u8 = 24;
const WINDOW_AMOUNT: u64 = 1;
const CAMPAIGN_ALLOWANCE: u64 = 24;
const PRELIGHT_MAX_SECS: u64 = 60 * 60;
const WARMUP_SECS: u64 = 60 * 60;
const MAX_AUTHORITY_LAG: u64 = 64;
const CLOCK_DRIFT_MS: u64 = 5_000;

#[derive(Clone, Copy)]
struct CampaignSchedule {
    warmup_secs: u64,
    authority_lag_limit: u64,
    handovers: usize,
}

impl CampaignSchedule {
    fn from_profile(profile: &Value) -> Result<Self> {
        if profile.is_null() {
            return Ok(Self {
                warmup_secs: WARMUP_SECS,
                authority_lag_limit: MAX_AUTHORITY_LAG,
                handovers: 1,
            });
        }
        let (warmup_secs, authority_lag_limit) = match profile["name"].as_str() {
            Some("normal-runtime-hoodi") => {
                ensure!(
                    profile.get("functionalOnly").is_none(),
                    "normal runtime cannot claim functional-only fast qualification; HOLD"
                );
                ensure!(
                    profile.get("cadencePatchSha256").is_none(),
                    "normal runtime cannot carry a cadence patch; HOLD"
                );
                (5 * 60 * 60, 2400)
            }
            Some("fast-runtime-hoodi") => {
                ensure!(
                    profile["functionalOnly"] == true,
                    "fast runtime must remain functional-only; HOLD"
                );
                ensure!(
                    bytes32(string(
                        &profile["cadencePatchSha256"],
                        "pinned cadence patch"
                    )?)? != [0; 32],
                    "fast cadence patch pin is zero; HOLD"
                );
                (60 * 60, 64)
            }
            _ => bail!("unsupported runtime campaign profile; HOLD"),
        };
        ensure!(
            profile.is_object()
                && profile["runtimeCommit"] == "19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c"
                && profile["runtimePullRequest"] == 5642
                && profile["warmupDurationMs"] == warmup_secs * 1000
                && profile["requiredAuthorityHandovers"] == 2
                && profile["tokenBatchDurationMs"] == 3_600_000
                && profile["applicationAttemptDurationMs"] == 2_640_000
                && profile["slotDurationMs"] == 3000
                && profile["epochDurationBlocks"] == authority_lag_limit
                && profile["testOnly"] == true
                && profile["executionAuthorized"] == false
                && profile["releaseQualified"] == false,
            "runtime campaign requires its exact sealed schedule; HOLD"
        );
        for name in [
            "gearBinarySha256",
            "runtimeCodeSha256",
            "runtimeCodeKeccak256",
            "runtimeCodeBlake2b256",
            "approvalSha256",
        ] {
            ensure!(
                bytes32(string(&profile[name], name)?)? != [0; 32],
                "runtime artifact/approval pin {name} is zero; HOLD"
            );
        }
        ensure!(
            matches!(
                profile["runtimeCiStatus"].as_str(),
                Some("unresolved" | "failed" | "passed")
            ),
            "runtime CI status must remain explicit; HOLD"
        );
        Ok(Self {
            warmup_secs,
            authority_lag_limit,
            handovers: 2,
        })
    }
}

sol! {
    #[sol(rpc)]
    interface CampaignToken {
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function balanceOf(address owner) external view returns (uint256);
        function totalSupply() external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function DOMAIN_SEPARATOR() external view returns (bytes32);
        function nonces(address owner) external view returns (uint256);
    }

    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    interface CampaignManager {
        event Bridged(bytes32 indexed from, address indexed to, address indexed token, uint256 amount);
        function tokens() external view returns (address[] memory);
        function getTokenType(address token) external view returns (uint8);
        function requestBridging(address token, uint256 amount, bytes32 to) external;
        function requestBridgingWithPermit(
            address token,
            uint256 amount,
            bytes32 to,
            uint256 deadline,
            uint8 v,
            bytes32 r,
            bytes32 s
        ) external;
    }

    #[sol(rpc)]
    interface CampaignPauser {
        function governance() external view returns (bytes32);
        function messageQueue() external view returns (address);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Preflight,
    Warmup,
    Start,
    Resume,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FeeMode {
    Normal,
    Priority,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Action {
    status: String,
    intent: Value,
    from_block: Option<u64>,
    nonce: Option<u64>,
    tx_hash: Option<String>,
    evidence: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Phase {
    status: String,
    started_at_ms: Option<u64>,
    completed_at_ms: Option<u64>,
    evidence: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    schema_version: u32,
    run_id: String,
    deployment_manifest_digest: String,
    token_stack_digest: String,
    source_launch_digest: String,
    source_genesis: String,
    bridge_domain: String,
    raw_spec_sha256: String,
    endpoints: Value,
    accounts: Value,
    readiness: BTreeMap<String, Value>,
    preflight: Phase,
    warmup: Phase,
    t0_ms: Option<u64>,
    status: String,
    last_wall_ms: Option<u64>,
    next_campaign_nonce: Option<u64>,
    actions: BTreeMap<String, Action>,
    windows: BTreeMap<String, Value>,
    incidents: Vec<Value>,
    terminal_report: Option<Value>,
}

impl Journal {
    #[allow(clippy::too_many_arguments)]
    fn new(
        run_id: String,
        manifest_digest: String,
        token_stack_digest: String,
        source_launch_digest: String,
        source_genesis: String,
        bridge_domain: String,
        raw_spec_sha256: String,
        endpoints: Value,
        accounts: Value,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            run_id,
            deployment_manifest_digest: manifest_digest,
            token_stack_digest,
            source_launch_digest,
            source_genesis,
            bridge_domain,
            raw_spec_sha256,
            endpoints,
            accounts,
            readiness: BTreeMap::new(),
            preflight: Phase::pending(),
            warmup: Phase::pending(),
            t0_ms: None,
            status: "preparing".into(),
            last_wall_ms: None,
            next_campaign_nonce: None,
            actions: BTreeMap::new(),
            windows: BTreeMap::new(),
            incidents: Vec::new(),
            terminal_report: None,
        }
    }

    fn validate_identity(&self, expected: &Journal, now: u64) -> Result<()> {
        self.validate_wall_time(now)?;
        ensure!(
            self.schema_version == SCHEMA_VERSION,
            "incompatible campaign journal schema"
        );
        ensure!(
            self.run_id == expected.run_id
                && self.deployment_manifest_digest == expected.deployment_manifest_digest
                && self.token_stack_digest == expected.token_stack_digest
                && self.source_launch_digest == expected.source_launch_digest
                && self.source_genesis == expected.source_genesis
                && self.bridge_domain == expected.bridge_domain
                && self.raw_spec_sha256 == expected.raw_spec_sha256
                && self.endpoints == expected.endpoints
                && self.accounts == expected.accounts,
            "campaign identity changed from its immutable journal"
        );
        Ok(())
    }

    fn validate_wall_time(&self, now: u64) -> Result<()> {
        ensure!(
            self.last_wall_ms.is_none_or(|last| now >= last),
            "system clock moved backwards from the persisted campaign timestamp; HOLD"
        );
        Ok(())
    }
}

impl Phase {
    fn pending() -> Self {
        Self {
            status: "pending".into(),
            started_at_ms: None,
            completed_at_ms: None,
            evidence: json!({}),
        }
    }
}

#[derive(Clone, Debug)]
struct Token {
    symbol: &'static str,
    component: &'static str,
    address: Address,
    peer: ActorId,
    gear_origin: bool,
    native_amount: Option<u64>,
    escrow: Option<ActorId>,
}

impl Token {
    fn raw_amount(&self, base: u64) -> Result<u64> {
        self.native_amount
            .unwrap_or(1)
            .checked_mul(base)
            .context("native campaign raw amount overflow")
    }
    fn gear_amount(&self, base: GearU256) -> Result<GearU256> {
        base.checked_mul(GearU256::from(self.native_amount.unwrap_or(1)))
            .context("native campaign amount overflow")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawSnapshot {
    evm_user: EthU256,
    evm_escrow: EthU256,
    evm_supply: EthU256,
    gear_user: GearU256,
    gear_supply: GearU256,
    gear_escrow: Option<GearU256>,
}

impl RawSnapshot {
    fn has_no_bridge_liabilities(&self) -> bool {
        self.evm_escrow.is_zero()
            && match self.gear_escrow {
                Some(escrow) => {
                    escrow.is_zero()
                        && self.evm_user.is_zero()
                        && self.evm_supply.is_zero()
                        && self.gear_supply >= self.gear_user
                }
                None => self.gear_user.is_zero() && self.gear_supply.is_zero(),
            }
    }
    fn to_json(&self) -> Value {
        let mut value = json!({
            "evmUser": self.evm_user.to_string(),
            "evmManagerEscrow": self.evm_escrow.to_string(),
            "erc20Supply": self.evm_supply.to_string(),
            "gearUser": self.gear_user.to_string(),
            "vftSupply": self.gear_supply.to_string(),
        });
        if let Some(escrow) = self.gear_escrow {
            value["gearManagerEscrow"] = json!(escrow.to_string());
        }
        value
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SnapshotSet {
    gear_height: u32,
    gear_hash: GearHash,
    evm_height: u64,
    evm_hash: B256,
    assets: BTreeMap<String, RawSnapshot>,
}

#[derive(Clone, Debug)]
struct OutboundRequest {
    nonce: GearU256,
    queue_id: u64,
    message_hash: [u8; 32],
    queue_block: u32,
    queue_block_hash: GearHash,
}

#[derive(Clone, Debug)]
struct QueueRoot {
    source_block: u32,
    block_hash: GearHash,
    queue_id: u64,
    root: [u8; 32],
}

#[derive(Deserialize)]
struct InboundJournal {
    schema_version: u32,
    runtime_identity: InboundRuntimeIdentity,
    #[serde(default)]
    manual_identity: Option<Value>,
    transactions: BTreeMap<String, InboundTransaction>,
    completed: BTreeMap<String, InboundTransaction>,
    failed: BTreeMap<String, String>,
}

impl InboundJournal {
    fn validate_identity(
        &self,
        expected: &InboundRuntimeIdentity,
        campaign_actor: ActorId,
        governance_actor: ActorId,
    ) -> Result<()> {
        ensure!(
            matches!(self.schema_version,5|6),
            "inbound worker journal lacks authenticated original-transaction proof provenance (schema 5 or 6 required)"
        );
        ensure!(
            self.manual_identity.is_none(),
            "manual inbound journal cannot qualify the automatic campaign worker; HOLD"
        );
        ensure!(
            self.runtime_identity == *expected,
            "inbound worker runtime identity differs from the immutable campaign lane; HOLD"
        );
        ensure!(
            self.runtime_identity.gear_sender != [0; 32]
                && self.runtime_identity.gear_sender != campaign_actor.into_bytes()
                && self.runtime_identity.gear_sender != governance_actor.into_bytes(),
            "inbound worker Gear signer is missing or overlaps campaign/governance; HOLD"
        );
        for (uuid, tx) in self.transactions.iter().chain(&self.completed) {
            ensure!(
                *uuid == tx.uuid.to_string(),
                "inbound worker transaction UUID differs from its key"
            );
            if let Some(response) = tx
                .receipt
                .as_ref()
                .and_then(|r| r.initial_response.as_ref())
            {
                ensure!(
                    response.manager == expected.vft_manager_address
                        && response.historical_proxy == expected.historical_proxy_address,
                    "inbound worker submitted to a different manager or historical proxy"
                );
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct OutboundTransactionStatus {
    version: u8,
    active: BTreeMap<String, String>,
    failed: Vec<String>,
}

fn outbound_status_idle(status: &OutboundTransactionStatus) -> Result<bool> {
    ensure!(
        status.version == 1,
        "unsupported outbound transaction status schema; HOLD"
    );
    ensure!(
        status.active.values().all(|state| matches!(
            state.as_str(),
            "WaitForMerkleRoot"
                | "FetchMerkleRoot"
                | "PrepareMessage"
                | "SendMessage"
                | "WaitConfirmations"
                | "Failed"
                | "NeedsReconciliation"
        )),
        "unknown active outbound transaction state; HOLD"
    );
    Ok(status.active.is_empty() && status.failed.is_empty())
}

fn outbound_save_complete(directory: &Path) -> Result<bool> {
    for marker in [
        directory.join(".state/save.pending"),
        directory.join("blocks.json.new"),
    ] {
        match fs::symlink_metadata(marker) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    match fs::read_dir(directory.join(".state")) {
        Ok(entries) => {
            for entry in entries {
                if entry?.file_name().to_string_lossy().ends_with(".new") {
                    return Ok(false);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(true)
}

struct Context {
    source_rpc: String,
    witness_rpc: String,
    ethereum_rpc: String,
    source: Source,
    witness: Source,
    gear_api: gclient::GearApi,
    remoting: GClientRemoting,
    manager_id: ActorId,
    payment_id: ActorId,
    historical_proxy_id: ActorId,
    checkpoint_id: ActorId,
    campaign_actor: ActorId,
    governance_actor: ActorId,
    campaign_address: Address,
    publisher_address: Address,
    follower_address: Address,
    campaign_wallet: PathBuf,
    governance_suri: String,
    follower_dir: PathBuf,
    inbound_dir: PathBuf,
    inbound_start_block: u64,
    outbound_dir: PathBuf,

    deployment: Value,
    ethereum: Ethereum,
    polling_eth: PollingEthApi,
    beacon: BeaconClient,
    tokens: Vec<Token>,
    schedule: CampaignSchedule,
    normal_fee: u128,
    priority_fee: u128,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    mode: Mode,
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    beacon_rpc: &str,
    campaign_wallet: &Path,
    follower_dir: &Path,
    inbound_dir: &Path,
    outbound_dir: &Path,
    campaign_suri: &str,
    governance_suri: &str,
    rotation_suri: &str,
    rotation_authority: &str,
    deployment_manifest_path: &Path,
    token_stack_path: &Path,
    raw_spec_path: &Path,
    source_launch_path: &Path,
    output_dir: &Path,
) -> Result<()> {
    ensure!(
        crate::local_source_rpc(source_rpc) && crate::local_source_rpc(witness_rpc),
        "development-key source and witness RPCs must be loopback"
    );
    ensure!(
        source_rpc != witness_rpc,
        "two independent Gear authorities are required"
    );
    let beacon_url = Url::parse(beacon_rpc).context("parse beacon endpoint")?;
    ensure!(
        beacon_url.scheme() == "https"
            && beacon_url.username().is_empty()
            && beacon_url.password().is_none()
            && beacon_url.fragment().is_none(),
        "beacon endpoint must be an authenticated HTTPS URL without userinfo or fragment"
    );
    ensure!(
        ethereum_rpc.starts_with("wss://"),
        "Hoodi requires secure websocket RPC"
    );
    ensure!(
        campaign_wallet.is_absolute()
            && follower_dir.is_absolute()
            && inbound_dir.is_absolute()
            && outbound_dir.is_absolute(),
        "campaign wallet and all service directories must be absolute protected paths"
    );

    let deployment: Value = serde_json::from_slice(&fs::read(deployment_manifest_path)?)?;
    let stack: Value = serde_json::from_slice(&fs::read(token_stack_path)?)?;
    let launch: Value = serde_json::from_slice(&fs::read(source_launch_path)?)?;
    let raw_spec = fs::read(raw_spec_path)
        .with_context(|| format!("read raw chain spec {}", raw_spec_path.display()))?;
    verify_raw_spec_digest(&raw_spec, &launch["identity"]["rawSpecSha256"])?;
    let journal_path = output_dir.join("campaign-state.json");
    let saved_journal = if journal_path.exists() {
        Some(
            serde_json::from_slice::<Journal>(&fs::read(&journal_path)?)
                .context("read campaign journal")?,
        )
    } else {
        None
    };
    if let Some(journal) = saved_journal.as_ref() {
        journal.validate_wall_time(now_ms()?)?;
        ensure_preflight_probe_boundary(journal)?;
    }
    ensure!(
        saved_journal.is_some() || mode == Mode::Preflight,
        "start/resume requires an existing campaign journal"
    );
    if mode == Mode::Preflight {
        if let Some(journal) = saved_journal.as_ref() {
            governance_pause_completed(journal)?;
        }
    }
    let finalized_deployment: Value = serde_json::from_slice(&fs::read(
        deployment_manifest_path
            .parent()
            .context("deployment manifest has no run directory")?
            .join("hoodi/deployment-finalized.json"),
    )?)
    .context("read finalized token deployment evidence")?;
    let inbound_start_block = inbound_deployment_start_block(&deployment, &finalized_deployment)?;
    let preflight_readiness_deadline =
        (mode == Mode::Preflight).then(|| Instant::now() + Duration::from_secs(30 * 60));
    let service_state_path = follower_dir.join("state.json");
    let mut service_state = if service_state_path.exists() {
        Some(read_validated_follower_state(
            &service_state_path,
            &deployment,
        )?)
    } else {
        None
    };
    if service_state.is_none() && saved_journal.is_none() {
        service_state = Some(
            wait_for_follower_state(
                &service_state_path,
                &deployment,
                preflight_readiness_deadline.context("preflight readiness deadline missing")?,
            )
            .await?,
        );
    }
    let (follower_address, publisher_address) = match service_state.as_ref() {
        Some(state) => follower_signers(state, &deployment)?,
        None => journal_follower_signers(
            saved_journal
                .as_ref()
                .context("campaign identity is required while follower state is absent")?,
        )?,
    };
    let expected = Context::connect(
        source_rpc,
        witness_rpc,
        ethereum_rpc,
        beacon_rpc,
        campaign_wallet,
        follower_dir,
        inbound_dir,
        outbound_dir,
        follower_address,
        publisher_address,
        campaign_suri,
        governance_suri,
        &deployment,
        &stack,
        &launch,
        inbound_start_block,
    )
    .await?;
    let expected_journal = expected.journal_identity(output_dir, &deployment, &stack, &launch)?;
    let mut journal = if let Some(journal) = saved_journal {
        journal.validate_identity(&expected_journal, now_ms()?)?;
        journal
    } else {
        if output_dir.exists() {
            ensure!(
                fs::read_dir(output_dir)?.next().is_none(),
                "fresh preflight output directory is not empty"
            );
        } else {
            if let Some(parent) = output_dir.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::create_dir(output_dir)?;
            #[cfg(unix)]
            fs::set_permissions(
                output_dir,
                std::os::unix::fs::PermissionsExt::from_mode(0o700),
            )?;
        }
        expected_journal.clone()
    };
    let mut context = expected;
    if !journal_path.exists() {
        save_journal(&journal_path, &mut journal)?;
    }
    context
        .ethereum
        .api
        .enable_finality_archive(
            &output_dir.join("finality-history/headers"),
            HOODI_EXECUTION_GENESIS.parse()?,
        )
        .await?;
    revalidate_passed_windows(&context, &journal).await?;

    match mode {
        Mode::Preflight => {
            run_preflight(
                &mut context,
                &mut journal,
                &journal_path,
                preflight_readiness_deadline.context("preflight readiness deadline missing")?,
            )
            .await
        }
        Mode::Warmup => {
            run_warmup(
                &mut context,
                &mut journal,
                &journal_path,
                rotation_suri,
                rotation_authority,
            )
            .await
        }
        Mode::Start => {
            ensure!(
                journal.t0_ms.is_none(),
                "start cannot replace an existing T0; use resume"
            );
            ensure!(
                journal.preflight.status == "passed",
                "two complete Hoodi preflight batches are required before T0"
            );
            ensure!(
                journal.warmup.status == "passed",
                "the 60-minute rotation/restart capacity gate is required before T0"
            );
            lanes_ready(
                &mut context,
                &mut journal,
                &journal_path,
                "before_t0",
                0,
                Instant::now() + Duration::from_secs(120),
                &BTreeMap::new(),
            )
            .await?;
            prepare_campaign_allowances(&mut context, &mut journal, &journal_path).await?;
            verify_campaign_start(&context, &journal).await?;
            let t0 = now_ms()?;
            journal.t0_ms = Some(t0);
            journal.windows.insert(
                window_key(0),
                scheduled_window(
                    0,
                    t0,
                    t0.checked_add(HOUR_MS).context("hour deadline overflow")?,
                    t0,
                ),
            );
            journal.status = "running".into();
            save_journal(&journal_path, &mut journal)?;
            run_campaign(&mut context, &mut journal, &journal_path).await
        }
        Mode::Resume => {
            ensure!(
                journal.t0_ms.is_some(),
                "resume requires an immutable existing T0"
            );
            ensure!(
                journal.status != "passed",
                "campaign is already terminally qualified"
            );
            ensure!(
                journal.status != "failed" || journal.t0_ms.is_some(),
                "failed campaign cannot create a T0"
            );
            run_campaign(&mut context, &mut journal, &journal_path).await
        }
    }
}

impl Context {
    #[allow(clippy::too_many_arguments)]
    async fn connect(
        source_rpc: &str,
        witness_rpc: &str,
        ethereum_rpc: &str,
        beacon_rpc: &str,
        campaign_wallet: &Path,
        follower_dir: &Path,
        inbound_dir: &Path,
        outbound_dir: &Path,
        follower_address: Address,
        publisher_address: Address,
        campaign_suri: &str,
        governance_suri: &str,
        deployment: &Value,
        stack: &Value,
        launch: &Value,
        inbound_start_block: u64,
    ) -> Result<Self> {
        ensure!(
            deployment["mode"] == "hoodi-token-stack",
            "deployment manifest is not the isolated Hoodi token lane"
        );
        ensure!(
            stack["lane"] == "beefy-token-hoodi",
            "token stack is not the isolated Hoodi lane"
        );
        let schedule = CampaignSchedule::from_profile(&launch["identity"]["runtimeProfile"])?;
        let source_genesis = string(
            &deployment["anchor"]["sourceGenesis"],
            "anchor.sourceGenesis",
        )?;
        let bridge_domain = string(&deployment["anchor"]["bridgeDomain"], "anchor.bridgeDomain")?;
        ensure!(
            deployment["ethereum"]["sourceGenesis"] == source_genesis
                && deployment["ethereum"]["bridgeDomain"] == bridge_domain
                && stack["sourceGenesis"] == source_genesis,
            "source genesis/domain differs across deployment and token-stack manifests"
        );
        ensure!(
            deployment["ethereum"]["chainId"].as_u64() == Some(HOODI_CHAIN_ID),
            "deployment manifest is not bound to Hoodi chain ID 560048"
        );
        ensure!(
            stack["programs"]["vftManager"]["status"] == "active",
            "Gear VFT manager is not active"
        );
        ensure!(
            launch["phase"] == "ready"
                && launch["readiness"]["genesisHash"].as_str() == Some(source_genesis)
                && launch["identity"]["bridgeDomain"].as_str() == Some(bridge_domain)
                && launch["aliceRpc"].as_str() == Some(source_rpc)
                && launch["bobRpc"].as_str() == Some(witness_rpc)
                && launch["identity"]["validatorCount"].as_u64() == Some(2)
                && launch["readiness"]["validatorCount"].as_u64() == Some(2)
                && launch["readiness"]["peerCounts"]["alice"].as_u64() == Some(1)
                && launch["readiness"]["peerCounts"]["bob"].as_u64() == Some(1),
            "source launch identity/endpoints do not match the campaign"
        );
        let raw_spec_sha256 = string(
            &launch["identity"]["rawSpecSha256"],
            "launch.identity.rawSpecSha256",
        )?;
        ensure!(
            raw_spec_sha256.trim_start_matches("0x").len() == 64
                && hex::decode(raw_spec_sha256.trim_start_matches("0x")).is_ok(),
            "source launch state has an invalid raw chain-spec SHA256"
        );

        let (source, witness) = Source::connect_pair(
            gear_rpc_client::GearApi::new(source_rpc, 3).await?,
            gear_rpc_client::GearApi::new(witness_rpc, 3).await?,
        )
        .await?;
        source.validate_attachment(&deployment["anchor"]).await?;
        ensure!(source.identity.runtime["runtimeCodeSha256"] == launch["identity"]["runtimeCodeSha256"],
            "common finalized source runtime :code SHA256 differs from immutable launch identity; HOLD");
        if schedule.handovers == 2 {
            validate_source_runtime_profile(
                Some(&launch["identity"]["runtimeProfile"]),
                &source.identity.runtime,
            )?;
            ensure!(
                launch["identity"]["domainBindingBlock"] == source.identity.domain_binding_block,
                "source domain binding changed since launch; HOLD"
            );
        }
        ensure!(
            source.source_genesis == witness.source_genesis
                && source.bridge_domain == witness.bridge_domain
                && source.mmr_start_block == witness.mmr_start_block
                && source.beefy_activation_block == witness.beefy_activation_block
                && format!("0x{}", hex::encode(source.source_genesis)) == source_genesis
                && format!("0x{}", hex::encode(source.bridge_domain)) == bridge_domain,
            "source/witness finalized identities differ from the immutable deployment manifest"
        );
        let launch_height: u32 = launch["readiness"]["commonFinalized"]["height"]
            .as_u64()
            .context("launch.readiness.commonFinalized.height missing")?
            .try_into()?;
        let launch_hash = bytes32(string(
            &launch["readiness"]["commonFinalized"]["hash"],
            "launch.readiness.commonFinalized.hash",
        )?)?;
        ensure!(
            source.api.block_number_to_hash(launch_height).await?.0 == launch_hash
                && witness.api.block_number_to_hash(launch_height).await?.0 == launch_hash,
            "launcher finalized checkpoint is not canonical on both source nodes"
        );
        let (source_set, source_next) = source.checkpoint_at(launch_height).await?;
        let (witness_set, witness_next) = witness.checkpoint_at_hash(GearHash(launch_hash)).await?;
        ensure!(
            source_set == witness_set
                && source_next == witness_next
                && source_set.keys.len() == 2
                && source_next.keys.len() == 2,
            "both nodes must expose identical two-validator current/next BEEFY sets"
        );

        let campaign_address = hoodi_wallet_address(campaign_wallet)?;
        ensure!(
            campaign_address != publisher_address
                && campaign_address != follower_address
                && publisher_address != follower_address,
            "campaign, follower, and root-publication EVM accounts must be distinct"
        );
        let campaign_pair = sr25519::Pair::from_string(campaign_suri, None)
            .context("derive campaign Gear signer")?;
        let governance_pair = sr25519::Pair::from_string(governance_suri, None)
            .context("derive Gear governance signer")?;
        let campaign_actor = ActorId::from(campaign_pair.public().0);
        let governance_actor = ActorId::from(governance_pair.public().0);
        ensure!(
            campaign_actor != governance_actor,
            "campaign and governance Gear signers must be distinct"
        );

        let gear_api = gclient::GearApi::builder()
            .suri(campaign_suri)
            .uri(source_rpc)
            .build()
            .await
            .context("connect campaign Gear signer")?;
        let remoting = GClientRemoting::new(gear_api.clone());
        let manager_id = actor_id(string(
            &stack["programs"]["vftManager"]["id"],
            "programs.vftManager.id",
        )?)?;
        let payment_id = actor_id(string(
            &stack["programs"]["bridgingPayment"]["id"],
            "programs.bridgingPayment.id",
        )?)?;
        let historical_proxy_id = actor_id(string(
            &stack["programs"]["historicalProxy"]["id"],
            "programs.historicalProxy.id",
        )?)?;
        let ethereum =
            Ethereum::connect_hoodi(ethereum_rpc, campaign_wallet, &deployment["ethereum"])
                .await
                .context("connect campaign account to Hoodi")?;
        ensure!(
            ethereum.receiver_address() == campaign_manager_address(deployment)?,
            "deployment receiver and ERC20 manager differ"
        );
        ethereum
            .verify_token_bindings(manager_id.into_bytes())
            .await?;
        let polling_eth = PollingEthApi::new(ethereum_rpc)
            .await
            .context("connect receipt-proof Ethereum client")?;
        let beacon =
            BeaconClient::new(beacon_rpc.to_owned(), Some(Duration::from_secs(15))).await?;
        verify_beacon_identity(&beacon, &ethereum).await?;

        let service = VftManager::new(remoting.clone());
        let finalized_gear_hash = source.api.latest_finalized_block().await?;
        let finalized_evm_height = ethereum.api.finalized_block_number().await?;
        let finalized_evm_block = ethereum
            .api
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(finalized_evm_height))
            .await?
            .context("finalized Hoodi block is missing")?;
        ensure!(
            service
                .erc_20_manager_address()
                .at_block(sails_block_hash(finalized_gear_hash))
                .recv(manager_id)
                .await?
                == Some(H160::from_slice(&ethereum.receiver_address())),
            "Gear manager ERC20 manager address differs from the deployment manifest"
        );
        ensure!(
            !service
                .is_paused()
                .at_block(sails_block_hash(finalized_gear_hash))
                .recv(manager_id)
                .await?
                && !service
                    .is_emergency_stopped()
                    .at_block(sails_block_hash(finalized_gear_hash))
                    .recv(manager_id)
                    .await?,
            "Gear token manager is paused or emergency-stopped"
        );
        let mappings = service
            .vara_to_eth_addresses()
            .at_block(sails_block_hash(finalized_gear_hash))
            .recv(manager_id)
            .await?;
        let registered = CampaignManager::new(
            ethereum.receiver_address().into(),
            ethereum.api.raw_provider().clone(),
        )
        .tokens()
        .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
        .call()
        .await?;
        let manager = CampaignManager::new(
            ethereum.receiver_address().into(),
            ethereum.api.raw_provider().clone(),
        );
        let mut tokens = Vec::with_capacity(4);
        for (symbol, component, peer_symbol, decimals) in [
            ("USDC", "circleVft", "hUSDC", 6),
            ("USDT", "tetherVft", "hUSDT", 6),
            ("WETH", "etherVft", "hWETH", 18),
            ("WBTC", "bitcoinVft", "hWBTC", 8),
        ] {
            let peer = actor_id(string(
                &stack["programs"][component]["id"],
                "token peer ID",
            )?)?;
            let mut found = None;
            for address in &registered {
                if manager
                    .getTokenType(*address)
                    .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
                    .call()
                    .await?
                    != 1
                {
                    continue;
                }
                let candidate = CampaignToken::new(*address, ethereum.api.raw_provider().clone());
                if candidate
                    .symbol()
                    .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
                    .call()
                    .await?
                    == symbol
                {
                    ensure!(
                        found.replace(*address).is_none(),
                        "duplicate Ethereum-origin {symbol} entries in EVM manager registry"
                    );
                }
            }
            let address =
                found.context(format!("{symbol} is not registered by the EVM manager"))?;
            ensure!(
                !tokens.iter().any(|token: &Token| token.address == address),
                "duplicate EVM token address in manager registry"
            );
            let token = CampaignToken::new(address, ethereum.api.raw_provider().clone());
            ensure!(
                token
                    .decimals()
                    .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
                    .call()
                    .await?
                    == decimals,
                "{symbol} ERC20 decimals mismatch"
            );
            ensure!(
                manager
                    .getTokenType(address)
                    .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
                    .call()
                    .await?
                    == 1,
                "{symbol} is not an Ethereum-supply token"
            );
            let mapping = mappings
                .iter()
                .find(|(mapped_peer, mapped_address, supply)| {
                    *mapped_peer == peer
                        && *supply == TokenSupply::Ethereum
                        && mapped_address.as_bytes() == address.as_slice()
                });
            ensure!(
                mapping.is_some(),
                "{symbol} VFT is not mapped to its Ethereum peer with Ethereum supply"
            );
            let metadata = VftMetadata::new(remoting.clone());
            ensure!(
                metadata
                    .symbol()
                    .at_block(sails_block_hash(finalized_gear_hash))
                    .recv(peer)
                    .await?
                    == peer_symbol
                    && metadata
                        .decimals()
                        .at_block(sails_block_hash(finalized_gear_hash))
                        .recv(peer)
                        .await?
                        == decimals,
                "{symbol} Gear VFT symbol/decimals mismatch"
            );
            let vft_admin = VftAdmin::new(remoting.clone());
            ensure!(
                vft_admin
                    .minter()
                    .at_block(sails_block_hash(finalized_gear_hash))
                    .recv(peer)
                    .await?
                    == manager_id
                    && vft_admin
                        .burner()
                        .at_block(sails_block_hash(finalized_gear_hash))
                        .recv(peer)
                        .await?
                        == manager_id,
                "{symbol} VFT minter/burner is not the configured manager"
            );
            tokens.push(Token {
                symbol,
                component,
                address,
                peer,
                gear_origin: false,
                native_amount: None,
                escrow: None,
            });
        }
        let ethereum_mappings = mappings
            .iter()
            .filter(|(_, _, supply)| *supply == TokenSupply::Ethereum)
            .count();
        ensure!(
            ethereum_mappings == 4,
            "Gear manager must have exactly four Ethereum-supply token peers"
        );
        let mut ethereum_registry_count = 0;
        for address in &registered {
            if manager
                .getTokenType(*address)
                .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
                .call()
                .await?
                == 1
            {
                ethereum_registry_count += 1;
            }
        }
        ensure!(
            ethereum_registry_count == 4 && tokens.len() == 4,
            "EVM manager registry must contain exactly four Ethereum-origin test tokens"
        );
        if schedule.handovers == 2 {
            let deposit = source
                .api
                .api
                .constants()
                .at(&gsdk::ext::subxt::dynamic::constant(
                    "Balances",
                    "ExistentialDeposit",
                ))?
                .as_type::<u128>()?;
            let witnessed = witness
                .api
                .api
                .constants()
                .at(&gsdk::ext::subxt::dynamic::constant(
                    "Balances",
                    "ExistentialDeposit",
                ))?
                .as_type::<u128>()?;
            ensure!(
                deposit == witnessed && deposit == 1_000_000_000_000,
                "normal native payout minimum differs from the pinned runtime; HOLD"
            );
            let wrapper = service
                .native_wrapper()
                .at_block(sails_block_hash(finalized_gear_hash))
                .recv(manager_id)
                .await?;
            for (symbol, component, native) in [
                ("GOT", "gearOriginVft", false),
                ("WTVARA", "nativeVft", true),
            ] {
                let peer = actor_id(string(
                    &stack["programs"][component]["id"],
                    "Gear-origin token peer",
                )?)?;
                let mut candidates = mappings
                    .iter()
                    .filter(|(mapped, _, supply)| *mapped == peer && *supply == TokenSupply::Gear);
                let (_, address, _) = candidates
                    .next()
                    .context("Gear-origin supply mapping is missing")?;
                ensure!(
                    candidates.next().is_none()
                        && !tokens.iter().any(|token| token.peer == peer
                            || token.address.as_slice() == address.as_bytes()),
                    "duplicate Gear-origin mapping; HOLD"
                );
                let address = Address::from_slice(address.as_bytes());
                ensure!(
                    manager
                        .getTokenType(address)
                        .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
                        .call()
                        .await?
                        == 2
                        && CampaignToken::new(address, ethereum.api.raw_provider().clone())
                            .decimals()
                            .block(BlockId::hash_canonical(finalized_evm_block.header.hash))
                            .call()
                            .await?
                            == 12,
                    "Gear-origin ERC20 configuration differs; HOLD"
                );
                ensure!(
                    VftMetadata::new(remoting.clone())
                        .decimals()
                        .at_block(sails_block_hash(finalized_gear_hash))
                        .recv(peer)
                        .await?
                        == 12
                        && wrapper
                            == Some(actor_id(string(
                                &stack["programs"]["nativeVft"]["id"],
                                "configured native wrapper"
                            )?)?),
                    "Gear-origin decimals/native manager binding differs; HOLD"
                );
                if native {
                    ensure!(
                        wrapper == Some(peer)
                            && vft_vara_client::NativeEscrow::new(remoting.clone())
                                .manager()
                                .at_block(sails_block_hash(finalized_gear_hash))
                                .recv(peer)
                                .await?
                                == Some(manager_id),
                        "native wrapper is not explicitly manager authorized; HOLD"
                    );
                } else {
                    ensure!(
                        wrapper != Some(peer),
                        "ordinary Gear VFT cannot be native settlement"
                    );
                }
                tokens.push(Token {
                    symbol,
                    component,
                    peer,
                    address,
                    gear_origin: true,
                    native_amount: native.then_some(u64::try_from(deposit)?),
                    escrow: Some(manager_id),
                });
            }
            ensure!(registered.len() == 6 && mappings.len() == 6, "normal campaign must retain four Ethereum tokens plus exact ordinary/native Gear mappings");
        }
        let payment_state = bridging_payment_client::BridgingPayment::new(remoting.clone())
            .get_state()
            .at_block(sails_block_hash(finalized_gear_hash))
            .recv(payment_id)
            .await?;
        ensure!(
            payment_state.fee != 0 && payment_state.priority_fee != 0,
            "normal and priority bridging fees must both be nonzero"
        );
        ensure!(
            payment_state.admin_address != ActorId::zero(),
            "bridging-payment administrator is unconfigured"
        );
        Ok(Self {
            source_rpc: source_rpc.to_owned(),
            witness_rpc: witness_rpc.to_owned(),
            ethereum_rpc: ethereum_rpc.to_owned(),
            source,
            witness,
            gear_api,
            remoting,
            manager_id,
            payment_id,
            historical_proxy_id,
            checkpoint_id: actor_id(string(&stack["checkpoint"], "checkpoint")?)?,
            campaign_actor,
            governance_actor,
            campaign_address,
            publisher_address,
            follower_address,
            campaign_wallet: campaign_wallet.to_owned(),
            governance_suri: governance_suri.to_owned(),
            follower_dir: follower_dir.to_owned(),
            inbound_dir: inbound_dir
                .canonicalize()
                .context("inbound worker directory is missing")?,
            inbound_start_block,
            outbound_dir: outbound_dir
                .canonicalize()
                .context("outbound worker directory is missing")?,

            deployment: deployment.clone(),
            ethereum,
            polling_eth,
            beacon,
            tokens,
            schedule,
            normal_fee: payment_state.fee,
            priority_fee: payment_state.priority_fee,
        })
    }

    fn journal_identity(
        &self,
        output_dir: &Path,
        deployment: &Value,
        stack: &Value,
        launch: &Value,
    ) -> Result<Journal> {
        let run_id = output_dir
            .file_name()
            .and_then(|name| name.to_str())
            .context("run output directory must have a name")?
            .to_owned();
        let source_genesis = format!("0x{}", hex::encode(self.source.source_genesis));
        let bridge_domain = format!("0x{}", hex::encode(self.source.bridge_domain));
        Ok(Journal::new(
            run_id,
            json_digest(deployment)?,
            json_digest(stack)?,
            json_digest(&launch["identity"])?,
            source_genesis,
            bridge_domain,
            string(
                &launch["identity"]["rawSpecSha256"],
                "launch.identity.rawSpecSha256",
            )?
            .to_owned(),
            json!({
                "sourceRpcDigest": endpoint_digest(&self.source_rpc),
                "witnessRpcDigest": endpoint_digest(&self.witness_rpc),
                "ethereumRpcDigest": endpoint_digest(&self.ethereum_rpc),
                "inboundDirectory": self.inbound_dir,
                "outboundDirectory": self.outbound_dir,
            }),
            json!({
                "campaignEvm": format!("{:#x}", self.campaign_address),
                "rootPublisherEvm": format!("{:#x}", self.publisher_address),
                "followerEvm": format!("{:#x}", self.follower_address),
                "campaignGear": format!("0x{}", hex::encode(self.campaign_actor.into_bytes())),
                "governanceGear": format!("0x{}", hex::encode(self.governance_actor.into_bytes())),
                "inboundEthereumStartBlock": self.inbound_start_block,
            }),
        ))
    }

    fn follower_state(&self) -> Result<Option<Value>> {
        let path = self.follower_dir.join("state.json");
        if !path.exists() {
            return Ok(None);
        }
        let state = read_validated_follower_state(&path, &self.deployment)?;
        let (follower, publisher) = follower_signers(&state, &self.deployment)?;
        ensure!(
            follower == self.follower_address && publisher == self.publisher_address,
            "token follower or root-publisher signer changed"
        );
        Ok(Some(state))
    }

    fn inbound_journal(&self) -> Result<Option<InboundJournal>> {
        let Some(state) = read_worker_json::<InboundJournal>(&self.inbound_dir.join("state.json"))?
        else {
            return Ok(None);
        };
        let expected = InboundRuntimeIdentity {
            ethereum_chain_id: number(&self.deployment["ethereum"]["chainId"])?,
            ethereum_genesis_hash: bytes32(HOODI_EXECUTION_GENESIS)?.into(),
            ethereum_start_block: self.inbound_start_block,
            erc20_manager_address: Some(campaign_manager_address(&self.deployment)?.into()),
            bridging_payment_address: None,
            gear_genesis_hash: self.source.source_genesis.into(),
            vft_manager_address: self.manager_id.into_bytes().into(),
            checkpoint_light_client_address: self.checkpoint_id.into_bytes().into(),
            historical_proxy_address: self.historical_proxy_id.into_bytes().into(),
            gear_sender: state.runtime_identity.gear_sender,
        };
        state.validate_identity(&expected, self.campaign_actor, self.governance_actor)?;
        Ok(Some(state))
    }

    async fn worker_readiness(
        &self,
        allowed_queued: &BTreeMap<String, OutboundRequest>,
    ) -> Result<Value> {
        // Capture targets before loading advancing journals; later canonical cursors may cover them.
        let source_hash = self.source.api.latest_finalized_block().await?;
        let source_height = self.source.api.block_hash_to_number(source_hash).await?;
        let ethereum_height = self
            .ethereum
            .api
            .verified_finalized_view()
            .await?
            .block_number();
        let target_header = self.beacon.get_block_header_finalized().await?;
        if !outbound_save_complete(&self.outbound_dir)? {
            return Ok(json!({"ready":false,"waitingFor":"outbound durable snapshot completion"}));
        }
        let status_path = self.outbound_dir.join("transaction_status.json");
        let Some(outbound_status) = read_worker_json::<OutboundTransactionStatus>(&status_path)?
        else {
            return Ok(json!({"ready":false,"waitingFor":"outbound transaction_status.json"}));
        };
        let outbound_idle = outbound_status_idle(&outbound_status)?;
        let Some(inbound) = self.inbound_journal()? else {
            return Ok(json!({"ready":false,"waitingFor":"inbound state.json"}));
        };
        let Some(blocks) =
            read_worker_json::<BTreeMap<u64, InboundBlock>>(&self.inbound_dir.join("blocks.json"))?
        else {
            return Ok(json!({"ready":false,"waitingFor":"inbound blocks.json"}));
        };
        let Some(events) = read_worker_json::<Value>(&self.outbound_dir.join("gear_events.json"))?
        else {
            return Ok(json!({"ready":false,"waitingFor":"outbound gear_events.json"}));
        };
        let Some(root_cursor) =
            read_worker_json::<u64>(&self.outbound_dir.join("ethereum_root_cursor"))?
        else {
            return Ok(json!({"ready":false,"waitingFor":"outbound ethereum_root_cursor"}));
        };
        ensure!(
            events["version"] == 2,
            "outbound event journal lacks bound fee-exemption policy (schema 2 required)"
        );
        ensure!(
            bytes32(string(&events["genesis_hash"], "worker genesis_hash")?)?
                == self.source.source_genesis,
            "outbound worker follows a different source genesis"
        );
        let mut cursors_ready = true;
        for field in ["queued_cursor", "paid_cursor"] {
            if events[field].is_null() {
                cursors_ready = false;
                continue;
            }
            let height = u32::try_from(number(&events[field]["block"])?)?;
            let latest_source = self
                .source
                .api
                .block_hash_to_number(self.source.api.latest_finalized_block().await?)
                .await?;
            let latest_witness = self
                .witness
                .api
                .block_hash_to_number(self.witness.api.latest_finalized_block().await?)
                .await?;
            ensure!(
                height <= latest_source && height <= latest_witness,
                "outbound cursor is ahead of canonical source/witness finality"
            );
            let expected = bytes32(string(&events[field]["hash"], "worker cursor hash")?)?;
            let canonical = self.source.api.block_number_to_hash(height).await?;
            ensure!(
                canonical.as_bytes() == expected
                    && self.witness.api.block_number_to_hash(height).await? == canonical,
                "outbound cursor does not match both canonical source nodes"
            );
            cursors_ready &= height >= source_height;
        }
        let queued_observations = events["queued"]
            .as_object()
            .context("outbound queued observations missing")?;
        let queued = queued_observations.len();
        let queued_ready = queued_work_matches(queued_observations, allowed_queued)?;
        let paid = events["paid"]
            .as_object()
            .context("outbound paid observations missing")?
            .len();
        let pairs = events["pending_pairs"]
            .as_object()
            .context("outbound pending pairs missing")?
            .len();
        let pending_blocks = blocks
            .values()
            .filter(|b| !b.transactions.is_empty())
            .count();
        let inbound_cursor = blocks.values().map(|b| b.number.0).max();
        let checkpoint = checkpoint_light_client_client::ServiceState::new(self.remoting.clone())
            .get(Order::Reverse, 0, 1)
            .at_block(sails_block_hash(source_hash))
            .recv(self.checkpoint_id)
            .await
            .context("query finalized checkpoint state")?;
        let mut checkpoint_ready = false;
        let mut checkpoint_slot = None;
        if let Some((slot, root)) = checkpoint.checkpoints.first() {
            let header = self.beacon.get_block_header(*slot).await?;
            let canonical = header.tree_hash_root();
            let canonical_bytes: &[u8] = canonical.as_ref();
            ensure!(
                header.slot == *slot && canonical_bytes == root.as_ref(),
                "checkpoint program root does not match the canonical Hoodi beacon header"
            );
            checkpoint_slot = Some(*slot);
            checkpoint_ready = *slot >= target_header.slot && checkpoint.replay_back.is_none();
        }
        let beacon_ready =
            check_beacon_el_finality(&self.beacon, &self.ethereum, ethereum_height).await?;
        if !outbound_save_complete(&self.outbound_dir)?
            || read_worker_json::<OutboundTransactionStatus>(&status_path)?.as_ref()
                != Some(&outbound_status)
        {
            return Ok(
                json!({"ready":false,"waitingFor":"consistent outbound transaction status observation"}),
            );
        }
        let ready = cursors_ready
            && outbound_idle
            && root_cursor >= ethereum_height
            && inbound_cursor.is_some_and(|n| n >= ethereum_height)
            && inbound.transactions.is_empty()
            && inbound.failed.is_empty()
            && pending_blocks == 0
            && queued_ready
            && paid == 0
            && pairs == 0
            && checkpoint_ready
            && beacon_ready;
        Ok(json!({
            "ready":ready,"sourceFinalizedHeight":source_height,"sourceFinalizedHash":format!("{source_hash:#x}"),
            "ethereumFinalizedHeight":ethereum_height,"inboundCursor":inbound_cursor,
            "inboundActive":inbound.transactions.len(),"inboundFailed":inbound.failed.len(),
            "inboundPendingBlocks":pending_blocks,"queuedCursor":events["queued_cursor"],
            "paidCursor":events["paid_cursor"],"outboundRootCursor":root_cursor,
            "outboundQueued":queued,"outboundPaid":paid,"outboundPendingPairs":pairs,
            "outboundActive":outbound_status.active.len(),"outboundFailed":outbound_status.failed.len(),
            "checkpointProgram":format!("0x{}",hex::encode(self.checkpoint_id.into_bytes())),"checkpointSlot":checkpoint_slot,
            "requiredBeaconSlot":target_header.slot,"checkpointReady":checkpoint_ready,"beaconReady":beacon_ready,
        }))
    }

    async fn snapshot(&self) -> Result<SnapshotSet> {
        self.snapshot_at(None).await
    }

    async fn snapshot_at(&self, saved: Option<&SnapshotSet>) -> Result<SnapshotSet> {
        acquire_snapshot(
            &self.source.api,
            &self.witness.api,
            &self.ethereum,
            &self.remoting,
            &self.tokens,
            self.campaign_actor,
            self.campaign_address,
            saved,
        )
        .await
    }

    async fn fee_state(&self) -> Result<(u128, u128)> {
        let state = bridging_payment_client::BridgingPayment::new(self.remoting.clone())
            .get_state()
            .recv(self.payment_id)
            .await?;
        ensure!(
            state.fee == self.normal_fee && state.priority_fee == self.priority_fee,
            "bridging fee policy changed after preflight"
        );
        Ok((state.fee, state.priority_fee))
    }
}

async fn verify_beacon_identity(beacon: &BeaconClient, ethereum: &Ethereum) -> Result<()> {
    let genesis = beacon
        .get_genesis()
        .await
        .context("read Hoodi beacon genesis")?;
    ensure!(
        genesis.data.genesis_validators_root.as_slice()
            == bytes32(HOODI_GENESIS_VALIDATORS_ROOT)?.as_slice(),
        "beacon endpoint is not Hoodi"
    );
    ensure!(
        check_beacon_el_finality(beacon, ethereum, 0).await?,
        "Hoodi EL has not synchronized to the finalized beacon execution payload"
    );
    Ok(())
}

async fn check_beacon_el_finality(
    beacon: &BeaconClient,
    ethereum: &Ethereum,
    required_el_block: u64,
) -> Result<bool> {
    let finalized = beacon
        .get_block_finalized::<Value>()
        .await
        .context("read finalized Hoodi beacon block")?;
    let payload = &finalized["body"]["execution_payload"];
    let number = string(&payload["block_number"], "beacon execution block number")?
        .parse::<u64>()
        .context("parse beacon execution block number")?;
    let expected = bytes32(string(
        &payload["block_hash"],
        "beacon execution block hash",
    )?)?;
    let finalized_el = ethereum.api.finalized_block_number().await?;
    let block = ethereum
        .api
        .raw_provider()
        .get_block_by_number(BlockNumberOrTag::Number(number))
        .await?;
    beacon_coverage(
        required_el_block,
        number,
        expected,
        finalized_el,
        block.map(|b| b.header.hash.0),
    )
}

fn beacon_coverage(
    required: u64,
    beacon: u64,
    expected: [u8; 32],
    finalized_el: u64,
    canonical: Option<[u8; 32]>,
) -> Result<bool> {
    match canonical {
        Some(hash) => ensure!(
            hash == expected,
            "Hoodi beacon/EL finalized execution hashes disagree"
        ),
        None => ensure!(
            finalized_el < beacon,
            "finalized EL block claimed by Beacon is missing"
        ),
    }
    Ok(canonical.is_some() && beacon >= required && finalized_el >= beacon)
}

async fn run_preflight(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    readiness_deadline: Instant,
) -> Result<()> {
    journal.validate_wall_time(now_ms()?)?;
    ensure!(journal.t0_ms.is_none(), "preflight cannot run after T0");
    ensure!(
        journal.preflight.status != "passed",
        "both Hoodi preflight batches are already complete"
    );
    ensure!(
        journal.preflight.status != "failed",
        "failed preflight is immutable; use a separately labeled run"
    );
    journal.preflight.status = "running".into();
    if journal.preflight.started_at_ms.is_none() {
        journal.preflight.started_at_ms = Some(now_ms()?);
    }
    save_journal(path, journal)?;
    match run_preflight_inner(ctx, journal, path, readiness_deadline).await {
        Ok(()) => {
            journal.preflight.status = "passed".into();
            journal.preflight.completed_at_ms = Some(now_ms()?);
            journal.status = "preflight_passed".into();
            save_journal(path, journal)
        }
        Err(error) => {
            journal.preflight.status = "failed".into();
            journal.preflight.evidence["failure"] = json!(format!("{error:#}"));
            journal.status = "failed_before_t0".into();
            journal.incidents.push(
                json!({"atMs": now_ms()?, "phase":"preflight", "error":format!("{error:#}")}),
            );
            save_journal(path, journal)?;
            Err(error)
        }
    }
}

fn governance_pause_completed(journal: &Journal) -> Result<bool> {
    let evidence = &journal.preflight.evidence["governancePause"];
    if evidence.is_null() {
        ensure!(
            journal.windows.is_empty(),
            "preflight windows have no completed governance probe; HOLD"
        );
        return Ok(false);
    }
    ensure!(
        evidence["status"] == "passed"
            && evidence["pausedBlockHash"].is_string()
            && evidence["unpausedBlockHash"].is_string()
            && evidence["rejectedAs"] == "Paused"
            && evidence["balancesInFlight"] == false,
        "governance pause probe is incomplete; HOLD and reconcile the original intent without repeating or automatically unpausing it"
    );
    Ok(true)
}

fn validate_preflight_setup(journal: &Journal, baseline: &SnapshotSet) -> Result<bool> {
    ensure_preflight_probe_boundary(journal)?;
    let completed = governance_pause_completed(journal)?;
    if !journal
        .windows
        .values()
        .any(|window| window["status"] == "running")
    {
        for (symbol, state) in &baseline.assets {
            ensure!(
                state.has_no_bridge_liabilities(),
                "{symbol} baseline contains pre-existing bridge custody/liabilities"
            );
            if state.gear_escrow.is_none() {
                ensure!(
                    state.evm_user >= EthU256::from(CAMPAIGN_ALLOWANCE),
                    "campaign account needs at least24raw {symbol} units"
                );
            }
        }
    }
    Ok(!completed)
}

async fn run_preflight_inner(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    readiness_deadline: Instant,
) -> Result<()> {
    require_queue_bootstrap(ctx, journal, path, readiness_deadline).await?;
    let baseline = ctx.snapshot().await?;
    let run_governance_probe = validate_preflight_setup(journal, &baseline)?;
    if ctx.schedule.handovers == 2
        && !journal
            .windows
            .values()
            .any(|window| window["status"] == "running")
    {
        let remaining = ["preflight-normal", "preflight-priority"]
            .iter()
            .filter(|label| {
                journal
                    .windows
                    .get(**label)
                    .is_none_or(|window| window["status"] != "passed")
            })
            .count() as u64;
        for token in ctx.tokens.iter().filter(|token| token.gear_origin) {
            let state = baseline
                .assets
                .get(token.symbol)
                .context("source inventory missing")?;
            ensure!(state.gear_user>=GearU256::from(token.raw_amount(remaining)?),"campaign source inventory cannot cover remaining ordinary/native original batches; HOLD");
        }
    }
    ctx.fee_state().await?;
    if run_governance_probe {
        governance_pause_test(ctx, journal, path).await?;
    }
    let mut readiness_deadline = readiness_deadline;
    let mut allowed_queued = BTreeMap::new();
    for (label, fee_mode) in [
        ("preflight-normal", FeeMode::Normal),
        ("preflight-priority", FeeMode::Priority),
    ] {
        if journal
            .windows
            .get(label)
            .is_some_and(|window| window["status"] == "running")
        {
            let (_, deadline_ms) = start_preflight_window(journal, label, fee_mode, now_ms()?)?;
            readiness_deadline = readiness_deadline.min(instant_deadline(deadline_ms)?);
            let mut requests = Vec::new();
            for token in &ctx.tokens {
                let id = format!("{label}/{}-gear-burn", token.symbol);
                if let Some(action) = journal.actions.get(&id) {
                    let start = u32::try_from(number(&action.intent["startBlock"])?)?;
                    ensure!(
                        action.intent
                            == json!({"kind":"vft-manager-request","peer":token.component,"amountRaw":token.raw_amount(WINDOW_AMOUNT)?,"receiver":format!("{:#x}",ctx.campaign_address),"startBlock":start}),
                        "saved preflight burn intent changed; HOLD"
                    );
                    requests.push((id, token.symbol.to_owned(), start));
                }
            }
            if let Some(start) = requests.iter().map(|(_, _, start)| *start).min() {
                for (_, _, request) in reconcile_outbound_requests(
                    ctx,
                    journal,
                    path,
                    &requests,
                    start,
                    GearU256::from(WINDOW_AMOUNT),
                    readiness_deadline,
                )
                .await?
                {
                    let mut nonce = [0; 32];
                    request.nonce.to_big_endian(&mut nonce);
                    allowed_queued.insert(hex::encode(nonce), request);
                }
            }
        }
    }
    lanes_ready(
        ctx,
        journal,
        path,
        "preflight",
        baseline.gear_height,
        readiness_deadline,
        &allowed_queued,
    )
    .await?;
    for (label, fee_mode) in [
        ("preflight-normal", FeeMode::Normal),
        ("preflight-priority", FeeMode::Priority),
    ] {
        if journal
            .windows
            .get(label)
            .is_some_and(|value| value["status"] == "passed")
        {
            continue;
        }
        let (started, deadline_ms) = start_preflight_window(journal, label, fee_mode, now_ms()?)?;
        save_journal(path, journal)?;
        let deadline = instant_deadline(deadline_ms)?;
        let result = timeout_at(
            deadline.into(),
            run_roundtrip(
                ctx,
                journal,
                path,
                label,
                fee_mode,
                WINDOW_AMOUNT,
                label == "preflight-normal",
                deadline,
            ),
        )
        .await
        .map_err(|_| {
            anyhow!("original preflight window deadline expired; preserve pending intents and HOLD")
        })
        .and_then(|result| result);
        if let Err(error) = result {
            journal.windows.get_mut(label).expect("window was inserted")["status"] =
                json!("failed");
            journal.windows.get_mut(label).expect("window was inserted")["failure"] =
                json!(format!("{error:#}"));
            save_journal(path, journal)?;
            return Err(error);
        }
        let completed = now_ms()?;
        let elapsed = completed
            .checked_sub(started)
            .context("preflight clock moved backwards")?;
        ensure!(
            completed < deadline_ms,
            "{label} exceeded its original preflight deadline"
        );
        let window = journal.windows.get_mut(label).expect("window was inserted");
        window["status"] = json!("passed");
        window["completedAtMs"] = json!(completed);
        window["elapsedMs"] = json!(elapsed);
        save_journal(path, journal)?;
    }
    journal.preflight.evidence["batches"] = json!([
        journal.windows["preflight-normal"].clone(),
        journal.windows["preflight-priority"].clone()
    ]);
    journal.preflight.evidence["assets"] = json!(ctx
        .tokens
        .iter()
        .map(|token| token.symbol)
        .collect::<Vec<_>>());
    journal.preflight.evidence["fourAssetsPerBatch"] = json!(true);
    journal.preflight.evidence["normalAndPriorityPayments"] = json!(true);
    ensure!(
        receipt_probes_completed(journal)?,
        "both live receipt probes are required before passing preflight; HOLD"
    );
    Ok(())
}
fn start_preflight_window(
    journal: &mut Journal,
    label: &str,
    fee_mode: FeeMode,
    now: u64,
) -> Result<(u64, u64)> {
    let window = journal.windows.entry(label.into()).or_insert_with(|| {
        json!({
            "status":"running", "startedAtMs":now,
            "deadlineMs":now.checked_add(PRELIGHT_MAX_SECS * 1_000),
            "feeMode":fee_mode, "amountRaw":"1", "assets":{}, "stages":{}
        })
    });
    ensure!(
        window["status"] == "running",
        "preflight window is not resumable"
    );
    let started = window["startedAtMs"]
        .as_u64()
        .context("missing original preflight start")?;
    let deadline = window["deadlineMs"]
        .as_u64()
        .context("missing original preflight deadline")?;
    ensure!(
        deadline.checked_sub(started).is_some_and(|duration| {
            duration == 44 * 60 * 1_000 || duration == PRELIGHT_MAX_SECS * 1_000
        }),
        "preflight deadline has an unrecognized batch duration"
    );
    ensure!(
        now >= started && now < deadline,
        "original preflight window has expired or its clock moved backwards"
    );
    Ok((started, deadline))
}

async fn normal_governance_pause_test(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    api: &gclient::GearApi,
    config: &vft_manager_client::Config,
) -> Result<()> {
    let before = ctx.snapshot().await?;
    if journal.preflight.evidence["governancePause"].is_null() {
        ensure!(
            !VftManager::new(ctx.remoting.clone())
                .is_paused()
                .at_block(sails_block_hash(before.gear_hash))
                .recv(ctx.manager_id)
                .await?,
            "normal governance probe cannot adopt another owner's pause; HOLD"
        );
        journal.preflight.evidence["governancePause"] = json!({"status":"pause-intent","startedAtMs":now_ms()?,"balancesInFlight":false,"beforeSnapshot":before.to_json(),"signer":format!("0x{}",hex::encode(ctx.governance_actor.into_bytes()))});
        save_journal(path, journal)?;
    }
    let saved =
        SnapshotSet::from_json(&journal.preflight.evidence["governancePause"]["beforeSnapshot"])?;
    ctx.snapshot_at(Some(&saved)).await?;
    let deadline = instant_deadline(
        number(&journal.preflight.evidence["governancePause"]["startedAtMs"])?
            .checked_add(PRELIGHT_MAX_SECS * 1000)
            .context("governance deadline overflow")?,
    )?;
    let pause_id = "preflight/governance-pause";
    signed_normal_message(ctx, (api, ctx.governance_actor), (journal, path), (pause_id, json!({"kind":"governance-pause","manager":format!("0x{}",hex::encode(ctx.manager_id.into_bytes()))})), (ctx.manager_id, vft_manager_client::vft_manager::io::Pause::encode_call(), 0), deadline).await?;
    let reply = normal_message_reply(
        ctx,
        ctx.governance_actor,
        journal,
        path,
        pause_id,
        ctx.manager_id,
        deadline,
    )
    .await?;
    decode_campaign_reply::<()>(&reply, vft_manager_client::vft_manager::io::Pause::ROUTE)?;
    let paused_hash = GearHash::from_str(string(
        &journal.actions[pause_id].evidence["originalReply"]["finalizedHash"],
        "original pause pin",
    )?)?;
    ensure!(
        VftManager::new(ctx.remoting.clone())
            .is_paused()
            .at_block(sails_block_hash(paused_hash))
            .recv(ctx.manager_id)
            .await?,
        "normal governance original pause lacks finalized effect; HOLD"
    );
    let token = ctx
        .tokens
        .first()
        .context("mapped pause probe token missing")?;
    let request_id = "preflight/governance-paused-request";
    signed_normal_message(ctx, (api, ctx.governance_actor), (journal, path), (request_id, json!({"kind":"governance-paused-request","token":token.component,"amountRaw":1,"managerFee":config.fee_incoming.to_string()})), (ctx.manager_id, vft_manager_client::vft_manager::io::RequestBridging::encode_call(token.peer,GearU256::from(1u8),H160::from_slice(ctx.campaign_address.as_slice())), config.fee_incoming), deadline).await?;
    let reply = normal_message_reply(
        ctx,
        ctx.governance_actor,
        journal,
        path,
        request_id,
        ctx.manager_id,
        deadline,
    )
    .await?;
    let rejection = decode_campaign_reply::<Result<(GearU256, H160), vft_manager_client::Error>>(
        &reply,
        vft_manager_client::vft_manager::io::RequestBridging::ROUTE,
    )?;
    ensure!(
        matches!(rejection, Err(vft_manager_client::Error::Paused)),
        "mapped normal request was not rejected specifically by pause; HOLD"
    );
    let unpause_id = "preflight/governance-unpause";
    signed_normal_message(ctx, (api, ctx.governance_actor), (journal, path), (unpause_id, json!({"kind":"governance-unpause","originalPause":pause_id,"originalRejection":request_id})), (ctx.manager_id, vft_manager_client::vft_manager::io::Unpause::encode_call(), 0), deadline).await?;
    let reply = normal_message_reply(
        ctx,
        ctx.governance_actor,
        journal,
        path,
        unpause_id,
        ctx.manager_id,
        deadline,
    )
    .await?;
    decode_campaign_reply::<()>(&reply, vft_manager_client::vft_manager::io::Unpause::ROUTE)?;
    let unpaused_hash = GearHash::from_str(string(
        &journal.actions[unpause_id].evidence["originalReply"]["finalizedHash"],
        "original unpause pin",
    )?)?;
    ensure!(
        !VftManager::new(ctx.remoting.clone())
            .is_paused()
            .at_block(sails_block_hash(unpaused_hash))
            .recv(ctx.manager_id)
            .await?,
        "normal governance unpause lacks finalized effect; HOLD"
    );
    let after = ctx.snapshot().await?;
    verify_roundtrip_delta(&saved, &after, &ctx.tokens)?;
    let view = &mut journal.preflight.evidence["governancePause"];
    view["pausedBlockHash"] = json!(format!("{paused_hash:#x}"));
    view["unpausedBlockHash"] = json!(format!("{unpaused_hash:#x}"));
    view["rejectedAs"] = json!("Paused");
    view["status"] = json!("passed");
    save_journal(path, journal)
}

async fn governance_pause_test(ctx: &Context, journal: &mut Journal, path: &Path) -> Result<()> {
    let api = gclient::GearApi::builder()
        .suri(&ctx.governance_suri)
        .uri(&ctx.source_rpc)
        .build()
        .await
        .context("connect protected Gear governance signer")?;
    let remoting = GClientRemoting::new(api.clone());
    let mut manager = VftManager::new(remoting);
    let admin = manager.admin().recv(ctx.manager_id).await?;
    let pause_admin = manager.pause_admin().recv(ctx.manager_id).await?;
    ensure!(
        ctx.governance_actor == admin || ctx.governance_actor == pause_admin,
        "governance signer is not the configured Gear manager admin or pause admin"
    );
    let config = manager.get_config().recv(ctx.manager_id).await?;
    ensure!(
        !governance_pause_completed(journal)?,
        "governance probe already completed"
    );
    if ctx.schedule.handovers == 2 {
        return normal_governance_pause_test(ctx, journal, path, &api, &config).await;
    }
    journal.preflight.evidence["governancePause"] = json!({
        "status":"pause-intent","startedAtMs":now_ms()?,
        "admin":format!("0x{}",hex::encode(admin.into_bytes())),
        "pauseAdmin":format!("0x{}",hex::encode(pause_admin.into_bytes())),
        "signer":format!("0x{}",hex::encode(ctx.governance_actor.into_bytes())),
        "manager":format!("0x{}",hex::encode(ctx.manager_id.into_bytes())),
        "beforeBlockHash":format!("{:#x}",ctx.source.api.latest_finalized_block().await?),
        "balancesInFlight":false,
    });
    save_journal(path, journal)?;
    manager.pause().send_recv(ctx.manager_id).await?;
    let paused_hash = ctx.source.api.latest_finalized_block().await?;
    ensure!(
        manager
            .is_paused()
            .at_block(sails_block_hash(paused_hash))
            .recv(ctx.manager_id)
            .await?,
        "Gear manager pause did not finalize"
    );
    journal.preflight.evidence["governancePause"]["pausedBlockHash"] =
        json!(format!("{paused_hash:#x}"));
    journal.preflight.evidence["governancePause"]["status"] = json!("paused-request-intent");
    save_journal(path, journal)?;
    let token = ctx.tokens.first().context("four-asset mapping is empty")?;
    let paused_request = manager
        .request_bridging(
            token.peer,
            GearU256::from(1u8),
            H160::from_slice(ctx.campaign_address.as_slice()),
        )
        .with_value(config.fee_incoming)
        .with_gas_limit(api.block_gas_limit()?)
        .send_recv(ctx.manager_id)
        .await?;
    ensure!(
        paused_request.is_err() && format!("{paused_request:?}").contains("Paused"),
        "a real mapped-token request was not rejected specifically by the manager pause"
    );
    journal.preflight.evidence["governancePause"]["rejectedAs"] = json!("Paused");
    journal.preflight.evidence["governancePause"]["status"] = json!("unpause-intent");
    save_journal(path, journal)?;
    manager.unpause().send_recv(ctx.manager_id).await?;
    let unpaused_hash = ctx.source.api.latest_finalized_block().await?;
    ensure!(
        !manager
            .is_paused()
            .at_block(sails_block_hash(unpaused_hash))
            .recv(ctx.manager_id)
            .await?,
        "Gear manager unpause did not finalize"
    );
    journal.preflight.evidence["governancePause"]["unpausedBlockHash"] =
        json!(format!("{unpaused_hash:#x}"));
    journal.preflight.evidence["governancePause"]["status"] = json!("passed");
    save_journal(path, journal)
}

async fn run_warmup(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    rotation_suri: &str,
    rotation_authority: &str,
) -> Result<()> {
    journal.validate_wall_time(now_ms()?)?;
    ensure!(
        receipt_probes_completed(journal)?,
        "warmup requires both original live receipt probes; HOLD"
    );
    ensure!(
        journal.t0_ms.is_none(),
        "warmup cannot change an existing T0"
    );
    ensure!(
        journal.preflight.status == "passed",
        "warmup requires two successful four-asset Hoodi preflight batches"
    );
    if journal.warmup.status == "pending" {
        let started = now_ms()?;
        journal.warmup.status = "running".into();
        journal.warmup.evidence = json!({
            "readinessStartedAtMs":started,
            "readinessDeadlineAtMs":started.checked_add(30 * 60_000).context("readiness deadline overflow")?,
            "durationRequiredMs":ctx.schedule.warmup_secs * 1_000,"samples":[],
        });
        save_journal(path, journal)?;
    } else {
        ensure!(journal.warmup.status == "running" && journal.warmup.started_at_ms.is_none()
            && !journal.warmup.evidence["rotationIntent"].is_null(),
            "warmup cannot restart, backfill missed samples or rewrite an original failed/passed attempt; HOLD");
    }
    match run_warmup_inner(ctx, journal, path, rotation_suri, rotation_authority).await {
        Ok(()) => {
            journal.warmup.status = "passed".into();
            journal.warmup.completed_at_ms = Some(now_ms()?);
            journal.status = "warmup_passed".into();
            save_journal(path, journal)
        }
        Err(error) => {
            journal.warmup.status = "failed".into();
            journal.warmup.evidence["failure"] = json!(format!("{error:#}"));
            journal.status = "failed_before_t0".into();
            journal
                .incidents
                .push(json!({"atMs":now_ms()?,"phase":"warmup","error":format!("{error:#}")}));
            save_journal(path, journal)?;
            Err(error)
        }
    }
}

fn prepare_rotation_handoff(
    journal: &mut Journal,
    path: &Path,
    intent: Value,
    at_ms: u64,
) -> Result<bool> {
    let deadline = number(&journal.warmup.evidence["readinessDeadlineAtMs"])?;
    ensure!(at_ms < deadline && journal.warmup.started_at_ms.is_none(),
        "rotation cannot exceed its original readiness deadline or restart the sampling clock; HOLD");
    let evidence = &mut journal.warmup.evidence;
    if !evidence["rotationIntent"].is_null() {
        ensure!(
            evidence["rotationIntent"] == intent,
            "original rotation nonce/key/signed identity changed; HOLD"
        );
    } else {
        ensure!(
            evidence["rotationHandoffAtMs"].is_null(),
            "rotation handoff has no original intent; HOLD"
        );
        evidence["rotationIntent"] = intent;
    }
    if !journal.warmup.evidence["rotationHandoffAtMs"].is_null() {
        return Ok(false);
    }
    journal.warmup.evidence["rotationHandoffAtMs"] = json!(at_ms);
    save_journal(path, journal)?;
    Ok(true)
}

async fn authenticate_normal_handover(
    ctx: &Context,
    original: &Value,
    previous: &Value,
) -> Result<Value> {
    ensure!(original["finalized"] == true && original["current"] == previous["next"]
        && number(&original["current"]["id"])? == number(&previous["current"]["id"])? + 1
        && original["clientAddress"] == format!("{:#x}", ctx.ethereum.client_address),
        "normal handover must be sequential, authenticated and accepted by the original active client; HOLD");
    let block = u32::try_from(number(&original["block"])?)?;
    let raw = hex::decode(
        string(&original["rawScale"], "original handover SCALE")?
            .strip_prefix("0x")
            .context("handover SCALE hex prefix")?,
    )?;
    let (signed, witnessed) = tokio::try_join!(
        ctx.source.recapture(block, raw.clone()),
        ctx.witness.recapture(block, raw)
    )?;
    ensure!(
        signed.block_hash == bytes32(string(&original["blockHash"], "original handover hash")?)?
            && signed.current == witnessed.current
            && signed.next == witnessed.next
            && signed.block_hash == witnessed.block_hash
            && signed.validated.commitment_hash == witnessed.validated.commitment_hash
            && signed.validated.signed_indices.len() == 2
            && witnessed.validated.signed_indices.len() == 2
            && serde_json::to_value(&signed.current)? == original["current"]
            && serde_json::to_value(&signed.next)? == original["next"],
        "normal handover lacks both genuine canonical validator signatures; HOLD"
    );
    let proof = ctx
        .source
        .proof(
            block.checked_sub(1).context("handover at block zero")?,
            &signed,
        )
        .await?;
    let hash: B256 = string(&original["txHash"], "original handover transaction")?.parse()?;
    let destination = ctx
        .ethereum
        .verify_accepted_commitment(hash, &signed, &proof.snapshot)
        .await?;
    let receipt = ctx
        .ethereum
        .api
        .get_finalized_receipt(hash)
        .await?
        .context("original handover is not canonically finalized; HOLD")?;
    ensure!(
        destination.0 == receipt.included_block_number
            && destination.1 == receipt.included_block_hash
            && original["destinationBlock"].as_u64() == Some(destination.0)
            && bytes32(string(
                &original["destinationHash"],
                "original handover destination"
            )?)? == destination.1 .0,
        "original handover finality/receipt identity changed; HOLD"
    );
    Ok(
        json!({"original":original,"current":original["current"],"next":original["next"],"observedAtMs":now_ms()?}),
    )
}

async fn run_warmup_inner(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    rotation_suri: &str,
    rotation_authority: &str,
) -> Result<()> {
    let readiness_deadline =
        instant_deadline(number(&journal.warmup.evidence["readinessDeadlineAtMs"])?)?;
    lanes_ready(
        ctx,
        journal,
        path,
        "before_rotation",
        0,
        readiness_deadline,
        &BTreeMap::new(),
    )
    .await?;
    let initial_hash = if journal.warmup.evidence["initialSourceHash"].is_null() {
        ctx.source.api.latest_finalized_block().await?
    } else {
        string(
            &journal.warmup.evidence["initialSourceHash"],
            "original pre-rotation source hash",
        )?
        .parse()?
    };
    let initial_height = ctx.source.api.block_hash_to_number(initial_hash).await?;
    ensure!(
        ctx.witness.api.block_number_to_hash(initial_height).await? == initial_hash,
        "original pre-rotation finalized source differs across canonical nodes; HOLD"
    );
    let (initial_current, initial_next) = ctx.source.checkpoint_at_hash(initial_hash).await?;
    ensure!(
        ctx.witness.checkpoint_at_hash(initial_hash).await?
            == (initial_current.clone(), initial_next.clone()),
        "pre-rotation source authority sets differ; HOLD"
    );
    ensure!(
        initial_current.keys.len() == 2 && initial_next.keys.len() == 2,
        "source does not have exactly two BEEFY validators"
    );
    if journal.warmup.evidence["initialSourceHash"].is_null() {
        journal.warmup.evidence["initialSourceHash"] = json!(format!("{initial_hash:#x}"));
        journal.warmup.evidence["initialCurrent"] = serde_json::to_value(&initial_current)?;
        journal.warmup.evidence["initialNext"] = serde_json::to_value(&initial_next)?;
        journal.warmup.evidence["rotationRequestedAtMs"] = json!(now_ms()?);
        save_journal(path, journal)?;
    } else {
        ensure!(
            journal.warmup.evidence["initialCurrent"] == serde_json::to_value(&initial_current)?
                && journal.warmup.evidence["initialNext"] == serde_json::to_value(&initial_next)?,
            "original pre-rotation authority evidence changed; HOLD"
        );
    }
    let prepared: source::PreparedRotation = if journal.warmup.evidence["rotationIntent"].is_null()
    {
        timeout_at(
            readiness_deadline.into(),
            ctx.source
                .prepare_rotation(rotation_authority, rotation_suri),
        )
        .await
        .context("rotation preparation exceeded original readiness deadline")??
    } else {
        serde_json::from_value(journal.warmup.evidence["rotationIntent"].clone())?
    };
    ensure!(
        prepared.authority.eq_ignore_ascii_case(rotation_authority)
            && !initial_current.keys.contains(&prepared.beefy_key)
            && !initial_next.keys.contains(&prepared.beefy_key),
        "requested BEEFY key already exists or original authority changed; HOLD"
    );
    let should_send =
        prepare_rotation_handoff(journal, path, serde_json::to_value(&prepared)?, now_ms()?)?;
    if should_send {
        match timeout_at(
            readiness_deadline.into(),
            ctx.source.submit_prepared_rotation(&prepared),
        )
        .await
        {
            Ok(Ok(rotation)) => {
                journal.warmup.evidence["rotation"] = serde_json::to_value(rotation)?;
                save_journal(path, journal)?;
            }
            _ => {
                journal.warmup.evidence["rotationSubmissionState"] =
                    json!("held: original handoff requires read-only reconciliation");
                save_journal(path, journal)?;
            }
        }
    }
    let rotation = timeout_at(readiness_deadline.into(), async {
        loop {
            if let Some(rotation) = ctx.source.reconcile_rotation(&prepared).await? {
                let observed = serde_json::to_value(&rotation)?;
                if !journal.warmup.evidence["rotation"].is_null() {
                    ensure!(journal.warmup.evidence["rotation"] == observed, "original rotation inclusion changed; HOLD");
                } else {
                    journal.warmup.evidence["rotation"] = observed;
                    save_journal(path, journal)?;
                }
                let witness_finalized = ctx.witness.api.block_hash_to_number(ctx.witness.api.latest_finalized_block().await?).await?;
                if witness_finalized >= rotation.block {
                    ensure!(ctx.witness.api.block_number_to_hash(rotation.block).await?.0 == rotation.block_hash,
                        "original rotation finalized inclusion differs across canonical nodes; HOLD");
                    journal.warmup.evidence["rotationCanonicalObservedAtMs"] = json!(now_ms()?);
                    save_journal(path, journal)?;
                    return Ok::<_, anyhow::Error>(rotation);
                }
            }
            sleep_until(readiness_deadline.min(Instant::now() + Duration::from_secs(3)).into()).await;
        }
    }).await.context("original rotation remained held through its unchanged readiness deadline")??;
    lanes_ready(
        ctx,
        journal,
        path,
        "before_sampling",
        0,
        readiness_deadline,
        &BTreeMap::new(),
    )
    .await?;

    let baseline = SnapshotSet::from_json(&journal.windows["preflight-priority"]["baseline"])?;
    ctx.snapshot_at(Some(&baseline)).await?;
    let before_sampling = ctx.snapshot().await?;
    verify_roundtrip_delta(&baseline, &before_sampling, &ctx.tokens)?;
    journal.warmup.evidence["baseline"] = baseline.to_json();
    journal.warmup.evidence["beforeSampling"] = before_sampling.to_json();

    let started = now_ms()?;
    let clock = ClockAnchor::new(started)?;
    let warmup_deadline = clock.monotonic + Duration::from_secs(ctx.schedule.warmup_secs);
    let warmup_deadline_ms = started
        .checked_add(ctx.schedule.warmup_secs * 1_000)
        .context("warmup deadline overflow")?;
    journal.warmup.started_at_ms = Some(started);
    journal.warmup.evidence["terminalAuditDeadlineAtMs"] = json!(warmup_deadline_ms
        .checked_add(60_000)
        .context("terminal audit deadline overflow")?);
    journal.warmup.evidence["startedAtMs"] = json!(started);
    journal.warmup.evidence["deadlineAtMs"] = json!(warmup_deadline_ms);
    save_journal(path, journal)?;
    let mut next_seen = None;
    let mut current_seen = None;
    let mut restart_before = None;
    let mut restarted = false;
    let rotated_key = serde_json::to_value(&rotation.beefy_key)?;
    for minute in 0..ctx.schedule.warmup_secs / 60 {
        sleep_until((clock.monotonic + Duration::from_secs(minute * 60)).into()).await;
        clock.check()?;
        let sample_deadline = warmup_sample_deadline(clock.monotonic, minute, Instant::now())?;
        timeout_at(sample_deadline.into(), async {
            let workers = loop {
                let mut observation = ctx.worker_readiness(&BTreeMap::new()).await?;
                observation["observedAtMs"] = json!(now_ms()?);
                journal.warmup.evidence["latestWorkerObservation"] = observation.clone();
                save_journal(path, journal)?;
                if observation["ready"] == true { break observation; }
                sleep_until(sample_deadline.min(Instant::now() + Duration::from_secs(3)).into()).await;
            };
            let (service_state, checkpoint, mined) = coherent_follower_observation(ctx, sample_deadline).await?;
            let source_hash = ctx.source.api.latest_finalized_block().await?;
            let source_height = ctx.source.api.block_hash_to_number(source_hash).await?;
            let witness_height = ctx.witness.api.block_hash_to_number(
                ctx.witness.api.latest_finalized_block().await?).await?;
            ensure!(witness_height >= source_height,
                "witness finalized head is behind during warmup");
            ensure!(ctx.witness.api.block_number_to_hash(source_height).await? == source_hash,
                "warmup source/witness canonical hashes differ");
            let (current, next) = ctx.source.checkpoint_at_hash(source_hash).await?;
            let (witness_current, witness_next) = ctx.witness.checkpoint_at_hash(source_hash).await?;
            ensure!(current == witness_current && next == witness_next,
                "warmup current/next sets differ across independent Gear nodes");
            ensure!(current.keys.len() == 2 && next.keys.len() == 2,
                "warmup lost the two-validator topology");
            if next.keys.contains(&rotation.beefy_key) && next_seen.is_none() {
                next_seen = Some(json!({"height":source_height,"hash":format!("{source_hash:#x}"),"setId":next.id}));
            }
            if current.keys.contains(&rotation.beefy_key)
                && current.id > initial_current.id && current_seen.is_none() {
                current_seen = Some(json!({"height":source_height,"hash":format!("{source_hash:#x}"),"setId":current.id}));
            }
            if current_seen.is_some() && journal.warmup.evidence["handoverCommitment"].is_null() {
                for entry in service_state["commitments"].as_array()
                    .context("follower commitments are missing")?.iter().rev() {
                    let keys = entry["current"]["keys"].as_array()
                        .context("accepted commitment has no authority keys")?;
                    if number(&entry["current"]["id"])? <= initial_current.id || !keys.contains(&rotated_key) {
                        continue;
                    }
                    let block = u32::try_from(number(&entry["block"])?)?;
                    let raw = hex::decode(string(&entry["rawScale"], "accepted rawScale")?
                        .strip_prefix("0x").context("rawScale must have a hex prefix")?)?;
                    let signed = ctx.source.recapture(block, raw.clone()).await?;
                    let witnessed = ctx.witness.recapture(block, raw).await?;
                    ensure!(signed.current.id > initial_current.id
                        && signed.current.keys.contains(&rotation.beefy_key)
                        && signed.validated.signed_indices.len() == 2
                        && signed.block_hash == bytes32(string(&entry["blockHash"], "accepted blockHash")?)?
                        && witnessed.block_hash == signed.block_hash
                        && witnessed.validated.commitment_hash == signed.validated.commitment_hash
                        && witnessed.current == signed.current && witnessed.next == signed.next
                        && witnessed.validated.signed_indices.len() == 2,
                        "new session commitment is not independently witnessed with both validator signatures");
                    journal.warmup.evidence["handoverCommitment"] = json!({
                        "block":signed.block,"blockHash":format!("0x{}",hex::encode(signed.block_hash)),
                        "commitmentHash":format!("0x{}",hex::encode(signed.validated.commitment_hash)),
                        "signedValidators":2,"current":signed.current,"next":signed.next,
                        "observedAtMs":now_ms()?,
                    });
                    break;
                }
            }
            if ctx.schedule.handovers == 2 {
                if journal.warmup.evidence["handoverCommitments"].is_null() {
                    journal.warmup.evidence["handoverCommitments"] = json!([]);
                }
                for entry in service_state["commitments"].as_array().context("original commitment journal is missing")? {
                    let saved = journal.warmup.evidence["handoverCommitments"].as_array().context("handover evidence malformed")?;
                    if saved.len() == ctx.schedule.handovers { break; }
                    let previous = saved.last().cloned().unwrap_or_else(|| json!({"current":initial_current,"next":initial_next}));
                    if entry["finalized"] != true || number(&entry["current"]["id"])? != number(&previous["current"]["id"])? + 1 { continue; }
                    let evidence = authenticate_normal_handover(ctx, entry, &previous).await?;
                    journal.warmup.evidence["handoverCommitments"].as_array_mut().context("handover evidence malformed")?.push(evidence);
                    save_journal(path, journal)?;
                }
            }
            ensure!(u64::from(source_height) >= checkpoint.block,
                "accepted Hoodi anchor is ahead of finalized Gear");
            let lag = u64::from(source_height) - checkpoint.block;
            let finalized_follower_block = follower_checkpoint(&service_state)?;
            let mined_inclusion = mined.map(|head| json!({
                "client":format!("{:#x}",ctx.ethereum.client_address),
                "sourceBlock":head.source_block,"sourceHash":format!("0x{}",hex::encode(head.source_hash)),
                "txHash":format!("{:#x}",head.tx_hash),"block":head.destination_block,
                "blockHash":format!("{:#x}",head.destination_hash),
            }));
            if restart_before.is_none() && minute >= ctx.schedule.warmup_secs / 120 {
                let sequence = number(&service_state["startupSequence"])?;
                let minimum = u64::from(source_height).max(checkpoint.block.checked_add(1)
                    .context("follower checkpoint cannot advance")?);
                let at = now_ms()?;
                journal.warmup.evidence["restartGate"] = json!({
                    "beforeStartupSequence":sequence,"beforeAcceptedBlock":checkpoint.block,
                    "minimumAcceptedBlock":minimum,"sourceHeight":source_height,
                    "sourceHash":format!("{source_hash:#x}"),"atMs":at,
                    "deadlineAtMs":at.saturating_add(600_000).min(warmup_deadline_ms),
                });
                restart_before = Some(service_state.clone());
                save_journal(path, journal)?;
            }
            if let Some(before) = restart_before.as_ref().filter(|_| !restarted) {
                ensure_actor_history_preserved(before, &service_state)?;
                let gate = &journal.warmup.evidence["restartGate"];
                ensure!(now_ms()? < number(&gate["deadlineAtMs"])?,
                    "supervised token follower restart was not observed before its original deadline");
                let sequence = number(&service_state["startupSequence"])?;
                ensure!(sequence >= number(&gate["beforeStartupSequence"])?,
                    "follower startup sequence regressed");
                if sequence > number(&gate["beforeStartupSequence"])?
                    && service_state["follower"]["status"] == "healthy"
                    && checkpoint.block >= number(&gate["minimumAcceptedBlock"])?
                    && mined.is_some_and(|head| head.source_block == checkpoint.block)
                    && lag < ctx.schedule.authority_lag_limit
                    && number(&service_state["follower"]["freshnessDeadlineMs"])? >= now_ms()? {
                    journal.warmup.evidence["followerRestart"] = json!({
                        "beforeStartupSequence":gate["beforeStartupSequence"],"afterStartupSequence":sequence,
                        "beforeAcceptedBlock":gate["beforeAcceptedBlock"],"afterAcceptedBlock":checkpoint.block,
                        "sourceHeightAtGate":gate["sourceHeight"],"sourceHeightAfterRestart":source_height,
                        "gateAtMs":gate["atMs"],"observedAtMs":now_ms()?,
                        "serviceState":ctx.follower_dir.join("state.json"),
                        "afterMinedInclusion":mined_inclusion.clone(),"caughtUp":true,
                    });
                    restarted = true;
                }
            }
            let catching_up = restart_before.is_some() && !restarted;
            if !catching_up {
                ensure!(service_state["follower"]["status"] == "healthy",
                    "token follower is not healthy outside supervised restart/catch-up");
                ensure!(lag < ctx.schedule.authority_lag_limit,
                    "accepted BEEFY anchor lag reached the sealed epoch bound");
                ensure!(mined.is_some_and(|head| head.source_block == checkpoint.block),
                    "follower journal has no verified canonical mined anchor for the active client");
                ensure!(number(&service_state["follower"]["freshnessDeadlineMs"])? >= now_ms()?,
                    "follower checkpoint freshness expired");
            }
            journal.warmup.evidence["samples"].as_array_mut().context("warmup samples are missing")?.push(json!({
                "minute":minute,"scheduledAtMs":started + minute * 60_000,"atMs":now_ms()?,
                "sourceHeight":source_height,"sourceHash":format!("{source_hash:#x}"),
                "acceptedBlock":checkpoint.block,"acceptedLag":lag,
                "followerBlock":finalized_follower_block,"followerFinalizedBlock":finalized_follower_block,
                "followerMinedBlock":mined.map(|head| head.source_block),
                "minedInclusion":mined_inclusion,"lastFinalizedUpdate":service_state["follower"]["lastFinalizedUpdate"],
                "actorStatus":service_state["follower"]["status"],
                "phase":if catching_up { "restart_catchup" } else if restarted { "post_catchup" } else { "before_restart" },
                "startupSequence":service_state["startupSequence"],"currentSetId":current.id,"nextSetId":next.id,
                "workers":workers,
            }));
            journal.warmup.evidence["nextSetObserved"] = json!(next_seen);
            journal.warmup.evidence["currentSetObserved"] = json!(current_seen);
            save_journal(path, journal)
        }).await.map_err(|_| anyhow!("warmup minute {minute} expired; missed samples cannot be backfilled"))??;
    }
    sleep_until(warmup_deadline.into()).await;
    clock.check()?;
    ensure!(
        now_ms()?.saturating_sub(started) >= ctx.schedule.warmup_secs * 1_000,
        "warmup ended before its sealed duration"
    );
    ensure!(
        next_seen.is_some()
            && current_seen.is_some()
            && !journal.warmup.evidence["handoverCommitment"].is_null(),
        "warmup did not witness next/current transitions and a signed rotated-set commitment"
    );
    if ctx.schedule.handovers == 2 {
        ensure!(journal.warmup.evidence["handoverCommitments"].as_array().is_some_and(|records| records.len() == 2),
            "named runtime warmup lacks two real sequential authenticated finalized authority handovers; HOLD");
    }
    ensure!(
        restarted,
        "warmup did not restart and catch up the follower"
    );
    let terminal_deadline = warmup_deadline + Duration::from_secs(60);
    timeout_at(terminal_deadline.into(), async {
        lanes_ready(
            ctx,
            journal,
            path,
            "warmup_terminal",
            0,
            terminal_deadline,
            &BTreeMap::new(),
        )
        .await?;
        let after = ctx.snapshot().await?;
        verify_roundtrip_delta(&baseline, &after, &ctx.tokens)?;
        journal.warmup.evidence["terminalSnapshot"] = after.to_json();
        save_journal(path, journal)
    })
    .await
    .context("warmup terminal worker/accounting observation exceeded its fixed deadline")??;
    let samples = journal.warmup.evidence["samples"]
        .as_array()
        .context("warmup samples are missing")?;
    validate_warmup_samples(samples, started, ctx.schedule.warmup_secs / 60)?;
    let first_post_catchup = samples
        .iter()
        .position(|s| s["phase"] == "post_catchup")
        .context("post-catch-up capacity samples are missing")?;
    let post_catchup = &samples[first_post_catchup..];
    ensure!(
        post_catchup.len() >= 3,
        "insufficient post-catch-up capacity samples"
    );
    let first_lag = number(&post_catchup[0]["acceptedLag"])?;
    let last_lag = number(&post_catchup[post_catchup.len() - 1]["acceptedLag"])?;
    let early_avg = mean_lag(&post_catchup[..3])?;
    let late_avg = mean_lag(&post_catchup[post_catchup.len() - 3..])?;
    ensure!(
        last_lag <= first_lag.saturating_add(2) && late_avg <= early_avg + 1.0,
        "accepted-anchor lag trended upward after restart catch-up"
    );
    journal.warmup.evidence["summary"] = json!({
        "durationMs":now_ms()?.saturating_sub(started),"samples":samples.len(),
        "postCatchupSamples":post_catchup.len(),"firstLag":first_lag,"lastLag":last_lag,
        "earlyAverageLag":early_avg,"lateAverageLag":late_avg,"twoValidators":true,
        "nextAndCurrentTransition":true,"followerRestartAndCatchUp":true,
        "workersCheckedEveryMinute":true,"terminalAccountingUnchanged":true,
    });
    Ok(())
}

async fn prepare_campaign_allowances(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
) -> Result<()> {
    ensure!(
        journal.preflight.status == "passed" && journal.warmup.status == "passed",
        "campaign allowances can only be prepared after both readiness gates"
    );
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    let mut evm_actions = Vec::new();
    for token in &ctx.tokens {
        let desired_evm = EthU256::from(token.raw_amount(CAMPAIGN_ALLOWANCE)?);
        let contract = CampaignToken::new(token.address, ctx.ethereum.api.raw_provider().clone());
        let finalized = ctx.ethereum.api.finalized_block_number().await?;
        let block = ctx
            .ethereum
            .api
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(finalized))
            .await?
            .context("missing finalized Hoodi block")?;
        let allowance = contract
            .allowance(ctx.campaign_address, ctx.ethereum.receiver_address().into())
            .block(BlockId::hash_canonical(block.header.hash))
            .call()
            .await?;
        if allowance == desired_evm {
            continue;
        }
        let action_id = format!("start-allowance-evm-{}", token.symbol);
        let manager_address: Address = ctx.ethereum.receiver_address().into();
        let recovery_token = token.address;
        let recovery_owner = ctx.campaign_address;
        let recovery_spender = manager_address;
        let recovery_amount = desired_evm;
        let recovery_context: &Context = ctx;
        let recovery_provider = ctx.ethereum.api.raw_provider().clone();
        let hash = evm_action(
            ctx, journal, path, &action_id,
            json!({"kind":"erc20-approve","token":format!("{:#x}",token.address),"spender":format!("{manager_address:#x}"),"amountRaw":desired_evm.to_string()}),
            move |nonce| async move {
                Ok(CampaignToken::new(recovery_token, recovery_provider)
                    .approve(recovery_spender, recovery_amount).nonce(nonce).into_transaction_request())
            },
            move |from, nonce| find_approval_tx(recovery_context, recovery_token, recovery_owner, recovery_spender, recovery_amount, from, nonce),
        ).await?;
        evm_actions.push((action_id, hash));
    }
    let receipts = wait_finalized_actions(ctx, journal, path, &evm_actions, deadline).await?;
    ensure_beacon_through_receipts(ctx, &receipts, deadline).await?;

    let finalized_gear = ctx.source.api.latest_finalized_block().await?;
    let mut vft = Vft::new(ctx.remoting.clone());
    let mut pending = Vec::new();
    for token in &ctx.tokens {
        let allowance = vft
            .allowance(ctx.campaign_actor, ctx.manager_id)
            .at_block(sails_block_hash(finalized_gear))
            .recv(token.peer)
            .await?;
        let desired = GearU256::from(token.raw_amount(CAMPAIGN_ALLOWANCE)?);
        if allowance == desired {
            continue;
        }
        let action_id = format!("start-allowance-gear-{}", token.symbol);
        let intent = json!({"kind":"vft-approve","token":token.component,"amountRaw":token.raw_amount(CAMPAIGN_ALLOWANCE)?});
        if ctx.schedule.handovers == 2 {
            signed_normal_message(
                ctx,
                (&ctx.gear_api, ctx.campaign_actor),
                (journal, path),
                (&action_id, intent),
                (
                    token.peer,
                    vft_client::vft::io::Approve::encode_call(ctx.manager_id, desired),
                    0,
                ),
                deadline,
            )
            .await?;
            let reply = normal_message_reply(
                ctx,
                ctx.campaign_actor,
                journal,
                path,
                &action_id,
                token.peer,
                deadline,
            )
            .await?;
            ensure!(
                decode_campaign_reply::<bool>(&reply, vft_client::vft::io::Approve::ROUTE)?,
                "VFT approval rejected; HOLD"
            );
            pending.push((token.symbol, true));
            continue;
        }
        if !prepare_gear_action(journal, path, &action_id, intent)? {
            continue;
        }
        let approved = vft
            .approve(ctx.manager_id, desired)
            .send_recv(token.peer)
            .await?;
        pending.push((token.symbol, approved));
    }
    let approvals: Vec<bool> = pending.into_iter().map(|(_, approved)| approved).collect();
    ensure!(
        approvals.into_iter().all(|approved| approved),
        "Gear VFT allowance approval returned false"
    );
    let finalized_gear = ctx.source.api.latest_finalized_block().await?;
    for token in &ctx.tokens {
        let allowance = vft
            .allowance(ctx.campaign_actor, ctx.manager_id)
            .at_block(sails_block_hash(finalized_gear))
            .recv(token.peer)
            .await?;
        ensure!(
            allowance == GearU256::from(token.raw_amount(CAMPAIGN_ALLOWANCE)?),
            "{} VFT campaign allowance was not finalized",
            token.symbol
        );
        let key = format!("start-allowance-gear-{}", token.symbol);
        if let Some(action) = journal.actions.get_mut(&key) {
            action.status = "finalized".into();
            action.evidence["finalizedGearHash"] = json!(format!("{finalized_gear:#x}"));
            action.evidence["allowanceRaw"] = json!(allowance.to_string());
        }
    }
    save_journal(path, journal)?;
    let pending_nonce = ctx
        .ethereum
        .api
        .raw_provider()
        .get_transaction_count(ctx.campaign_address)
        .pending()
        .await?;
    if let Some(next_nonce) = journal.next_campaign_nonce {
        ensure!(
            pending_nonce <= next_nonce,
            "campaign account has an unjournaled pending EVM nonce"
        );
    } else {
        journal.next_campaign_nonce = Some(pending_nonce);
        save_journal(path, journal)?;
    }
    Ok(())
}

async fn verify_campaign_start(ctx: &Context, journal: &Journal) -> Result<()> {
    ensure!(
        journal.preflight.status == "passed" && journal.warmup.status == "passed",
        "readiness gates are incomplete"
    );
    ensure!(journal.t0_ms.is_none(), "T0 already exists");
    let snapshot = ctx.snapshot().await?;
    for token in &ctx.tokens {
        let state = snapshot
            .assets
            .get(token.symbol)
            .context("missing start balance")?;
        ensure!(
            state.evm_escrow.is_zero(),
            "{} manager escrow is not empty before T0",
            token.symbol
        );
        if token.gear_origin {
            ensure!(state.evm_user.is_zero() && state.evm_supply.is_zero() && state.gear_user>=GearU256::from(token.raw_amount(CAMPAIGN_ALLOWANCE)?) && state.gear_supply>=state.gear_user && state.gear_escrow.is_some_and(|escrow|escrow.is_zero()),"campaign source inventory/custody cannot cover24original Gear-origin windows; HOLD");
        } else {
            ensure!(
                state.evm_user >= EthU256::from(CAMPAIGN_ALLOWANCE)
                    && state.gear_user.is_zero()
                    && state.gear_supply.is_zero()
                    && state.evm_supply != EthU256::ZERO,
                "campaign Ethereum-origin baseline incomplete; HOLD"
            );
        }
        let evm_allowance =
            CampaignToken::new(token.address, ctx.ethereum.api.raw_provider().clone())
                .allowance(ctx.campaign_address, ctx.ethereum.receiver_address().into())
                .block(BlockId::hash_canonical(snapshot.evm_hash))
                .call()
                .await?;
        ensure!(
            evm_allowance == EthU256::from(token.raw_amount(CAMPAIGN_ALLOWANCE)?),
            "{} EVM campaign allowance is not 24 raw units",
            token.symbol
        );
        let gear_allowance = Vft::new(ctx.remoting.clone())
            .allowance(ctx.campaign_actor, ctx.manager_id)
            .at_block(sails_block_hash(snapshot.gear_hash))
            .recv(token.peer)
            .await?;
        ensure!(
            gear_allowance == GearU256::from(token.raw_amount(CAMPAIGN_ALLOWANCE)?),
            "{} Gear campaign allowance is not 24 raw units",
            token.symbol
        );
    }
    Ok(())
}

async fn run_campaign(ctx: &mut Context, journal: &mut Journal, path: &Path) -> Result<()> {
    let t0 = journal.t0_ms.context("campaign T0 is missing")?;
    let now = now_ms()?;
    journal.validate_wall_time(now)?;
    let end_ms = t0
        .checked_add(u64::from(QUALIFICATION_HOURS) * HOUR_MS)
        .context("campaign end timestamp overflow")?;
    let report_path = path
        .parent()
        .context("journal has no parent")?
        .join("qualification-report.json");
    if journal.terminal_report.is_some() || report_path.exists() {
        return write_terminal_report(journal, path);
    }
    let terminal_deadline = Instant::now() + Duration::from_millis(end_ms.saturating_sub(now));
    let clock = ClockAnchor::new(now)?;
    if journal.status == "failed" {
        observe_to_terminal(ctx, journal, path, terminal_deadline, clock).await?;
        return write_terminal_report(journal, path);
    }

    let mut failure = None;
    for hour in 0..QUALIFICATION_HOURS {
        if window_complete(journal, hour) {
            continue;
        }
        let start = t0 + u64::from(hour) * HOUR_MS;
        let deadline_ms = start
            .checked_add(HOUR_MS)
            .context("hour deadline overflow")?;
        let key = window_key(hour);
        let fee_mode = if hour.is_multiple_of(2) {
            FeeMode::Normal
        } else {
            FeeMode::Priority
        };
        let result: Result<()> = async {
            let current = now_ms()?;
            let prior = journal.windows.get(&key).cloned();
            let boundary_intent = prior.as_ref().is_some_and(|record| window_has_boundary_intent(record, start));
            if prior.as_ref().is_some_and(|record| record["status"] == "scheduled") && !boundary_intent {
                bail!("hour {hour} has an invalid persisted boundary intent");
            }
            if current > start && !boundary_intent {
                bail!("hour {hour} scheduled start was missed without a persisted pre-boundary intent");
            }
            if current <= start && prior.is_none() {
                journal.windows.insert(key.clone(), scheduled_window(hour, start, deadline_ms, current));
                save_journal(path, journal)?;
            }
            if current < start {
                wait_observing(ctx, journal, path, start, clock).await?;
            }
            clock.check()?;
            let started_at = now_ms()?;
            ensure!(can_start_window(prior.as_ref(), started_at, start), "hour {hour} missed its absolute scheduled start");
            let deadline = instant_deadline(deadline_ms)?;
            let record = journal.windows.entry(key.clone()).or_insert_with(|| json!({
                "status":"running","hour":hour,"scheduledStartMs":start,"deadlineMs":deadline_ms,
                "feeMode":fee_mode,"amountRaw":"1","assets":{},"stages":{}
            }));
            if record["status"] == "scheduled" {
                record["status"] = json!("running");
                record["startedAtMs"] = json!(started_at);
            } else if record["startedAtMs"].is_null() {
                record["startedAtMs"] = json!(started_at);
            }
            save_journal(path, journal)?;
            run_roundtrip(ctx, journal, path, &key, fee_mode, WINDOW_AMOUNT, false, deadline).await?;
            let completed_at = now_ms()?;
            ensure!(completed_at < deadline_ms, "hour {hour} completed at or after its absolute deadline");
            let record = journal.windows.get_mut(&key).expect("window inserted");
            record["status"] = json!("passed");
            record["completedAtMs"] = json!(completed_at);
            save_journal(path, journal)
        }.await;
        if let Err(error) = result {
            failure = Some(format!("hour {hour}: {error:#}"));
            break;
        }
    }

    if failure.is_none() {
        if let Err(error) = wait_observing(ctx, journal, path, end_ms, clock).await {
            failure = Some(format!("qualification observation failed: {error:#}"));
        }
    }
    if let Some(error) = failure {
        fail_campaign(journal, path, &error)?;
        observe_to_terminal(ctx, journal, path, terminal_deadline, clock).await?;
        return write_terminal_report(journal, path);
    }

    let all_complete = (0..QUALIFICATION_HOURS).all(|hour| window_complete(journal, hour));
    if !all_complete {
        fail_campaign(
            journal,
            path,
            "not all 24 hourly windows contain four complete roundtrips",
        )?;
    } else {
        journal.status = "passed".into();
        save_journal(path, journal)?;
    }
    write_terminal_report(journal, path)
}

fn decode_campaign_reply<T: Decode + 'static>(payload: &[u8], route: &[u8]) -> Result<T> {
    // Sails unit actions acknowledge with an empty payload, without a route envelope.
    let mut bytes = if std::any::TypeId::of::<T>() == std::any::TypeId::of::<()>() {
        payload
    } else {
        payload
            .strip_prefix(route)
            .context("original consumer reply route differs; HOLD")?
    };
    let value = T::decode(&mut bytes)?;
    ensure!(
        bytes.is_empty(),
        "original consumer reply has trailing bytes; HOLD"
    );
    Ok(value)
}

async fn normal_message_reply(
    ctx: &Context,
    actor: ActorId,
    journal: &mut Journal,
    path: &Path,
    id: &str,
    target: ActorId,
    deadline: Instant,
) -> Result<Vec<u8>> {
    let action = &journal.actions[id];
    let original = &action.evidence["sourceFinality"];
    let message_id =
        GearHash::from_str(string(&original["messageId"], "original campaign message")?)?;
    let enqueue = u32::try_from(number(&original["enqueueHeight"])?)?;
    let saved = action.evidence.get("originalReply").cloned();
    let mut next = saved
        .as_ref()
        .map(|reply| number(&reply["finalizedHeight"]))
        .transpose()?
        .map(u32::try_from)
        .transpose()?
        .unwrap_or(enqueue);
    loop {
        ensure!(
            Instant::now() < deadline,
            "original campaign reply deadline expired; HOLD"
        );
        let head = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?
            .min(
                ctx.witness
                    .api
                    .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
                    .await?,
            );
        for height in next..=head {
            let hash = ctx.source.api.block_number_to_hash(height).await?;
            ensure!(
                ctx.witness.api.block_number_to_hash(height).await? == hash,
                "original campaign reply differs across witnesses"
            );
            for event in ctx
                .source
                .api
                .get_block_at(hash)
                .await?
                .events()
                .await?
                .iter()
            {
                let RuntimeEvent::Gear(GearEvent::UserMessageSent { message, .. }) =
                    event?.as_gear()?
                else {
                    continue;
                };
                let Some(details) = message.details() else {
                    continue;
                };
                if GearHash::from_slice(details.to_message_id().as_ref()) != message_id {
                    continue;
                }
                ensure!(
                    message.source() == target
                        && message.destination() == actor
                        && details.to_reply_code().is_success(),
                    "original campaign reply failed or changed sender/recipient; HOLD"
                );
                let reply = json!({"finalizedHeight":height,"finalizedHash":format!("{hash:#x}"),"originalMessageId":format!("{message_id:#x}"),"replyId":format!("0x{}",hex::encode(message.id().into_bytes())),"payload":format!("0x{}",hex::encode(message.payload_bytes()))});
                ensure!(
                    saved.as_ref().is_none_or(|original| *original == reply),
                    "original campaign consumer reply changed; HOLD"
                );
                let payload = message.payload_bytes().to_vec();
                set_action_evidence(journal, id, "originalReply", reply);
                save_journal(path, journal)?;
                return Ok(payload);
            }
            ensure!(
                saved.is_none(),
                "original campaign consumer reply disappeared; HOLD"
            );
        }
        next = head
            .checked_add(1)
            .context("campaign reply block overflow")?;
        sleep_until(deadline.min(Instant::now() + Duration::from_secs(3)).into()).await;
    }
}

async fn signed_normal_gear_call<P: gsdk::ext::subxt::tx::Payload>(
    ctx: &Context,
    (api, actor): (&gclient::GearApi, ActorId),
    (journal, path): (&mut Journal, &Path),
    (id, intent): (&str, Value),
    call: P,
    target: Option<ActorId>,
    deadline: Instant,
) -> Result<Value> {
    let signer: gsdk::signer::Signer = api.clone().into();
    ensure!(
        signer.api().genesis_hash().0 == ctx.source.source_genesis
            && <[u8; 32]>::from(signer.account_id().clone()) == actor.into_bytes(),
        "campaign signed source/signer changed; HOLD"
    );
    let call_data = signer.api().tx().call_data(&call)?;
    let original = journal.actions.get(id);
    if let Some(action) = original {
        ensure!(
            action.intent == intent && action.evidence["normalOriginalBytes"] == true,
            "unsigned legacy or changed campaign action must remain HOLD"
        );
    } else {
        journal.actions.insert(id.into(), Action {status:"prepared".into(), intent, from_block:None, nonce:None, tx_hash:None,
            evidence:json!({"normalOriginalBytes":true,"callData":format!("0x{}",hex::encode(&call_data))})});
        save_journal(path, journal)?;
    }
    let signed = if journal.actions[id].evidence["rawExtrinsic"].is_null() {
        ensure!(
            journal.actions[id].status == "prepared",
            "ambiguous unsigned source intent cannot be signed again; HOLD"
        );
        let pin = ctx.source.api.latest_finalized_block().await?;
        let nonce = signer.api().tx().account_nonce(signer.account_id()).await?;
        let signed = signer
            .api()
            .tx()
            .create_signed(
                &call,
                signer.signer(),
                gsdk::ext::subxt::config::polkadot::PolkadotExtrinsicParamsBuilder::<
                    gsdk::GearConfig,
                >::new()
                .nonce(nonce)
                .build(),
            )
            .await?;
        let action = journal
            .actions
            .get_mut(id)
            .context("campaign source intent disappeared")?;
        action.from_block = Some(u64::from(ctx.source.api.block_hash_to_number(pin).await?));
        action.nonce = Some(nonce);
        action.evidence["fromHash"] = json!(format!("{pin:#x}"));
        action.evidence["rawExtrinsic"] = json!(format!("0x{}", hex::encode(signed.encoded())));
        action.evidence["extrinsicHash"] = json!(format!("{:#x}", signed.hash()));
        action.evidence["sourceGenesis"] =
            json!(format!("0x{}", hex::encode(ctx.source.source_genesis)));
        record_action_milestone(journal, id, "submissionHandoffAtMs", now_ms()?)?;
        save_journal(path, journal)?;
        signed.encoded().to_vec()
    } else {
        hex::decode(
            string(
                &journal.actions[id].evidence["rawExtrinsic"],
                "original source bytes",
            )?
            .trim_start_matches("0x"),
        )?
    };
    let action = &journal.actions[id];
    let hash = bytes32(string(
        &action.evidence["extrinsicHash"],
        "original source hash",
    )?)?;
    let start = u32::try_from(action.from_block.context("source start pin missing")?)?;
    ensure!(
        sp_core::blake2_256(&signed) == hash
            && action.evidence["callData"] == format!("0x{}", hex::encode(&call_data))
            && ctx.source.api.block_number_to_hash(start).await?.0
                == bytes32(string(&action.evidence["fromHash"], "original source pin")?)?,
        "original source bytes/call/pin changed; HOLD"
    );
    let decoded =
        gsdk::ext::subxt::ext::subxt_core::blocks::Extrinsics::<gsdk::GearConfig>::decode_from(
            vec![signed.clone()],
            signer.api().metadata(),
        )?;
    let extrinsic = decoded
        .iter()
        .next()
        .context("original source extrinsic missing")?;
    let mut address = [0; 33];
    address[1..].copy_from_slice(&actor.into_bytes());
    ensure!(
        extrinsic.address_bytes() == Some(address.as_slice())
            && extrinsic.call_bytes() == call_data
            && extrinsic
                .transaction_extensions()
                .and_then(|extensions| extensions.nonce())
                == action.nonce,
        "original source signer/nonce/call differs; HOLD"
    );
    let saved_inclusion = action.evidence.get("sourceFinality").cloned();
    let mut broadcast = false;
    loop {
        ensure!(
            Instant::now() < deadline,
            "original campaign deadline expired retaining signed source bytes; HOLD"
        );
        let head = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?
            .min(
                ctx.witness
                    .api
                    .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
                    .await?,
            );
        let from = saved_inclusion
            .as_ref()
            .map(|saved| number(&saved["enqueueHeight"]))
            .transpose()?
            .map(u32::try_from)
            .transpose()?
            .unwrap_or(start);
        for height in from..=head {
            let pin = ctx.source.api.block_number_to_hash(height).await?;
            ensure!(
                ctx.witness.api.block_number_to_hash(height).await? == pin,
                "campaign original source inclusion differs across witnesses; HOLD"
            );
            let block = ctx.source.api.get_block_at(pin).await?;
            for included in block.extrinsics().await?.iter() {
                if included.hash().0 != hash {
                    continue;
                }
                ensure!(
                    included.bytes() == signed,
                    "original campaign source inclusion changed bytes; HOLD"
                );
                let mut success = false;
                let mut message = None;
                let mut value_read = false;
                for event in included.events().await?.iter() {
                    let event = event?;
                    ensure!(
                        event.pallet_name() != "System"
                            || event.variant_name() != "ExtrinsicFailed",
                        "original campaign source dispatch failed; HOLD"
                    );
                    success |= event.pallet_name() == "System"
                        && event.variant_name() == "ExtrinsicSuccess";
                    value_read |=
                        event.pallet_name() == "Gear" && event.variant_name() == "UserMessageRead";
                    if let RuntimeEvent::Gear(GearEvent::MessageQueued {
                        id,
                        source,
                        destination,
                        ..
                    }) = event.as_gear()?
                    {
                        if target == Some(destination) {
                            ensure!(
                                source.0 == actor.into_bytes() && message.is_none(),
                                "source enqueue identity changed"
                            );
                            message = Some(format!("0x{}", hex::encode(id.as_ref())));
                        }
                    }
                }
                ensure!(
                    success && (target.is_none() || message.is_some()),
                    "original campaign source enqueue lacks successful dispatch; HOLD"
                );
                let evidence = json!({"messageId":message,"enqueueHeight":height,"enqueueBlockHash":format!("{pin:#x}"),"extrinsicHash":format!("0x{}",hex::encode(hash)),"valueRead":value_read});
                ensure!(
                    saved_inclusion
                        .as_ref()
                        .is_none_or(|saved| *saved == evidence),
                    "original source finality changed; HOLD"
                );
                set_action_evidence(journal, id, "sourceFinality", evidence.clone());
                journal
                    .actions
                    .get_mut(id)
                    .context("source intent missing")?
                    .status = "finalized".into();
                save_journal(path, journal)?;
                return Ok(evidence);
            }
        }
        ensure!(
            saved_inclusion.is_none(),
            "original signed source inclusion disappeared; HOLD"
        );
        if !broadcast {
            let nonce = signer.api().tx().account_nonce(signer.account_id()).await?;
            ensure!(
                Some(nonce) == journal.actions[id].nonce,
                "original source nonce is no longer available; HOLD without replacement"
            );
            let transaction =
                gsdk::ext::subxt::tx::SubmittableTransaction::<gsdk::GearConfig, _>::from_bytes(
                    (**signer.api()).clone(),
                    signed.clone(),
                );
            ensure!(
                transaction.hash().0 == hash,
                "source SDK changed original hash"
            );
            transaction.submit_and_watch().await?;
            broadcast = true;
        }
        sleep_until(deadline.min(Instant::now() + Duration::from_secs(3)).into()).await;
    }
}

async fn signed_normal_message(
    ctx: &Context,
    (api, actor): (&gclient::GearApi, ActorId),
    (journal, path): (&mut Journal, &Path),
    (id, intent): (&str, Value),
    (target, payload, value): (ActorId, Vec<u8>, u128),
    deadline: Instant,
) -> Result<Value> {
    let call =
        gsdk::gear::tx()
            .gear()
            .send_message(target, payload, api.block_gas_limit()?, value, false);
    signed_normal_gear_call(
        ctx,
        (api, actor),
        (journal, path),
        (id, intent),
        call,
        Some(target),
        deadline,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_inbound_leg(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    window: &str,
    amount: u64,
    permit_circle: bool,
    gear_origin: bool,
    deadline: Instant,
) -> Result<Vec<(String, u64, u64)>> {
    let mut lock_actions = Vec::new();
    let ctx_view: &Context = ctx;
    for token in ctx
        .tokens
        .iter()
        .filter(|token| token.gear_origin == gear_origin)
    {
        let amount_evm = EthU256::from(token.raw_amount(amount)?);
        if !(permit_circle && token.symbol == "USDC") {
            let id = format!("{window}/{}-erc20-allowance", token.symbol);
            let erc20 = CampaignToken::new(token.address, ctx.ethereum.api.raw_provider().clone());
            let finalized = ctx.ethereum.api.finalized_block_number().await?;
            let block = ctx
                .ethereum
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Number(finalized))
                .await?
                .context("missing finalized Hoodi block")?;
            let allowance = erc20
                .allowance(ctx.campaign_address, ctx.ethereum.receiver_address().into())
                .block(BlockId::hash_canonical(block.header.hash))
                .call()
                .await?;
            if journal.actions.contains_key(&id) || allowance < amount_evm {
                let token_address = token.address;
                let owner = ctx.campaign_address;
                let spender: Address = ctx.ethereum.receiver_address().into();
                let approve_amount = amount_evm;
                let hash=evm_action(ctx,journal,path,&id,json!({"kind":"erc20-approve","token":format!("{token_address:#x}"),"amountRaw":approve_amount.to_string()}),
                    move|nonce|async move{Ok(CampaignToken::new(token_address,ctx_view.ethereum.api.raw_provider().clone()).approve(spender,approve_amount).nonce(nonce).into_transaction_request())},
                    move|from,nonce|find_approval_tx(ctx_view,token_address,owner,spender,approve_amount,from,nonce)).await?;
                lock_actions.push((id, hash));
            }
        }
    }
    wait_mined_approvals(ctx, journal, path, &lock_actions, deadline).await?;

    let mut lock_hashes = Vec::new();
    for token in ctx
        .tokens
        .iter()
        .filter(|token| token.gear_origin == gear_origin)
    {
        let amount_evm = EthU256::from(token.raw_amount(amount)?);
        let id = format!("{window}/{}-evm-lock", token.symbol);
        let intent = json!({"kind":"erc20-lock","token":format!("{:#x}",token.address),"amountRaw":amount_evm.to_string(),"gearRecipient":format!("0x{}",hex::encode(ctx.campaign_actor.into_bytes())),"permit":permit_circle&&token.symbol=="USDC"});
        let token_address = token.address;
        let recipient = B256::from_slice(&ctx.campaign_actor.into_bytes());
        let permit = permit_circle && token.symbol == "USDC";
        if let Some(approval) = journal
            .actions
            .get(&format!("{window}/{}-erc20-allowance", token.symbol))
        {
            let lock_nonce = journal
                .actions
                .get(&id)
                .and_then(|action| action.nonce)
                .or(journal.next_campaign_nonce)
                .context("dependent lock has no reserved nonce")?;
            ensure_approval_precedes_lock(approval, lock_nonce)?;
        }
        let hash = evm_action(
            ctx,
            journal,
            path,
            &id,
            intent,
            move |nonce| async move {
                let manager = CampaignManager::new(
                    ctx_view.ethereum.receiver_address().into(),
                    ctx_view.ethereum.api.raw_provider().clone(),
                );
                let request = if permit {
                    let (deadline, v, r, s) =
                        circle_permit(ctx_view, token_address, amount_evm).await?;
                    manager
                        .requestBridgingWithPermit(
                            token_address,
                            amount_evm,
                            recipient,
                            EthU256::from(deadline),
                            v,
                            r,
                            s,
                        )
                        .nonce(nonce)
                        .into_transaction_request()
                } else {
                    manager
                        .requestBridging(token_address, amount_evm, recipient)
                        .nonce(nonce)
                        .into_transaction_request()
                };
                Ok(request)
            },
            move |from, nonce| {
                find_lock_tx(ctx_view, token_address, amount_evm, recipient, from, nonce)
            },
        )
        .await?;
        lock_hashes.push((id, hash));
    }
    let lock_action_hashes: Vec<_> = lock_hashes
        .iter()
        .map(|(id, hash)| (id.clone(), *hash))
        .collect();
    let lock_receipts =
        wait_finalized_actions(ctx, journal, path, &lock_action_hashes, deadline).await?;
    // Ordered approvals are audited after lock finality, not on its Beacon-proof critical path.
    wait_finalized_actions(ctx, journal, path, &lock_actions, deadline).await?;
    ensure_beacon_through_receipts(ctx, &lock_receipts, deadline).await?;
    let covered_at = now_ms()?;
    for (id, _) in &lock_hashes {
        record_action_milestone(journal, id, "beaconCoverageObservedAtMs", covered_at)?;
    }
    save_journal(path, journal)?;
    let mut inbound = Vec::new();
    for (action_id, hash) in &lock_hashes {
        let tx_hash = *hash;
        let receipt = if let Some(saved) =
            journal.actions[action_id].evidence.get("incomingReceipt")
        {
            let expected_tx = format!("{tx_hash:#x}");
            ensure!(
                saved["sourceEvmTx"].as_str() == Some(expected_tx.as_str()),
                "saved inbound proof belongs to a different EVM lock transaction"
            );
            let encoded = saved["scalePayload"]
                .as_str()
                .context("saved inbound SCALE payload is missing")?;
            hex::decode(encoded.strip_prefix("0x").unwrap_or(encoded))
                .context("saved inbound SCALE payload is malformed")?;
            saved.clone()
        } else {
            let composed = timeout_at(
                deadline.into(),
                relayer::message_relayer::eth_to_gear::proof_composer::compose(
                    &ctx.beacon,
                    &ctx.gear_api,
                    &ctx.polling_eth,
                    tx_hash,
                    ctx.historical_proxy_id,
                ),
            )
            .await
            .map_err(|_| {
                anyhow!("absolute window deadline expired composing finalized lock proof")
            })??;
            let payload = composed.encode();
            json!({"slot":composed.proof_block.block.slot,"transactionIndex":composed.transaction_index,
                "scalePayload":format!("0x{}",hex::encode(&payload)),"sourceEvmTx":format!("{tx_hash:#x}")})
        };
        let slot = receipt["slot"]
            .as_u64()
            .context("saved inbound proof slot is missing")?;
        let tx_index = receipt["transactionIndex"]
            .as_u64()
            .context("saved inbound proof transaction index is missing")?;
        set_action_evidence(journal, action_id, "incomingReceipt", receipt);
        if journal.actions[action_id].status != "processed" {
            journal
                .actions
                .get_mut(action_id)
                .expect("action exists")
                .status = "receipt_composed".into();
        }
        save_journal(path, journal)?;
        inbound.push((action_id.clone(), slot, tx_index));
    }
    wait_inbound_processed(ctx, journal, path, &inbound, deadline).await?;
    Ok(inbound)
}

#[allow(clippy::too_many_arguments)]
async fn run_roundtrip(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    window: &str,
    fee_mode: FeeMode,
    amount: u64,
    permit_circle: bool,
    deadline: Instant,
) -> Result<()> {
    if journal
        .windows
        .get(window)
        .is_some_and(|v| v["status"] == "passed")
    {
        return Ok(());
    }
    ensure!(
        Instant::now() < deadline,
        "window deadline elapsed before roundtrip"
    );
    require_queue_bootstrap(ctx, journal, path, deadline).await?;
    follower_ready(ctx, 0, deadline).await?;
    let base = if let Some(saved) = journal
        .windows
        .get(window)
        .and_then(|record| record.get("baseline"))
    {
        let saved = SnapshotSet::from_json(saved)?;
        let base = ctx.snapshot_at(Some(&saved)).await?;
        ensure!(
            ctx.tokens
                .iter()
                .all(|token| base.assets.contains_key(token.symbol)),
            "saved campaign baseline token set differs from the four-asset manifest"
        );
        base
    } else {
        let base = ctx.snapshot().await?;
        let record = journal
            .windows
            .entry(window.to_owned())
            .or_insert_with(|| json!({"status":"running","assets":{},"stages":{}}));
        record["baseline"] = base.to_json();
        save_journal(path, journal)?;
        base
    };
    let amount_evm = EthU256::from(amount);
    let amount_gear = GearU256::from(amount);
    let mut inbound = run_inbound_leg(
        ctx,
        journal,
        path,
        window,
        amount,
        permit_circle,
        false,
        deadline,
    )
    .await?;
    let after_mint = if let Some(saved) = journal.windows[window]["stages"].get("afterMint") {
        let saved = SnapshotSet::from_json(saved)?;
        ctx.snapshot_at(Some(&saved)).await?
    } else {
        ensure!(
            !ctx.tokens.iter().any(|token| journal
                .actions
                .contains_key(&format!("{window}/{}-gear-burn", token.symbol))),
            "burn intent exists without original mint checkpoint; HOLD"
        );
        ctx.snapshot().await?
    };
    verify_mint_delta(&base, &after_mint, &ctx.tokens, amount_evm, amount_gear)?;
    let mint_observed_at = now_ms()?;
    for (id, _, _) in &inbound {
        record_action_milestone(journal, id, "mintDeltaObservedAtMs", mint_observed_at)?;
    }
    record_balance_checkpoint(journal, path, window, "afterMint", &after_mint)?;
    if window == "preflight-normal" {
        run_preflight_receipt_probes(ctx, journal, path, deadline).await?;
    }

    let finalized_gear = ctx.source.api.latest_finalized_block().await?;
    let finalized_gear_number = ctx.source.api.block_hash_to_number(finalized_gear).await?;
    let mut vft = Vft::new(ctx.remoting.clone());
    let mut approval_actions = Vec::new();
    for token in &ctx.tokens {
        let amount_gear = GearU256::from(token.raw_amount(amount)?);
        if journal
            .actions
            .contains_key(&format!("{window}/{}-gear-burn", token.symbol))
        {
            continue;
        }
        let allowance = vft
            .allowance(ctx.campaign_actor, ctx.manager_id)
            .at_block(sails_block_hash(finalized_gear))
            .recv(token.peer)
            .await?;
        let id = format!("{window}/{}-vft-allowance", token.symbol);
        let burn_id = format!("{window}/{}-gear-burn", token.symbol);
        if journal.actions.contains_key(&burn_id) {
            continue;
        }
        let intent = json!({"kind":"vft-approve","peer":token.component,"amountRaw":token.raw_amount(amount)?});
        if allowance >= amount_gear {
            if let Some(action) = journal.actions.get_mut(&id) {
                ensure!(
                    action.intent == intent,
                    "Gear VFT approval intent changed for {}",
                    token.symbol
                );
                action.status = "finalized".into();
                action.evidence["finalizedAllowance"] = json!(allowance.to_string());
                action.evidence["finalizedBlock"] = json!(finalized_gear_number);
                action.evidence["finalizedBlockHash"] = json!(format!("{finalized_gear:#x}"));
                save_journal(path, journal)?;
            }
            continue;
        }
        if ctx.schedule.handovers == 2 {
            signed_normal_message(
                ctx,
                (&ctx.gear_api, ctx.campaign_actor),
                (journal, path),
                (&id, intent),
                (
                    token.peer,
                    vft_client::vft::io::Approve::encode_call(ctx.manager_id, amount_gear),
                    0,
                ),
                deadline,
            )
            .await?;
            let reply = normal_message_reply(
                ctx,
                ctx.campaign_actor,
                journal,
                path,
                &id,
                token.peer,
                deadline,
            )
            .await?;
            ensure!(
                decode_campaign_reply::<bool>(&reply, vft_client::vft::io::Approve::ROUTE)?,
                "VFT approval rejected; HOLD"
            );
            approval_actions.push((id, token.symbol));
            continue;
        }
        ensure!(
            prepare_gear_action(journal, path, &id, intent)?,
            "Gear VFT approval for {} is unresolved; refusing to resend",
            token.symbol
        );
        let reply = match vft
            .approve(ctx.manager_id, amount_gear)
            .send(token.peer)
            .await
        {
            Ok(reply) => reply,
            Err(error) => {
                let action = journal
                    .actions
                    .get_mut(&id)
                    .expect("approval action exists");
                action.status = "ambiguous".into();
                action.evidence["diagnostic"] = json!(format!(
                    "Sails send returned an error without an extrinsic identity: {error:?}"
                ));
                save_journal(path, journal)?;
                return Err(anyhow!(
                    "Gear VFT approval dispatch is ambiguous; refusing to resend: {error:?}"
                ));
            }
        };
        let action = journal
            .actions
            .get_mut(&id)
            .expect("approval action exists");
        action.status = "ambiguous".into();
        action.evidence["dispatch"] = json!(
            "Sails send returned no extrinsic hash; reconcile only finalized allowance state"
        );
        save_journal(path, journal)?;
        approval_actions.push((id, token.symbol));
        drop(reply);
    }
    save_journal(path, journal)?;
    let approval_block = ctx.source.api.latest_finalized_block().await?;
    let approval_block_number = ctx.source.api.block_hash_to_number(approval_block).await?;
    for (id, symbol) in &approval_actions {
        let token = ctx
            .tokens
            .iter()
            .find(|token| token.symbol == *symbol)
            .context("approved VFT is missing")?;
        let allowance = vft
            .allowance(ctx.campaign_actor, ctx.manager_id)
            .at_block(sails_block_hash(approval_block))
            .recv(token.peer)
            .await?;
        ensure!(
            allowance >= GearU256::from(token.raw_amount(amount)?),
            "{} VFT allowance is not finalized at the expected amount",
            token.symbol
        );
        let action = journal.actions.get_mut(id).expect("approval action exists");
        action.status = "finalized".into();
        action.evidence["finalizedAllowance"] = json!(allowance.to_string());
        action.evidence["finalizedBlock"] = json!(approval_block_number);
        action.evidence["finalizedBlockHash"] = json!(format!("{approval_block:#x}"));
    }
    save_journal(path, journal)?;

    let finalized_request_head = ctx.source.api.latest_finalized_block().await?;
    let request_start_default = ctx
        .source
        .api
        .block_hash_to_number(finalized_request_head)
        .await?
        .checked_add(1)
        .context("Gear request start block overflow")?;
    let mut request_start = request_start_default;
    let mut manager = VftManager::new(ctx.remoting.clone());
    let manager_config = manager.get_config().recv(ctx.manager_id).await?;
    let request_gas = ctx.gear_api.block_gas_limit()?;
    let mut request_actions = Vec::new();
    for token in &ctx.tokens {
        let amount_gear = GearU256::from(token.raw_amount(amount)?);
        let id = format!("{window}/{}-gear-burn", token.symbol);
        let start = match journal.actions.get(&id) {
            Some(action) => u32::try_from(
                action.intent["startBlock"]
                    .as_u64()
                    .context("saved Gear request start block missing")?,
            )?,
            None => request_start_default,
        };
        let intent = json!({"kind":"vft-manager-request","peer":token.component,"amountRaw":token.raw_amount(amount)?,"receiver":format!("{:#x}",ctx.campaign_address),"startBlock":start});
        let should_send = if ctx.schedule.handovers != 2 {
            prepare_gear_action(journal, path, &id, intent.clone())?
        } else {
            false
        };
        request_start = request_start.min(start);
        request_actions.push((id.clone(), token.symbol.to_owned(), start));
        if ctx.schedule.handovers == 2 {
            signed_normal_message(
                ctx,
                (&ctx.gear_api, ctx.campaign_actor),
                (journal, path),
                (&id, intent),
                (
                    ctx.manager_id,
                    vft_manager_client::vft_manager::io::RequestBridging::encode_call(
                        token.peer,
                        amount_gear,
                        H160::from_slice(ctx.campaign_address.as_slice()),
                    ),
                    manager_config.fee_incoming,
                ),
                deadline,
            )
            .await?;
            let reply = normal_message_reply(
                ctx,
                ctx.campaign_actor,
                journal,
                path,
                &id,
                ctx.manager_id,
                deadline,
            )
            .await?;
            ensure!(
                decode_campaign_reply::<Result<(GearU256, H160), vft_manager_client::Error>>(
                    &reply,
                    vft_manager_client::vft_manager::io::RequestBridging::ROUTE
                )?
                .is_ok(),
                "original manager request rejected; HOLD"
            );
            continue;
        }
        if should_send {
            let reply = match manager
                .request_bridging(
                    token.peer,
                    amount_gear,
                    H160::from_slice(ctx.campaign_address.as_slice()),
                )
                .with_value(manager_config.fee_incoming)
                .with_gas_limit(request_gas)
                .send(ctx.manager_id)
                .await
            {
                Ok(reply) => reply,
                Err(error) => {
                    let action = journal
                        .actions
                        .get_mut(&id)
                        .expect("Gear request action exists");
                    action.status = "ambiguous".into();
                    action.evidence["diagnostic"] = json!(format!(
                        "Sails send returned an error without an extrinsic identity: {error:?}"
                    ));
                    save_journal(path, journal)?;
                    return Err(anyhow!(
                        "Gear burn/request dispatch is ambiguous; refusing to resend: {error:?}"
                    ));
                }
            };
            let action = journal
                .actions
                .get_mut(&id)
                .expect("Gear request action exists");
            action.status = "ambiguous".into();
            action.evidence["dispatch"] = json!("Sails send returned no extrinsic hash; reconcile only a unique finalized manager event");
            save_journal(path, journal)?;
            drop(reply);
        }
    }
    let outbound = reconcile_outbound_requests(
        ctx,
        journal,
        path,
        &request_actions,
        request_start,
        amount_gear,
        deadline,
    )
    .await?;
    let (normal_fee, priority_fee) = ctx.fee_state().await?;
    let finalized_payment_head = ctx.source.api.latest_finalized_block().await?;
    let payment_start_default = ctx
        .source
        .api
        .block_hash_to_number(finalized_payment_head)
        .await?
        .checked_add(1)
        .context("paid-event start block overflow")?;
    let mut payment = BridgingPayment::new(ctx.remoting.clone());
    let mut pay_actions = Vec::new();
    for (id, token, request) in &outbound {
        let pay_id = format!("{window}/{}-paid", token.symbol);
        let start = match journal.actions.get(&pay_id) {
            Some(action) => u32::try_from(
                action.intent["startBlock"]
                    .as_u64()
                    .context("saved paid-event start block missing")?,
            )?,
            None => payment_start_default,
        };
        let fee_raw = match fee_mode {
            FeeMode::Normal => normal_fee,
            FeeMode::Priority => priority_fee,
        };
        let payload = json!({"kind":"bridging-paid","feeMode":fee_mode,"nonce":request.nonce.to_string(),
            "queueBlockHash":format!("0x{}",hex::encode(request.queue_block_hash.0)),"feeRaw":fee_raw.to_string(),"startBlock":start});
        let should_send = if ctx.schedule.handovers != 2 {
            prepare_gear_action(journal, path, &pay_id, payload.clone())?
        } else {
            false
        };
        pay_actions.push((pay_id.clone(), id.clone(), start, request.clone()));
        if ctx.schedule.handovers == 2 {
            let (bytes, route) = match fee_mode {
                FeeMode::Normal => (
                    bridging_payment_client::bridging_payment::io::PayFees::encode_call(
                        request.nonce,
                    ),
                    bridging_payment_client::bridging_payment::io::PayFees::ROUTE,
                ),
                FeeMode::Priority => (
                    bridging_payment_client::bridging_payment::io::PayPriorityFees::encode_call(
                        sails_block_hash(request.queue_block_hash),
                        request.nonce,
                    ),
                    bridging_payment_client::bridging_payment::io::PayPriorityFees::ROUTE,
                ),
            };
            signed_normal_message(
                ctx,
                (&ctx.gear_api, ctx.campaign_actor),
                (journal, path),
                (&pay_id, payload),
                (ctx.payment_id, bytes, fee_raw),
                deadline,
            )
            .await?;
            let reply = normal_message_reply(
                ctx,
                ctx.campaign_actor,
                journal,
                path,
                &pay_id,
                ctx.payment_id,
                deadline,
            )
            .await?;
            decode_campaign_reply::<()>(&reply, route)?;
            continue;
        }
        if should_send {
            let dispatched = match fee_mode {
                FeeMode::Normal => payment
                    .pay_fees(request.nonce)
                    .with_value(normal_fee)
                    .send(ctx.payment_id)
                    .await
                    .map(drop),
                FeeMode::Priority => payment
                    .pay_priority_fees(sails_block_hash(request.queue_block_hash), request.nonce)
                    .with_value(priority_fee)
                    .send(ctx.payment_id)
                    .await
                    .map(drop),
            };
            match dispatched {
                Ok(()) => {
                    let action = journal
                        .actions
                        .get_mut(&pay_id)
                        .expect("paid action exists");
                    action.status = "ambiguous".into();
                    action.evidence["dispatch"] = json!("Sails send returned no extrinsic hash; reconcile only a unique finalized paid event");
                    save_journal(path, journal)?;
                }
                Err(error) => {
                    let action = journal
                        .actions
                        .get_mut(&pay_id)
                        .expect("paid action exists");
                    action.status = "ambiguous".into();
                    action.evidence["diagnostic"] = json!(format!(
                        "Sails send returned an error without an extrinsic identity: {error:?}"
                    ));
                    save_journal(path, journal)?;
                }
            }
        }
    }
    for (pay_id, _, start, request) in &pay_actions {
        let event = scan_paid_event(ctx, *start, request, fee_mode, deadline).await?;
        if let Some(saved) = journal.actions[pay_id].evidence.get("paidEvent") {
            ensure!(
                saved["blockHash"] == event["blockHash"],
                "replayed paid-event canonical identity changed"
            );
        }
        journal
            .actions
            .get_mut(pay_id)
            .expect("payment action exists")
            .status = "reconciled".into();
        set_action_evidence(journal, pay_id, "paidEvent", event);
        record_action_milestone(journal, pay_id, "paymentFinalizedObservedAtMs", now_ms()?)?;
        save_journal(path, journal)?;
    }
    save_journal(path, journal)?;

    let mut roots =
        BTreeMap::<(u32, [u8; 32]), (QueueRoot, Vec<(String, String, OutboundRequest)>)>::new();
    for (id, token, request) in &outbound {
        let root = find_covering_root(ctx, request, deadline).await?;
        roots
            .entry((root.source_block, root.root))
            .or_insert_with(|| (root.clone(), Vec::new()))
            .1
            .push((id.clone(), token.symbol.to_owned(), request.clone()));
    }
    let mut root_futures = FuturesUnordered::new();
    for (_, (root, messages)) in roots {
        let view: &Context = ctx;
        root_futures.push(async move {
            let result = wait_actor_root(view, &root, deadline).await;
            (root, messages, result)
        });
    }
    while let Some((root, messages, result)) = root_futures.next().await {
        let mut actor_evidence = result?;
        // A recovered registration may be observed after its release already finalized.
        actor_evidence["ethScanStartBlock"] = json!(base.evm_height);
        let root_id = format!("{}:{}", root.source_block, hex::encode(root.root));
        for (action_id, symbol, request) in messages {
            let mut evidence = actor_evidence.clone();
            evidence["requestNonce"] = json!(request.nonce.to_string());
            evidence["asset"] = json!(symbol);
            evidence["rootId"] = json!(root_id);
            record_action_milestone(
                journal,
                &action_id,
                "rootRegisteredAtMs",
                number(&evidence["registrationTimestampMs"])?,
            )?;
            record_action_milestone(
                journal,
                &action_id,
                "rootMaturityEligibleAtMs",
                number(&evidence["maturityEligibleAtMs"])?,
            )?;
            record_action_milestone(
                journal,
                &action_id,
                "rootPublicationObservedAtMs",
                number(&evidence["publicationObservedAtMs"])?,
            )?;
            set_action_evidence(journal, &action_id, "rootPublication", evidence);
        }
        save_journal(path, journal)?;
    }

    let mut release_futures = FuturesUnordered::new();
    for (id, token, request) in &outbound {
        let action = &journal.actions[id];
        let publication = action
            .evidence
            .get("rootPublication")
            .context("root publication evidence is missing")?;
        let root_block = publication["sourceBlock"]
            .as_u64()
            .context("root source block missing")?;
        let scan_from = publication["ethScanStartBlock"]
            .as_u64()
            .context("root publication EVM scan block missing")?;
        let nonce = eth_u256(request.nonce)?;
        let id = id.clone();
        let hash = request.message_hash;
        let view: &Context = ctx;
        let token = token.clone();
        let token_amount = EthU256::from(token.raw_amount(amount)?);
        release_futures.push(async move {
            (
                id,
                wait_release(
                    view,
                    nonce,
                    hash,
                    root_block,
                    scan_from,
                    deadline,
                    (&token, token_amount),
                )
                .await,
            )
        });
    }
    while let Some((id, result)) = release_futures.next().await {
        let observed = result?;
        let release = if let Some(saved) = journal.actions[&id].evidence.get("releaseReceipt") {
            for field in [
                "transactionHash",
                "sourceBlock",
                "messageHash",
                "nonce",
                "destination",
            ] {
                ensure!(
                    saved[field] == observed[field],
                    "original release identity changed; HOLD"
                );
            }
            for field in ["transactionHash", "receiptBlock", "receiptBlockHash"] {
                ensure!(
                    saved["receipt"][field] == observed["receipt"][field],
                    "original release inclusion changed; HOLD"
                );
            }
            let anchor: B256 =
                string(&saved["finalizedHash"], "saved release finality anchor")?.parse()?;
            ensure!(
                ctx.ethereum
                    .api
                    .is_finalized_block(number(&saved["finalizedBlock"])?, anchor.0.into())
                    .await?,
                "original release finality anchor changed; HOLD"
            );
            saved.clone()
        } else {
            observed
        };
        record_action_milestone(
            journal,
            &id,
            "rootMaturityObservedAtMs",
            number(&release["maturityObservedAtMs"])?,
        )?;
        record_action_milestone(
            journal,
            &id,
            "releaseFinalityObservedAtMs",
            number(&release["receipt"]["finalityObservedAtMs"])?,
        )?;
        set_action_evidence(journal, &id, "releaseReceipt", release);
        journal
            .actions
            .get_mut(&id)
            .expect("outbound action exists")
            .status = "released".into();
        save_journal(path, journal)?;
    }
    if ctx.schedule.handovers == 2 {
        let after_export =
            if let Some(saved) = journal.windows[window]["stages"].get("afterGearExport") {
                ctx.snapshot_at(Some(&SnapshotSet::from_json(saved)?))
                    .await?
            } else {
                ensure!(
                    !ctx.tokens
                        .iter()
                        .filter(|token| token.gear_origin)
                        .any(|token| journal
                            .actions
                            .contains_key(&format!("{window}/{}-evm-lock", token.symbol))),
                    "Gear return intent lacks original export checkpoint; HOLD"
                );
                ctx.snapshot().await?
            };
        verify_gear_export_delta(&base, &after_export, &ctx.tokens, amount)?;
        record_balance_checkpoint(journal, path, window, "afterGearExport", &after_export)?;
        inbound.extend(
            run_inbound_leg(ctx, journal, path, window, amount, false, true, deadline).await?,
        );
        let returned_at = now_ms()?;
        for (id, _, _) in inbound.iter().filter(|(id, _, _)| {
            ctx.tokens.iter().any(|token| {
                token.gear_origin && id == &format!("{window}/{}-evm-lock", token.symbol)
            })
        }) {
            record_action_milestone(journal, id, "mintDeltaObservedAtMs", returned_at)?;
        }
    }
    let final_state = if let Some(saved) = journal.windows[window]["stages"].get("finalizedReturn")
    {
        let saved = SnapshotSet::from_json(saved)?;
        ctx.snapshot_at(Some(&saved)).await?
    } else {
        ctx.snapshot().await?
    };
    verify_settlement_delta(&base, &final_state, &ctx.tokens, amount)?;
    record_balance_checkpoint(journal, path, window, "finalizedReturn", &final_state)?;
    wait_inbound_processed(ctx, journal, path, &inbound, deadline).await?;
    let record = journal
        .windows
        .get_mut(window)
        .context("window journal missing")?;
    record["assets"] = json!(ctx
        .tokens
        .iter()
        .map(|token| {
            let lock = journal
                .actions
                .get(&format!("{window}/{}-evm-lock", token.symbol));
            let burn = journal
                .actions
                .get(&format!("{window}/{}-gear-burn", token.symbol));
            let paid = journal
                .actions
                .get(&format!("{window}/{}-paid", token.symbol));
            (
                token.symbol,
                json!({"status":"passed","amountRaw":token.raw_amount(amount).expect("validated raw amount"),"origin":if token.gear_origin {"Gear"}else{"Ethereum"},"native":token.native_amount.is_some(),
            "lock":lock.map(|x|x.evidence.clone()).unwrap_or(Value::Null),
            "burn":burn.map(|x|x.evidence.clone()).unwrap_or(Value::Null),
            "paid":paid.map(|x|x.evidence.clone()).unwrap_or(Value::Null)}),
            )
        })
        .collect::<BTreeMap<_, _>>());
    record["completedAtMs"] = json!(now_ms()?);
    save_journal(path, journal)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum ReceiptProbe {
    InvalidReceiptProof,
    ProcessedReceiptReplay,
}

impl ReceiptProbe {
    fn key(self) -> &'static str {
        match self {
            Self::InvalidReceiptProof => "invalidReceiptProof",
            Self::ProcessedReceiptReplay => "processedReceiptReplay",
        }
    }

    fn action_id(self) -> &'static str {
        match self {
            Self::InvalidReceiptProof => "preflight-normal/USDC-invalid-receipt-proof",
            Self::ProcessedReceiptReplay => "preflight-normal/USDC-processed-receipt-replay",
        }
    }

    fn rejection(self) -> &'static str {
        match self {
            Self::InvalidReceiptProof => "EthereumEventClient(InvalidReceiptProof)",
            Self::ProcessedReceiptReplay => "AlreadyProcessed",
        }
    }
}

const RECEIPT_PROBES: [ReceiptProbe; 2] = [
    ReceiptProbe::InvalidReceiptProof,
    ReceiptProbe::ProcessedReceiptReplay,
];
const PROBE_LOCK_ID: &str = "preflight-normal/USDC-evm-lock";

fn receipt_probes_completed(journal: &Journal) -> Result<bool> {
    let mut complete = true;
    for probe in RECEIPT_PROBES {
        let record = &journal.preflight.evidence["receiptProbes"][probe.key()];
        if record.is_null() {
            complete = false;
            continue;
        }
        let action = journal
            .actions
            .get(probe.action_id())
            .context("receipt probe action missing; HOLD")?;
        ensure!(
            record["status"] == "passed"
                && record["actionId"] == probe.action_id()
                && action.status == "finalized"
                && action.evidence["decodedRejection"] == probe.rejection()
                && action.evidence["reply"].is_object()
                && action.evidence["before"].is_object()
                && action.evidence["after"].is_object()
                && action.evidence["receiptStatusAfter"]["status"] == "Processed"
                && number(&action.evidence["completedAtMs"])?
                    < number(&action.intent["batchDeadlineMs"])?,
            "live receipt probe evidence is incomplete; HOLD"
        );
    }
    Ok(complete)
}

fn ensure_preflight_probe_boundary(journal: &Journal) -> Result<()> {
    if journal.preflight.status == "passed"
        || journal
            .windows
            .get("preflight-normal")
            .is_some_and(|window| window["status"] == "passed")
        || journal
            .actions
            .keys()
            .any(|id| id.starts_with("preflight-normal/") && id.ends_with("-gear-burn"))
    {
        ensure!(
            receipt_probes_completed(journal)?,
            "normal preflight has no pre-burn live receipt probes; HOLD"
        );
    }
    Ok(())
}

fn decode_probe_event(bytes: &[u8]) -> Result<EthToVaraEvent> {
    let mut input = bytes;
    let event =
        EthToVaraEvent::decode(&mut input).context("decode original receipt proof; HOLD")?;
    ensure!(
        input.is_empty(),
        "original receipt proof has trailing SCALE bytes; HOLD"
    );
    Ok(event)
}

fn probe_rejection(probe: ReceiptProbe, raw: &[u8], receipt: &[u8]) -> Result<&'static str> {
    let reply = Redirect::decode_reply(raw).context("decode HistoricalProxy probe reply; HOLD")?;
    ensure!(
        Redirect::ROUTE.len() + reply.encoded_size() == raw.len(),
        "proxy reply has trailing bytes; HOLD"
    );
    match (probe, reply) {
        (
            ReceiptProbe::InvalidReceiptProof,
            Err(historical_proxy_client::ProxyError::EthereumEventClient(
                historical_proxy_client::Error::InvalidReceiptProof,
            )),
        ) => {}
        (ReceiptProbe::ProcessedReceiptReplay, Ok((returned, manager_reply))) => {
            ensure!(
                returned == receipt,
                "replay proxy returned a different receipt; HOLD"
            );
            let reply = SubmitReceipt::decode_reply(&manager_reply)
                .context("decode replay manager reply; HOLD")?;
            ensure!(
                SubmitReceipt::ROUTE.len() + reply.encoded_size() == manager_reply.len(),
                "manager reply has trailing bytes; HOLD"
            );
            ensure!(
                matches!(reply, Err(vft_manager_client::Error::AlreadyProcessed)),
                "replay did not produce AlreadyProcessed; HOLD"
            );
        }
        _ => bail!("receipt probe produced a different application rejection; HOLD"),
    }
    Ok(probe.rejection())
}

fn prepare_receipt_probe(
    journal: &mut Journal,
    path: &Path,
    probe: ReceiptProbe,
    intent: Value,
    before: &SnapshotSet,
) -> Result<bool> {
    if let Some(action) = journal.actions.get(probe.action_id()) {
        ensure!(
            SnapshotSet::from_json(&action.evidence["before"])? == *before,
            "original pre-probe snapshot changed or is missing; HOLD"
        );
    }
    let send = prepare_gear_action(journal, path, probe.action_id(), intent)?;
    if send {
        set_action_evidence(journal, probe.action_id(), "before", before.to_json());
        record_action_milestone(
            journal,
            probe.action_id(),
            "preSendFinalizedObservedAtMs",
            now_ms()?,
        )?;
        save_journal(path, journal)?;
    }
    Ok(send)
}

fn record_probe_submission(
    journal: &mut Journal,
    path: &Path,
    probe: ReceiptProbe,
    message_id: GearHash,
    enqueue_hash: GearHash,
) -> Result<()> {
    let action = journal
        .actions
        .get_mut(probe.action_id())
        .context("probe intent disappeared")?;
    ensure!(
        action.status == "broadcasting" && action.evidence.get("submission").is_none(),
        "probe submission identity cannot be replaced; HOLD"
    );
    action.status = "submitted".into();
    action.evidence["submission"] = json!({"messageId":format!("{message_id:#x}"),
        "enqueueBlockHash":format!("{enqueue_hash:#x}"),"observedAtMs":now_ms()?});
    save_journal(path, journal)
}

#[allow(clippy::too_many_arguments)]
fn finalize_receipt_probe(
    journal: &mut Journal,
    path: &Path,
    probe: ReceiptProbe,
    reply: Value,
    before: &SnapshotSet,
    after: &SnapshotSet,
    tokens: &[Token],
    status: GearReceiptStatus,
    deadline: Instant,
) -> Result<()> {
    ensure!(
        Instant::now() < deadline,
        "original receipt probe deadline expired; HOLD"
    );
    let action = journal
        .actions
        .get(probe.action_id())
        .context("probe intent missing")?;
    ensure!(
        matches!(action.status.as_str(), "submitted" | "finalized"),
        "probe has no durable submission; HOLD"
    );
    ensure!(
        matches!(status, GearReceiptStatus::Processed),
        "original receipt is no longer Processed; HOLD"
    );
    ensure!(
        SnapshotSet::from_json(&action.evidence["before"])? == *before,
        "historical pre-probe snapshot changed; HOLD"
    );
    verify_roundtrip_delta(before, after, tokens)?;
    let submission = &action.evidence["submission"];
    ensure!(
        reply["messageId"] == submission["messageId"]
            && reply["enqueueBlockHash"] == submission["enqueueBlockHash"]
            && reply["enqueueHeight"] == submission["enqueueHeight"]
            && reply["source"] == action.intent["proxy"]
            && reply["destination"] == action.intent["sender"]
            && reply["runtimeSuccess"] == true,
        "probe reply does not match its original successful dispatch; HOLD"
    );
    for key in ["messageId", "enqueueBlockHash", "replyId", "finalizedHash"] {
        bytes32(string(&reply[key], "probe reply identity")?)?;
    }
    let enqueue = number(&reply["enqueueHeight"])?;
    let finalized = number(&reply["finalizedHeight"])?;
    ensure!(
        u64::from(before.gear_height) <= enqueue
            && enqueue <= finalized
            && finalized <= u64::from(after.gear_height),
        "probe snapshots do not cover its finalized reply; HOLD"
    );
    let raw = hex::decode(
        string(&reply["rawReply"], "raw finalized probe reply")?.trim_start_matches("0x"),
    )?;
    let receipt = hex::decode(
        string(&action.intent["receiptRlp"], "original probe receipt")?.trim_start_matches("0x"),
    )?;
    let rejection = probe_rejection(probe, &raw, &receipt)?;
    let completed = if action.status == "finalized" {
        ensure!(
            action.evidence["reply"] == reply
                && SnapshotSet::from_json(&action.evidence["after"])? == *after,
            "original probe reply inclusion or accounting changed; HOLD"
        );
        number(&action.evidence["completedAtMs"])?
    } else {
        now_ms()?
    };
    let reply_observed = number(&reply["observedAtMs"])?;
    ensure!(
        reply_observed <= completed && completed < number(&action.intent["batchDeadlineMs"])?,
        "receipt probe did not complete before its original absolute deadline; HOLD"
    );
    if action.status != "finalized" {
        set_action_evidence(journal, probe.action_id(), "reply", reply);
        set_action_evidence(journal, probe.action_id(), "after", after.to_json());
        set_action_evidence(
            journal,
            probe.action_id(),
            "decodedRejection",
            json!(rejection),
        );
        set_action_evidence(
            journal,
            probe.action_id(),
            "receiptStatusAfter",
            json!({"status":"Processed",
            "receiptKey":journal.actions[probe.action_id()].intent["originalReceiptKey"],
            "finalizedBlock":after.gear_height,"finalizedHash":format!("{:#x}",after.gear_hash)}),
        );
        set_action_evidence(
            journal,
            probe.action_id(),
            "completedAtMs",
            json!(completed),
        );
        record_action_milestone(
            journal,
            probe.action_id(),
            "gearFinalizedReplyObservedAtMs",
            reply_observed,
        )?;
        record_action_milestone(
            journal,
            probe.action_id(),
            "accountingUnchangedObservedAtMs",
            completed,
        )?;
        journal
            .actions
            .get_mut(probe.action_id())
            .expect("probe exists")
            .status = "finalized".into();
    }
    let verdict = json!({"status":"passed","actionId":probe.action_id()});
    let existing = &journal.preflight.evidence["receiptProbes"][probe.key()];
    ensure!(
        existing.is_null() || *existing == verdict,
        "original receipt probe verdict changed; HOLD"
    );
    journal.preflight.evidence["receiptProbes"][probe.key()] = verdict;
    save_journal(path, journal)
}

async fn original_probe_event(ctx: &Context, journal: &Journal) -> Result<EthToVaraEvent> {
    let lock = journal
        .actions
        .get(PROBE_LOCK_ID)
        .context("original USDC lock is missing; HOLD")?;
    let incoming = &lock.evidence["incomingReceipt"];
    let hash: B256 = lock
        .tx_hash
        .as_deref()
        .context("USDC lock transaction is missing; HOLD")?
        .parse()?;
    ensure!(
        incoming["sourceEvmTx"] == format!("{hash:#x}"),
        "probe receipt refers to another lock; HOLD"
    );
    let payload = hex::decode(
        string(&incoming["scalePayload"], "original SCALE receipt proof")?.trim_start_matches("0x"),
    )?;
    let event = decode_probe_event(&payload)?;
    let key = (
        number(&incoming["slot"])?,
        number(&incoming["transactionIndex"])?,
    );
    ensure!(
        (event.proof_block.block.slot, event.transaction_index) == key
            && lock.evidence["receiptStatus"]["status"] == "Processed",
        "original receipt key or Processed evidence is missing; HOLD"
    );
    let worker = ctx
        .inbound_journal()?
        .context("independent inbound journal is missing; HOLD")?;
    let mut matches = worker
        .transactions
        .values()
        .chain(worker.completed.values())
        .filter(|tx| tx.tx.tx_hash.as_slice() == hash.as_slice());
    let tx = matches
        .next()
        .context("independent worker has no original USDC lock; HOLD")?;
    ensure!(
        matches.next().is_none(),
        "worker has conflicting USDC lock identities; HOLD"
    );
    let receipt = tx
        .receipt
        .as_ref()
        .context("independent worker proof is missing; HOLD")?;
    ensure!(
        receipt.receipt_key == key
            && receipt.composed_at_ms > 0
            && receipt
                .handed_off_at_ms
                .is_some_and(|at| at >= receipt.composed_at_ms)
            && lock.evidence["workerProof"]
                == json!({"receiptKey":key,"workerTransaction":tx.uuid,
            "payloadKeccak256":format!("0x{}",hex::encode(keccak256(&receipt.payload)))})
            && lock.evidence["milestones"]["workerProofHandoffAtMs"].as_u64()
                == receipt.handed_off_at_ms,
        "independent original proof/handoff evidence is missing or changed; HOLD"
    );
    let worker_event = decode_probe_event(&receipt.payload)?;
    ensure!(
        (
            worker_event.proof_block.block.slot,
            worker_event.transaction_index
        ) == key
            && worker_event.receipt_rlp == event.receipt_rlp
            && worker_event.proof == event.proof
            && worker_event.proof_block.block.encode() == event.proof_block.block.encode(),
        "independent proof belongs to a different receipt/block; HOLD"
    );
    Ok(event)
}

async fn find_receipt_probe_reply(
    ctx: &Context,
    action: &Action,
    deadline: Instant,
) -> Result<Value> {
    let submission = &action.evidence["submission"];
    let message_id: GearHash =
        string(&submission["messageId"], "original probe message ID; HOLD")?.parse()?;
    let enqueue_hash: GearHash = string(
        &submission["enqueueBlockHash"],
        "original probe enqueue hash; HOLD",
    )?
    .parse()?;
    let enqueue = u32::try_from(number(&submission["enqueueHeight"])?)?;
    let saved_reply = action.evidence.get("reply");
    let mut next = match saved_reply {
        Some(reply) => u32::try_from(number(&reply["finalizedHeight"])?)?,
        None => enqueue,
    };
    let mut enqueue_checked = false;
    loop {
        ensure!(
            Instant::now() < deadline,
            "deadline expired without an original finalized probe reply; HOLD"
        );
        let source_head = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?;
        let witness_head = ctx
            .witness
            .api
            .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
            .await?;
        let head = source_head.min(witness_head);
        if head >= enqueue && !enqueue_checked {
            ensure!(
                ctx.source.api.block_number_to_hash(enqueue).await? == enqueue_hash
                    && ctx.witness.api.block_number_to_hash(enqueue).await? == enqueue_hash,
                "original probe enqueue inclusion changed; HOLD"
            );
            let block = ctx.source.api.api.blocks().at(enqueue_hash).await?;
            let mut found = false;
            for event in block.events().await?.iter() {
                if let RuntimeEvent::Gear(GearEvent::MessageQueued {
                    id,
                    source,
                    destination,
                    ..
                }) = event?.as_gear()?
                {
                    if GearHash::from_slice(id.as_ref()) == message_id {
                        ensure!(
                            source.0 == ctx.campaign_actor.into_bytes()
                                && destination == ctx.historical_proxy_id,
                            "original probe enqueue sender/proxy changed; HOLD"
                        );
                        found = true;
                    }
                }
            }
            ensure!(
                found,
                "original probe message has no canonical enqueue event; HOLD"
            );
            enqueue_checked = true;
        }
        let start = next;
        for height in start..=head {
            let hash = ctx.source.api.block_number_to_hash(height).await?;
            ensure!(
                ctx.witness.api.block_number_to_hash(height).await? == hash,
                "probe source/witness canonical hashes differ; HOLD"
            );
            let block = ctx.source.api.api.blocks().at(hash).await?;
            for event in block.events().await?.iter() {
                let RuntimeEvent::Gear(GearEvent::UserMessageSent { message, .. }) =
                    event?.as_gear()?
                else {
                    continue;
                };
                let Some(details) = message.details() else {
                    continue;
                };
                if GearHash::from_slice(details.to_message_id().as_ref()) != message_id {
                    continue;
                }
                ensure!(
                    message.source() == ctx.historical_proxy_id
                        && message.destination() == ctx.campaign_actor,
                    "probe reply source/destination differs; HOLD"
                );
                ensure!(
                    details.to_reply_code().is_success(),
                    "probe runtime dispatch failed; HOLD"
                );
                let reply = json!({"messageId":format!("{message_id:#x}"),"enqueueHeight":enqueue,
                    "enqueueBlockHash":format!("{enqueue_hash:#x}"),"replyId":format!("0x{}",hex::encode(message.id().into_bytes())),
                    "finalizedHeight":height,"finalizedHash":format!("{hash:#x}"),
                    "source":format!("0x{}",hex::encode(message.source().into_bytes())),
                    "destination":format!("0x{}",hex::encode(message.destination().into_bytes())),
                    "runtimeSuccess":true,"rawReply":format!("0x{}",hex::encode(message.payload_bytes())),
                    "observedAtMs":match saved_reply { Some(reply) => number(&reply["observedAtMs"])?, None => now_ms()? }});
                ensure!(
                    saved_reply.is_none_or(|saved| *saved == reply),
                    "original finalized probe reply changed; HOLD"
                );
                return Ok(reply);
            }
            ensure!(
                saved_reply.is_none(),
                "original finalized probe reply disappeared; HOLD"
            );
            next = height.checked_add(1).context("probe reply scan overflow")?;
        }
        sleep(Duration::from_secs(6)).await;
    }
}

async fn run_preflight_receipt_probes(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    deadline: Instant,
) -> Result<()> {
    timeout_at(deadline.into(), async {
        ensure_preflight_probe_boundary(journal)?;
        let original = original_probe_event(ctx, journal).await?;
        let key = (original.proof_block.block.slot, original.transaction_index);
        for probe in RECEIPT_PROBES {
            wait_inbound_processed(ctx, journal, path, &[(PROBE_LOCK_ID.into(), key.0, key.1)], deadline).await?;
            let before = match journal.actions.get(probe.action_id()) {
                Some(action) => ctx.snapshot_at(Some(&SnapshotSet::from_json(&action.evidence["before"])?)).await?,
                None => ctx.snapshot().await?,
            };
            let (index, payload) = match probe {
                ReceiptProbe::InvalidReceiptProof => {
                    let mut invalid = original.clone();
                    invalid.transaction_index = u64::MAX;
                    (invalid.transaction_index, invalid.encode())
                }
                ReceiptProbe::ProcessedReceiptReplay => (original.transaction_index, original.encode()),
            };
            let mut intent = json!({"kind":"receipt-rejection-probe","originalLockId":PROBE_LOCK_ID,
                "originalLockHash":journal.actions[PROBE_LOCK_ID].tx_hash,"originalReceiptKey":key,
                "submittedTransactionIndex":index,"scalePayload":format!("0x{}",hex::encode(&payload)),
                "payloadKeccak256":format!("0x{}",hex::encode(keccak256(&payload))),
                "receiptRlp":format!("0x{}",hex::encode(&original.receipt_rlp)),
                "sourceGenesis":journal.source_genesis,"sender":format!("0x{}",hex::encode(ctx.campaign_actor.into_bytes())),
                "proxy":format!("0x{}",hex::encode(ctx.historical_proxy_id.into_bytes())),
                "manager":format!("0x{}",hex::encode(ctx.manager_id.into_bytes())),
                "route":format!("0x{}",hex::encode(SubmitReceipt::ROUTE)),"proxySlot":key.0,
                "expectedRejection":probe.rejection(),"preSendFinalizedHeight":before.gear_height,
                "preSendFinalizedHash":format!("{:#x}",before.gear_hash),
                "batchDeadlineMs":journal.windows["preflight-normal"]["deadlineMs"]});
            let proxy_payload = Redirect::encode_call(key.0, payload, ctx.manager_id, SubmitReceipt::ROUTE.to_vec());
            intent["proxyPayload"] = json!(format!("0x{}",hex::encode(&proxy_payload)));
            intent["proxyPayloadKeccak256"] = json!(format!("0x{}",hex::encode(keccak256(&proxy_payload))));
            let should_send=prepare_receipt_probe(journal,path,probe,intent.clone(),&before)?;
            if ctx.schedule.handovers==2 {
                if journal.actions[probe.action_id()].evidence["submission"]["messageId"].is_null() {
                    let dispatch=signed_normal_message(ctx, (&ctx.gear_api, ctx.campaign_actor), (journal, path), (&format!("{}/signed-dispatch",probe.action_id()), intent), (ctx.historical_proxy_id, proxy_payload, 0), deadline).await?;
                    record_probe_submission(journal,path,probe,GearHash::from_str(string(&dispatch["messageId"],"original probe message")?)?,GearHash::from_str(string(&dispatch["enqueueBlockHash"],"original probe block")?)?)?;
                }
            }else if should_send {
                let gas = ctx.gear_api.block_gas_limit()? / 100 * 95;
                match ctx.gear_api.send_message_bytes(ctx.historical_proxy_id, &proxy_payload, gas, 0).await {
                    Ok((message_id, enqueue_hash)) => record_probe_submission(journal, path, probe,
                        GearHash::from_slice(message_id.as_ref()), enqueue_hash)?,
                    Err(error) => {
                        journal.actions.get_mut(probe.action_id()).expect("probe exists").status = "ambiguous".into();
                        set_action_evidence(journal, probe.action_id(), "diagnostic", json!(format!("ambiguous Gear send; never resend: {error:?}")));
                        save_journal(path, journal)?;
                        bail!("receipt probe send is ambiguous; HOLD without resending: {error:?}");
                    }
                }
            }
            let action = &journal.actions[probe.action_id()];
            ensure!(matches!(action.status.as_str(), "submitted" | "finalized"), "receipt probe handoff is unresolved; HOLD without resending");
            if action.evidence["submission"]["enqueueHeight"].is_null() {
                let hash: GearHash = string(&action.evidence["submission"]["enqueueBlockHash"], "probe enqueue hash")?.parse()?;
                let height = ctx.source.api.block_hash_to_number(hash).await?;
                journal.actions.get_mut(probe.action_id()).expect("probe exists").evidence["submission"]["enqueueHeight"] = json!(height);
                save_journal(path, journal)?;
            }
            let reply = find_receipt_probe_reply(ctx, &journal.actions[probe.action_id()], deadline).await?;
            let after = match journal.actions[probe.action_id()].evidence.get("after") {
                Some(saved) => ctx.snapshot_at(Some(&SnapshotSet::from_json(saved)?)).await?,
                None => ctx.snapshot().await?,
            };
            let status = VftManager::new(ctx.remoting.clone()).receipt_status(key.0, key.1)
                .at_block(sails_block_hash(after.gear_hash)).recv(ctx.manager_id).await?;
            finalize_receipt_probe(journal, path, probe, reply, &before, &after, &ctx.tokens, status, deadline)?;
        }
        ensure!(receipt_probes_completed(journal)?, "both live receipt probes are required; HOLD");
        Ok(())
    }).await.map_err(|_| anyhow!("original normal-batch deadline expired during receipt probes; HOLD without resending"))?
}

async fn revalidate_receipt_probes(ctx: &Context, journal: &Journal) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(120);
    timeout_at(deadline.into(), async {
        let original = original_probe_event(ctx, journal).await?;
        for probe in RECEIPT_PROBES {
            let Some(action) = journal
                .actions
                .get(probe.action_id())
                .filter(|action| action.status == "finalized")
            else {
                continue;
            };
            let before = SnapshotSet::from_json(&action.evidence["before"])?;
            let after = SnapshotSet::from_json(&action.evidence["after"])?;
            ctx.snapshot_at(Some(&before)).await?;
            ctx.snapshot_at(Some(&after)).await?;
            verify_roundtrip_delta(&before, &after, &ctx.tokens)?;
            let reply = find_receipt_probe_reply(ctx, action, deadline).await?;
            let raw = hex::decode(
                string(&reply["rawReply"], "historical raw probe reply")?.trim_start_matches("0x"),
            )?;
            probe_rejection(probe, &raw, &original.receipt_rlp)?;
            let status = VftManager::new(ctx.remoting.clone())
                .receipt_status(original.proof_block.block.slot, original.transaction_index)
                .at_block(sails_block_hash(after.gear_hash))
                .recv(ctx.manager_id)
                .await?;
            ensure!(
                matches!(status, GearReceiptStatus::Processed),
                "historical original receipt is no longer Processed; HOLD"
            );
        }
        Ok(())
    })
    .await
    .map_err(|_| anyhow!("historical live receipt probe revalidation timed out; HOLD"))?
}

async fn canonical_inbound_call_reply<'a>(
    ctx: &Context,
    signed: &'a relayer::message_relayer::eth_to_gear::message_sender::SignedSubmission,
    target: ActorId,
    payload: &[u8],
) -> Result<Option<&'a [u8]>> {
    use sp_core::crypto::Ss58Codec;
    ensure!(
        bytes32(&signed.chain_genesis_hash)? == ctx.source.source_genesis
            && actor_id(&signed.manager)? == ctx.manager_id
            && actor_id(&signed.historical_proxy)? == ctx.historical_proxy_id
            && signed.finalized_dispatch_error.is_none()
            && sp_core::blake2_256(&signed.raw_extrinsic) == bytes32(&signed.extrinsic_hash)?,
        "original inbound source/profile/signed hash changed; HOLD"
    );
    let Some((height, hash)) = signed
        .inclusion_block_number
        .zip(signed.inclusion_block_hash.as_ref())
    else {
        return Ok(None);
    };
    let Some(reply) = signed.finalized_reply.as_ref() else {
        return Ok(None);
    };
    let Some(reply_height) = reply.observation.finalized_block_number else {
        return Ok(None);
    };
    let source_finalized = ctx
        .source
        .api
        .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
        .await?;
    let witness_finalized = ctx
        .witness
        .api
        .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
        .await?;
    if reply_height > source_finalized.min(witness_finalized) {
        return Ok(None);
    }
    ensure!(
        signed.prepared_finalized_number <= height
            && ctx
                .source
                .api
                .block_number_to_hash(signed.prepared_finalized_number)
                .await?
                == GearHash::from_str(&signed.prepared_finalized_hash)?
            && ctx
                .witness
                .api
                .block_number_to_hash(signed.prepared_finalized_number)
                .await?
                == GearHash::from_str(&signed.prepared_finalized_hash)?,
        "original inbound preparation pin is not common canonical ancestry; HOLD"
    );
    let hash = GearHash::from_str(hash)?;
    let reply_hash = GearHash::from_str(&reply.observation.finalized_block_hash)?;
    ensure!(
        reply_height >= height
            && ctx.source.api.block_number_to_hash(height).await? == hash
            && ctx.witness.api.block_number_to_hash(height).await? == hash
            && ctx.source.api.block_number_to_hash(reply_height).await? == reply_hash
            && ctx.witness.api.block_number_to_hash(reply_height).await? == reply_hash,
        "original inbound inclusion/reply are not common canonical finalized ancestry; HOLD"
    );
    let sender = if signed.sender.starts_with("0x") {
        actor_id(&signed.sender)?
    } else {
        let account = sp_core::crypto::AccountId32::from_ss58check(&signed.sender)
            .map_err(|error| anyhow!("original inbound account: {error:?}"))?;
        ActorId::from(<[u8; 32]>::from(account))
    };
    let message_id = GearHash::from_str(
        signed
            .message_id
            .as_deref()
            .context("original inbound message identity absent")?,
    )?;
    let mut original = false;
    for included in ctx
        .source
        .api
        .get_block_at(hash)
        .await?
        .extrinsics()
        .await?
        .iter()
    {
        if included.hash().0 != bytes32(&signed.extrinsic_hash)? {
            continue;
        }
        let mut address = [0; 33];
        address[1..].copy_from_slice(&sender.into_bytes());
        ensure!(
            included.bytes() == signed.raw_extrinsic
                && included.address_bytes() == Some(address.as_slice())
                && included
                    .transaction_extensions()
                    .and_then(|extensions| extensions.nonce())
                    == Some(signed.nonce)
                && included.pallet_name()? == "Gear"
                && included.variant_name()? == "send_message",
            "original inbound signed call/signer/nonce changed; HOLD"
        );
        let mut args = included
            .call_bytes()
            .get(2..)
            .context("original inbound call framing missing")?;
        let destination = <[u8; 32]>::decode(&mut args)?;
        let length = usize::try_from(parity_scale_codec::Compact::<u32>::decode(&mut args)?.0)?;
        ensure!(
            length == payload.len(),
            "original source proof payload length changed"
        );
        let (body, tail) = args
            .split_at_checked(length)
            .context("original source payload is truncated")?;
        args = tail;
        let (gas, value, keep_alive) = <(u64, u128, bool)>::decode(&mut args)?;
        ensure!(
            args.is_empty()
                && destination == target.into_bytes()
                && body == payload
                && gas > 0
                && value == 0
                && !keep_alive,
            "original inbound signed payload/value/consumer changed; HOLD"
        );
        let mut success = false;
        let mut enqueued = false;
        for event in included.events().await?.iter() {
            let event = event?;
            ensure!(
                event.pallet_name() != "System" || event.variant_name() != "ExtrinsicFailed",
                "original inbound runtime dispatch failed; HOLD"
            );
            success |=
                event.pallet_name() == "System" && event.variant_name() == "ExtrinsicSuccess";
            if let RuntimeEvent::Gear(GearEvent::MessageQueued {
                id,
                source,
                destination,
                ..
            }) = event.as_gear()?
            {
                if GearHash::from_slice(id.as_ref()) == message_id {
                    ensure!(
                        source.0 == sender.into_bytes() && destination == target,
                        "original inbound queued consumer changed; HOLD"
                    );
                    enqueued = true;
                }
            }
        }
        ensure!(
            success && enqueued && !original,
            "original inbound dispatch lacks unique finalized enqueue; HOLD"
        );
        original = true;
    }
    ensure!(
        original,
        "original inbound signed source bytes missing from recorded block; HOLD"
    );
    let mut found = false;
    for event in ctx
        .source
        .api
        .get_block_at(reply_hash)
        .await?
        .events()
        .await?
        .iter()
    {
        let RuntimeEvent::Gear(GearEvent::UserMessageSent { message, .. }) = event?.as_gear()?
        else {
            continue;
        };
        let Some(details) = message.details() else {
            continue;
        };
        if GearHash::from_slice(details.to_message_id().as_ref()) != message_id {
            continue;
        }
        ensure!(
            !found
                && message.source() == target
                && message.destination() == sender
                && details.to_reply_code().is_success()
                && message.payload_bytes() == reply.payload,
            "original inbound finalized reply bytes/source/linkage changed; HOLD"
        );
        found = true;
    }
    ensure!(
        found,
        "original inbound finalized reply absent from recorded block; HOLD"
    );
    Ok(Some(reply.payload.as_slice()))
}

async fn normal_inbound_reply(
    ctx: &Context,
    worker: Option<&InboundTransaction>,
    event: &EthToVaraEvent,
    native: bool,
) -> Result<(bool, bool)> {
    let Some(receipt) = worker.and_then(|tx| tx.receipt.as_ref()) else {
        return Ok((false, false));
    };
    let Some(signed) = receipt.signed_submission.as_ref() else {
        return Ok((false, false));
    };
    let key = (event.proof_block.block.slot, event.transaction_index);
    let original_event = decode_probe_event(&receipt.payload)?;
    ensure!(
        receipt.receipt_key == key
            && signed.receipt_key == key
            && signed.payload_hash == format!("0x{}", hex::encode(keccak256(&receipt.payload)))
            && original_event.receipt_rlp == event.receipt_rlp
            && (
                original_event.proof_block.block.slot,
                original_event.transaction_index
            ) == key,
        "original worker proof/receipt identity changed; HOLD"
    );
    let mut payload = Redirect::ROUTE.to_vec();
    (
        key.0,
        receipt.payload.as_slice(),
        ctx.manager_id,
        SubmitReceipt::ROUTE,
    )
        .encode_to(&mut payload);
    let Some(reply) =
        canonical_inbound_call_reply(ctx, signed, ctx.historical_proxy_id, &payload).await?
    else {
        return Ok((false, false));
    };
    let outer = decode_campaign_reply::<Result<(Vec<u8>, Vec<u8>), historical_proxy_client::Error>>(
        reply,
        Redirect::ROUTE,
    )?;
    let (returned, inner) =
        outer.map_err(|error| anyhow!("original historical proxy failed: {error:?}; HOLD"))?;
    ensure!(
        returned == event.receipt_rlp,
        "original proxy returned another receipt; HOLD"
    );
    let result = decode_campaign_reply::<Result<(), vft_manager_client::Error>>(
        &inner,
        SubmitReceipt::ROUTE,
    )?;
    match result {
        Ok(()) => Ok((true, true)),
        Err(vft_manager_client::Error::NativeSettlementPending) if native => {
            let Some(continuation) = signed.native_reconciliation.as_ref() else {
                return Ok((true, false));
            };
            ensure!(
                continuation.sender == signed.sender
                    && continuation.receipt_key == key
                    && continuation.nonce > signed.nonce
                    && continuation.prepared_finalized_number
                        >= signed
                            .finalized_reply
                            .as_ref()
                            .context("original native reply missing")?
                            .observation
                            .finalized_block_number
                            .context("original native reply pin missing")?,
                "native continuation changed original signer/key/nonce/order; HOLD"
            );
            let payload =
                vft_manager_client::vft_manager::io::ReconcileReceipt::encode_call(key.0, key.1);
            ensure!(
                continuation.payload_hash == format!("0x{}", hex::encode(keccak256(&payload)))
                    && continuation.native_reconciliation.is_none(),
                "native reconciliation payload changed or recursively replaced; HOLD"
            );
            let Some(reply) =
                canonical_inbound_call_reply(ctx, continuation, ctx.manager_id, &payload).await?
            else {
                return Ok((true, false));
            };
            ensure!(
                matches!(
                    decode_campaign_reply::<Result<GearReceiptStatus, vft_manager_client::Error>>(
                        reply,
                        vft_manager_client::vft_manager::io::ReconcileReceipt::ROUTE
                    )?,
                    Ok(GearReceiptStatus::Processed)
                ),
                "original non-economic reconciliation did not finalize all original deposits; HOLD"
            );
            Ok((true, true))
        }
        Err(error) => bail!("original inbound consumer rejected receipt: {error:?}; HOLD"),
    }
}

async fn reconcile_normal_receipt(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    id: &str,
    (slot, index, hash): (u64, u64, GearHash),
    worker: Option<&InboundTransaction>,
    deadline: Instant,
) -> Result<bool> {
    let token = ctx
        .tokens
        .iter()
        .find(|token| journal.actions[id].intent["token"] == format!("{:#x}", token.address))
        .context("normal receipt token not in immutable registry")?;
    let amount = GearU256::from_dec_str(string(
        &journal.actions[id].intent["amountRaw"],
        "original inbound amount",
    )?)?;
    let rows = VftManager::new(ctx.remoting.clone())
        .receipt_deposits(slot, index)
        .at_block(sails_block_hash(hash))
        .recv(ctx.manager_id)
        .await?;
    if rows.is_empty() {
        return Ok(false);
    }
    ensure!(
        rows.len() == 1,
        "campaign single-lock receipt has unexpected deposit rows; HOLD"
    );
    let row = &rows[0];
    ensure!(row.sender==H160::from_slice(ctx.campaign_address.as_slice()) && row.receiver==ctx.campaign_actor && row.token_id==token.peer
        && row.eth_token_id==H160::from_slice(token.address.as_slice()) && row.amount==amount && row.supply==if token.gear_origin{TokenSupply::Gear}else{TokenSupply::Ethereum}
        && row.native==token.native_amount.is_some(),"finalized deposit does not match original signed token/recipient/amount/settlement policy; HOLD");
    ensure!(
        !matches!(
            row.outcome,
            vft_manager_client::ReceiptDepositOutcome::Rejected
                | vft_manager_client::ReceiptDepositOutcome::Unknown
        ),
        "original economic child rejected or ambiguous; HOLD"
    );
    let payload = hex::decode(
        string(
            &journal.actions[id].evidence["incomingReceipt"]["scalePayload"],
            "original native receipt proof",
        )?
        .trim_start_matches("0x"),
    )?;
    let event = decode_probe_event(&payload)?;
    let (original_verified, reply_completed) =
        normal_inbound_reply(ctx, worker, &event, row.native).await?;
    if !original_verified {
        return Ok(false);
    }
    if !row.native {
        return Ok(reply_completed
            && matches!(
                row.outcome,
                vft_manager_client::ReceiptDepositOutcome::Settled
            ));
    }
    let operation = H256(keccak256(
        &(
            b"vara/native-escrow/v1",
            ctx.manager_id,
            ctx.historical_proxy_id,
            H160::from_slice(ctx.ethereum.receiver_address.as_slice()),
            slot,
            index,
            row.log_index,
            keccak256(&event.receipt_rlp),
        )
            .encode(),
    ));
    ensure!(
        row.operation_id == operation,
        "native operation is not bound to original receipt/domain/log; HOLD"
    );
    let wrapper = vft_vara_client::NativeEscrow::new(ctx.remoting.clone());
    let Some(redemption) = wrapper
        .redemption(operation)
        .at_block(sails_block_hash(hash))
        .recv(token.peer)
        .await?
    else {
        return Ok(false);
    };
    ensure!(
        redemption.from == ctx.manager_id
            && redemption.to == ctx.campaign_actor
            && redemption.amount == amount
            && redemption.returned_value == 0,
        "original native payout identity/amount/returned value changed; HOLD"
    );
    ensure!(
        !matches!(
            redemption.status,
            vft_vara_client::PayoutStatus::Returned | vft_vara_client::PayoutStatus::Ambiguous
        ),
        "original native payout returned or ambiguous; HOLD without economic retry"
    );
    let payout = journal.actions[id].evidence.get("nativePayout").cloned();
    let payout = if let Some(saved) = payout {
        ensure!(
            saved["child"] == format!("0x{}", hex::encode(redemption.child.into_bytes()))
                && saved["operationId"] == format!("{operation:#x}")
                && saved["amountRaw"] == amount.to_string(),
            "original native child changed; HOLD"
        );
        let height = u32::try_from(number(&saved["finalizedHeight"])?)?;
        let pin = GearHash::from_str(string(
            &saved["finalizedHash"],
            "original native payout pin",
        )?)?;
        ensure!(
            ctx.source.api.block_number_to_hash(height).await? == pin
                && ctx.witness.api.block_number_to_hash(height).await? == pin,
            "original native payout no longer canonical"
        );
        saved
    } else {
        let submission = worker
            .and_then(|tx| tx.receipt.as_ref())
            .and_then(|receipt| receipt.signed_submission.as_ref())
            .context("original native signed consumer dispatch missing; HOLD")?;
        let start = submission
            .inclusion_block_number
            .context("original native consumer inclusion missing; HOLD")?;
        let head = ctx.source.api.block_hash_to_number(hash).await?;
        let mut found = None;
        for height in start..=head {
            let pin = ctx.source.api.block_number_to_hash(height).await?;
            ensure!(
                ctx.witness.api.block_number_to_hash(height).await? == pin,
                "native payout witnesses differ"
            );
            for event in ctx
                .source
                .api
                .get_block_at(pin)
                .await?
                .events()
                .await?
                .iter()
            {
                let RuntimeEvent::Gear(GearEvent::UserMessageSent { message, .. }) =
                    event?.as_gear()?
                else {
                    continue;
                };
                if message.id() != redemption.child {
                    continue;
                }
                ensure!(
                    message.source() == token.peer
                        && message.destination() == ctx.campaign_actor
                        && GearU256::from(message.value()) == amount
                        && message.details().is_none(),
                    "original native payout has wrong sender/recipient/value; HOLD"
                );
                ensure!(found.is_none(), "duplicate original native payout; HOLD");
                found = Some(
                    json!({"child":format!("0x{}",hex::encode(redemption.child.into_bytes())),"operationId":format!("{operation:#x}"),"amountRaw":amount.to_string(),"finalizedHeight":height,"finalizedHash":format!("{pin:#x}")}),
                );
            }
        }
        let found =
            found.context("native queued delivery has no canonical original value event; HOLD")?;
        set_action_evidence(journal, id, "nativePayout", found.clone());
        save_journal(path, journal)?;
        found
    };
    let claim_id = format!("{id}/native-claim");
    let claim_intent = json!({"kind":"claim-original-native-value","operationId":format!("{operation:#x}"),"originalChild":payout["child"],"amountRaw":amount.to_string(),"nativeWrapper":format!("0x{}",hex::encode(token.peer.into_bytes()))});
    if !journal.actions.contains_key(&claim_id) {
        ensure!(
            matches!(redemption.status, vft_vara_client::PayoutStatus::Queued),
            "native value delivered without this campaign's original claim evidence; HOLD"
        );
        let (message, _) = ctx
            .gear_api
            .get_mailbox_message(redemption.child)
            .await?
            .context("original native payout absent from mailbox; HOLD without substitute claim")?;
        ensure!(
            message.source() == token.peer
                && message.destination() == ctx.campaign_actor
                && GearU256::from(message.value()) == amount,
            "original native mailbox identity differs"
        );
    }
    let claim = signed_normal_gear_call(
        ctx,
        (&ctx.gear_api, ctx.campaign_actor),
        (journal, path),
        (&claim_id, claim_intent),
        gsdk::gear::tx().gear().claim_value(redemption.child),
        None,
        deadline,
    )
    .await?;
    ensure!(
        claim["valueRead"] == true,
        "original native claim did not canonically read value; HOLD"
    );
    if !matches!(redemption.status, vft_vara_client::PayoutStatus::Delivered) {
        return Ok(false);
    }
    Ok(reply_completed
        && matches!(
            row.outcome,
            vft_manager_client::ReceiptDepositOutcome::Settled
        ))
}

async fn wait_inbound_processed(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    incoming: &[(String, u64, u64)],
    deadline: Instant,
) -> Result<()> {
    let manager = VftManager::new(ctx.remoting.clone());
    loop {
        ensure!(
            Instant::now() < deadline,
            "campaign deadline expired while awaiting finalized Gear receipt status"
        );
        let height = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?
            .min(
                ctx.witness
                    .api
                    .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
                    .await?,
            );
        let hash = ctx.source.api.block_number_to_hash(height).await?;
        ensure!(
            ctx.witness.api.block_number_to_hash(height).await? == hash,
            "inbound receipt pin differs from independent witness; HOLD"
        );
        let mut complete = true;
        let worker = ctx
            .inbound_journal()?
            .context("inbound worker journal disappeared")?;
        for (action_id, slot, index) in incoming {
            let expected: B256 = journal.actions[action_id]
                .tx_hash
                .as_deref()
                .context("inbound action has no lock transaction")?
                .parse()?;
            let mut matches = worker
                .transactions
                .values()
                .chain(worker.completed.values())
                .filter(|tx| tx.tx.tx_hash.as_slice() == expected.as_slice());
            let handed_off = match matches.next() {
                Some(tx) => {
                    ensure!(
                        matches.next().is_none(),
                        "multiple worker receipts refer to one EVM lock"
                    );
                    record_worker_receipt(journal, action_id, tx, (*slot, *index))?
                }
                None => false,
            };
            if ctx.schedule.handovers == 2 {
                let original = worker
                    .transactions
                    .values()
                    .chain(worker.completed.values())
                    .find(|tx| tx.tx.tx_hash.as_slice() == expected.as_slice());
                complete &= reconcile_normal_receipt(
                    ctx,
                    journal,
                    path,
                    action_id,
                    (*slot, *index, hash),
                    original,
                    deadline,
                )
                .await?;
            }
            complete &= handed_off;
            let status = manager
                .receipt_status(*slot, *index)
                .at_block(sails_block_hash(hash))
                .recv(ctx.manager_id)
                .await;
            let saved = &journal.actions[action_id].evidence["receiptStatus"];
            if saved["status"] == "Processed" {
                ensure!(
                    matches!(&status, Ok(GearReceiptStatus::Processed)),
                    "previously processed Gear receipt is no longer confirmed; HOLD"
                );
                let saved_height = u32::try_from(number(&saved["finalizedBlock"])?)?;
                let saved_hash: GearHash =
                    string(&saved["finalizedHash"], "original Gear receipt status hash")?
                        .parse()?;
                ensure!(
                    saved_height <= height
                        && ctx.source.api.block_number_to_hash(saved_height).await? == saved_hash
                        && ctx.witness.api.block_number_to_hash(saved_height).await? == saved_hash,
                    "original finalized Gear receipt status changed; HOLD"
                );
                journal
                    .actions
                    .get_mut(action_id)
                    .expect("inbound action exists")
                    .status = "processed".into();
                continue;
            }
            match status {
                Ok(GearReceiptStatus::Processed) => {
                    set_action_evidence(
                        journal,
                        action_id,
                        "receiptStatus",
                        json!({"status":"Processed","finalizedBlock":height,"finalizedHash":format!("{hash:#x}"),"slot":slot,"transactionIndex":index}),
                    );
                    record_action_milestone(
                        journal,
                        action_id,
                        "gearFinalizedStatusObservedAtMs",
                        now_ms()?,
                    )?;
                    journal
                        .actions
                        .get_mut(action_id)
                        .expect("inbound action exists")
                        .status = "processed".into();
                }
                Ok(GearReceiptStatus::Reserved) => {
                    complete = false;
                    set_action_evidence(
                        journal,
                        action_id,
                        "receiptStatus",
                        json!({"status":"Reserved","finalizedBlock":height,"finalizedHash":format!("{hash:#x}"),"slot":slot,"transactionIndex":index}),
                    );
                }
                Ok(GearReceiptStatus::Unknown) => {
                    complete = false;
                    set_action_evidence(
                        journal,
                        action_id,
                        "receiptStatus",
                        json!({"status":"Unknown","finalizedBlock":height,"finalizedHash":format!("{hash:#x}"),"slot":slot,"transactionIndex":index,"resend":false}),
                    );
                }
                Err(error) => {
                    complete = false;
                    set_action_evidence(
                        journal,
                        action_id,
                        "receiptStatus",
                        json!({"status":"query_error","finalizedBlock":height,"finalizedHash":format!("{hash:#x}"),"error":format!("{error:?}"),"resend":false}),
                    );
                }
            }
        }
        save_journal(path, journal)?;
        if complete {
            return Ok(());
        }
        sleep(Duration::from_secs(6)).await;
    }
}

async fn reconcile_outbound_requests(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    request_actions: &[(String, String, u32)],
    request_start: u32,
    amount_gear: GearU256,
    deadline: Instant,
) -> Result<Vec<(String, Token, OutboundRequest)>> {
    timeout_at(deadline.into(), async {
        let outbound_events = loop {
            ensure!(
                Instant::now() < deadline,
                "absolute window deadline expired reconciling Gear outbound requests"
            );
            let latest = ctx
                .source
                .api
                .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
                .await?;
            if latest >= request_start {
                let events = scan_outbound_events(ctx, request_start, latest).await?;
                let mut complete = true;
                for (_, symbol, start) in request_actions {
                    let token = ctx
                        .tokens
                        .iter()
                        .find(|token| token.symbol == symbol)
                        .context("outbound token missing")?;
                    let matches = events
                        .iter()
                        .flat_map(|block| block.manager_requests.iter())
                        .filter(|event| {
                            event.sender == ctx.campaign_actor
                                && event.token == token.peer
                                && event.receiver == H160::from_slice(ctx.campaign_address.as_slice())
                                && event.amount == token.gear_amount(amount_gear).expect("validated native campaign amount")
                                && event.block >= *start
                        })
                        .count();
                    ensure!(
                        matches <= 1,
                        "multiple finalized Gear requests match {symbol}'s saved intent"
                    );
                    complete &= matches == 1;
                }
                if complete {
                    break events;
                }
            }
            sleep(Duration::from_secs(3)).await;
        };
        let mut outbound = Vec::new();
        for (id, symbol, start) in request_actions {
            let token = ctx
                .tokens
                .iter()
                .find(|token| token.symbol == *symbol)
                .context("outbound token missing")?;
            let mut candidates = outbound_events
                .iter()
                .flat_map(|block| block.manager_requests.iter().cloned())
                .filter(|event| {
                    event.sender == ctx.campaign_actor
                        && event.token == token.peer
                        && event.receiver == H160::from_slice(ctx.campaign_address.as_slice())
                        && event.amount == token.gear_amount(amount_gear).expect("validated native campaign amount")
                        && event.block >= *start
                });
            let event = candidates.next().context(format!(
                "{} BridgingRequested event is missing",
                token.symbol
            ))?;
            ensure!(
                candidates.next().is_none(),
                "multiple finalized Gear requests match {}'s saved intent",
                token.symbol
            );
            let returned =
                VftManagerEvents::decode_event(&event.payload).context("decode manager event")?;
            let (event_nonce, event_queue_id, event_hash) = match returned {
                VftManagerEvents::BridgingRequested {
                    nonce,
                    queue_id,
                    hash,
                    vara_token_id,
                    sender,
                    amount: returned_amount,
                    receiver,
                } => {
                    ensure!(
                        vara_token_id == token.peer
                            && sender == ctx.campaign_actor
                            && returned_amount == token.gear_amount(amount_gear)?
                            && receiver == H160::from_slice(ctx.campaign_address.as_slice()),
                        "Gear manager event does not match saved outbound intent"
                    );
                    (nonce, queue_id, hash.0)
                }
                _ => bail!("unexpected VFT manager event for outbound request"),
            };
            ensure!(
                event_nonce == event.nonce
                    && event_queue_id == event.queue_id
                    && event_hash == event.message_hash,
                "Gear manager event differs from the built-in queued message"
            );
            let queued = outbound_events
                .iter()
                .flat_map(|block| block.queued.iter())
                .find(|message| {
                    message.nonce == event.nonce && message.message_hash == event.message_hash
                })
                .cloned()
                .context(format!(
                    "{} Gear MessageQueued event is missing",
                    token.symbol
                ))?;
            let expected_receiver = H160::from_slice(ctx.campaign_address.as_slice());
            ensure!(
                event.receiver == expected_receiver,
                "Gear manager request event differs from the saved campaign recipient"
            );
            let nonce = event.nonce.to_string();
            let evidence = json!({
                    "nonce":nonce,"queueId":event.queue_id,"messageHash":format!("0x{}",hex::encode(event.message_hash)),
                    "managerEventBlock":event.block,"managerEventBlockHash":format!("{:#x}",event.block_hash),
                    "managerEventPayload":format!("0x{}",hex::encode(&event.payload)),"amountRaw":event.amount.to_string(),
                    "receiver":format!("{:#x}",event.receiver),"queueBlock":queued.block,"queueBlockHash":format!("{:#x}",queued.block_hash),
                    "source":format!("0x{}",hex::encode(queued.source)),"destination":format!("0x{}",hex::encode(queued.destination)),
                    "payloadHash":format!("0x{}",hex::encode(queued.message_hash)),"payload":format!("0x{}",hex::encode(&queued.payload)),
                });
            if let Some(saved) = journal.actions[id].evidence.get("outboundRequest") {
                ensure!(saved == &evidence, "replayed Gear request changed its original source, anchor, nonce, payload or inclusion; HOLD");
            }
            if journal.actions[id].status != "released" {
                journal.actions.get_mut(id).expect("Gear request action exists").status = "reconciled".into();
            }
            set_action_evidence(journal, id, "outboundRequest", evidence);
            record_action_milestone(journal, id, "burnFinalizedObservedAtMs", now_ms()?)?;
            save_journal(path, journal)?;
            outbound.push((
                id.clone(),
                token.clone(),
                OutboundRequest {
                    nonce: event.nonce,
                    queue_id: event.queue_id,
                    message_hash: event.message_hash,
                    queue_block: queued.block,
                    queue_block_hash: queued.block_hash,
                },
            ));
        }
        Ok(outbound)
    }).await.map_err(|_| anyhow!("original window deadline expired reconciling outbound requests"))?
}

async fn scan_outbound_events(ctx: &Context, from: u32, to: u32) -> Result<Vec<ScannedGearBlock>> {
    let mut blocks = Vec::new();
    for number in from..=to {
        let hash = ctx.source.api.block_number_to_hash(number).await?;
        ensure!(
            ctx.witness.api.block_number_to_hash(number).await? == hash,
            "outbound event source/witness block hashes differ; HOLD"
        );
        let events = ctx.gear_api.events_at(hash).await?;
        let mut manager_requests = Vec::new();
        let mut queued = Vec::new();
        for event in &events {
            match event {
                gsdk::Event::Gear(gsdk::gear::gear::Event::UserMessageSent { message, .. })
                    if message.source().into_bytes() == ctx.manager_id.into_bytes()
                        && message.destination().into_bytes() == H256::zero().0 =>
                {
                    if let Ok(VftManagerEvents::BridgingRequested {
                        nonce,
                        queue_id,
                        hash: message_hash,
                        vara_token_id,
                        sender,
                        amount,
                        receiver,
                    }) = VftManagerEvents::decode_event(message.payload_bytes())
                    {
                        manager_requests.push(RequestEvent {
                            nonce,
                            queue_id,
                            message_hash: message_hash.0,
                            token: vara_token_id,
                            sender,
                            amount,
                            receiver,
                            block: number,
                            block_hash: hash,
                            payload: message.payload_bytes().to_vec(),
                        });
                    }
                }
                gsdk::Event::GearEthBridge(gsdk::gear::gear_eth_bridge::Event::MessageQueued {
                    message,
                    ..
                }) => {
                    let nonce = GearU256(message.nonce.0);
                    let mut nonce_be = [0u8; 32];
                    nonce.to_big_endian(&mut nonce_be);
                    let queued_message = gear_rpc_client::dto::Message {
                        nonce_be,
                        source: message.source.0,
                        destination: message.destination.0,
                        payload: message.payload.clone(),
                    };
                    let source = queued_message.source;
                    let destination = queued_message.destination.to_vec();
                    let message_hash =
                        relayer::message_relayer::common::message_hash(&queued_message);
                    queued.push(QueuedEvent {
                        nonce,
                        message_hash,
                        block: number,
                        block_hash: hash,
                        source,
                        destination,
                        payload: queued_message.payload,
                    });
                }
                _ => {}
            }
        }
        blocks.push(ScannedGearBlock {
            manager_requests,
            queued,
        });
    }
    Ok(blocks)
}

#[derive(Clone, Debug)]
struct RequestEvent {
    nonce: GearU256,
    queue_id: u64,
    message_hash: [u8; 32],
    token: ActorId,
    sender: ActorId,
    amount: GearU256,
    receiver: H160,
    block: u32,
    block_hash: GearHash,
    payload: Vec<u8>,
}
#[derive(Clone, Debug)]
struct QueuedEvent {
    nonce: GearU256,
    message_hash: [u8; 32],
    block: u32,
    block_hash: GearHash,
    source: [u8; 32],
    destination: Vec<u8>,
    payload: Vec<u8>,
}
#[derive(Clone, Debug)]
struct ScannedGearBlock {
    manager_requests: Vec<RequestEvent>,
    queued: Vec<QueuedEvent>,
}

async fn scan_paid_event(
    ctx: &Context,
    from: u32,
    request: &OutboundRequest,
    mode: FeeMode,
    deadline: Instant,
) -> Result<Value> {
    loop {
        ensure!(
            Instant::now() < deadline,
            "campaign deadline expired awaiting finalized paid event for nonce {}",
            request.nonce
        );
        let to = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?;
        if to >= from {
            for number in from..=to {
                let hash = ctx.source.api.block_number_to_hash(number).await?;
                let events = ctx.gear_api.events_at(hash).await?;
                for event in &events {
                    if let gsdk::Event::Gear(gsdk::gear::gear::Event::UserMessageSent {
                        message,
                        ..
                    }) = event
                    {
                        if message.source().into_bytes() != ctx.payment_id.into_bytes()
                            || message.destination().into_bytes() != H256::zero().0
                        {
                            continue;
                        }
                        match (
                            mode,
                            BridgingPaymentEvents::decode_event(message.payload_bytes()),
                        ) {
                            (
                                FeeMode::Normal,
                                Ok(BridgingPaymentEvents::BridgingPaid { nonce }),
                            ) if nonce == request.nonce => {
                                return Ok(
                                    json!({"type":"normal","nonce":nonce.to_string(),"block":number,"blockHash":format!("{hash:#x}")}),
                                )
                            }
                            (
                                FeeMode::Priority,
                                Ok(BridgingPaymentEvents::PriorityBridgingPaid { block, nonce }),
                            ) if nonce == request.nonce
                                && block.0 == request.queue_block_hash.0 =>
                            {
                                return Ok(
                                    json!({"type":"priority","nonce":nonce.to_string(),"queuedBlockHash":format!("0x{}",hex::encode(block.0)),"block":number,"blockHash":format!("{hash:#x}")}),
                                )
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        sleep(Duration::from_secs(3)).await;
    }
}

async fn find_covering_root(
    ctx: &Context,
    request: &OutboundRequest,
    deadline: Instant,
) -> Result<QueueRoot> {
    loop {
        ensure!(
            Instant::now() < deadline,
            "campaign deadline expired waiting for finalized source queue root"
        );
        let source_height = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?;
        let witness_height = ctx
            .witness
            .api
            .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
            .await?;
        if source_height < request.queue_block || witness_height < request.queue_block {
            sleep(Duration::from_secs(3)).await;
            continue;
        }
        let block_hash = request.queue_block_hash;
        ensure!(
            ctx.source
                .api
                .block_number_to_hash(request.queue_block)
                .await?
                == block_hash
                && ctx
                    .witness
                    .api
                    .block_number_to_hash(request.queue_block)
                    .await?
                    == block_hash,
            "source or witness finalized a different outbound message block hash"
        );
        ensure!(
            source::has_event(
                &ctx.source.api,
                block_hash,
                "GearEthBridge",
                "QueueMerkleRootChanged"
            )
            .await?
                && source::has_event(
                    &ctx.witness.api,
                    block_hash,
                    "GearEthBridge",
                    "QueueMerkleRootChanged"
                )
                .await?,
            "outbound message block has no witnessed QueueMerkleRootChanged event"
        );
        let (queue_id, root) = ctx.source.api.fetch_queue_merkle_root(block_hash).await?;
        ensure!(
            queue_id >= request.queue_id && root.0 != [0; 32],
            "message-block root does not cover its request"
        );
        let witness_root = ctx.witness.api.fetch_queue_merkle_root(block_hash).await?;
        ensure!(
            (queue_id, root) == witness_root,
            "source/witness queue storage differs at the message-block registration"
        );
        return Ok(QueueRoot {
            source_block: request.queue_block,
            block_hash,
            queue_id,
            root: root.0,
        });
    }
}

fn actor_root_accepted(status: &Value) -> Result<bool> {
    match status.as_str() {
        Some("pending" | "mined") => Ok(false),
        Some("accepted") => Ok(true),
        _ => bail!("actor root registration has an unsupported status"),
    }
}
async fn wait_actor_root(ctx: &Context, root: &QueueRoot, deadline: Instant) -> Result<Value> {
    let key = format!("{}-{}", root.source_block, hex::encode(root.root));
    let source_block = u64::from(root.source_block);
    let expected_block_hash = format!("0x{}", hex::encode(root.block_hash.0));
    let expected_root = format!("0x{}", hex::encode(root.root));
    let expected_publication = ctx
        .follower_dir
        .join("root-publications")
        .join(format!("{key}.json"));
    let state_path = ctx.follower_dir.join("state.json");
    let queue = ethereum_client::abi::IMessageQueue::new(
        ctx.ethereum.queue_address,
        ctx.ethereum.api.raw_provider().clone(),
    );
    loop {
        ensure!(
            Instant::now() < deadline,
            "campaign deadline expired waiting for actor root publication"
        );
        follower_ready(ctx, root.source_block.saturating_add(1), deadline).await?;
        let Some(state) = ctx.follower_state()? else {
            sleep(Duration::from_secs(3)).await;
            continue;
        };
        let Some(entry) = state["roots"].get(&key) else {
            sleep(Duration::from_secs(3)).await;
            continue;
        };
        ensure!(
            entry["block"].as_u64() == Some(source_block)
                && entry["blockHash"] == expected_block_hash
                && entry["queueId"].as_u64() == Some(root.queue_id)
                && entry["queueRoot"] == expected_root,
            "actor root registration differs from the exact queued message"
        );
        if !actor_root_accepted(&entry["status"])? {
            sleep(Duration::from_secs(3)).await;
            continue;
        }
        let publication_path =
            PathBuf::from(string(&entry["publication"], "root publication path")?);
        ensure!(
            publication_path == expected_publication,
            "actor root publication path differs from the durable per-root path"
        );
        let publication: Value = serde_json::from_slice(
            &fs::read(&publication_path).context("read actor root publication evidence")?,
        )?;
        ensure!(
            publication["status"] == "accepted"
                && publication["finalityStatus"] == "finalized"
                && (publication["publicationReceipt"]["finalized"] == true
                    || publication["reconciledAt"]["finalized"] == true),
            "mined root publication is not finalized acceptance evidence"
        );
        ensure!(
            publication["schemaVersion"] == 3
                && publication["sourceBlock"].as_u64() == Some(source_block)
                && publication["root"] == expected_root
                && publication["localRehearsal"] == false
                && publication["proof"]["sourceBlock"].as_u64() == Some(source_block)
                && publication["proof"]["sourceHash"] == expected_block_hash
                && publication["proof"]["queueId"].as_u64() == Some(root.queue_id)
                && publication["proof"]["queueRoot"] == expected_root,
            "actor root publication has the wrong schema, root, or rehearsal mode"
        );
        let expected_identity = json!({
            "sourceGenesis": ctx.deployment["ethereum"]["sourceGenesis"],
            "sourceDomain": ctx.deployment["ethereum"]["sourceDomain"],
            "bridgeDomain": ctx.deployment["ethereum"]["bridgeDomain"],
            "destinationChainId": ctx.deployment["ethereum"]["chainId"],
            "destinationQueue": ctx.deployment["ethereum"]["queue"],
        });
        ensure!(
            publication["sourceIdentity"] == expected_identity
                && parse_actor_address(&publication["sender"], "publication.sender")?
                    == ctx.publisher_address,
            "actor root publication belongs to another source, deployment, or publisher"
        );
        let anchor_block: u32 = publication["proof"]["anchorBlock"]
            .as_u64()
            .context("publication BEEFY anchor block is missing")?
            .try_into()?;
        ensure!(
            anchor_block > root.source_block,
            "publication is not pinned to a covering accepted BEEFY anchor"
        );
        let anchor_hash = GearHash(bytes32(string(
            &publication["proof"]["anchorHash"],
            "publication BEEFY anchor hash",
        )?)?);
        let source_finalized = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?;
        let witness_finalized = ctx
            .witness
            .api
            .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
            .await?;
        if source_finalized < anchor_block || witness_finalized < anchor_block {
            sleep(Duration::from_secs(3)).await;
            continue;
        }
        ensure!(
            ctx.source.api.block_number_to_hash(anchor_block).await? == anchor_hash
                && ctx.witness.api.block_number_to_hash(anchor_block).await? == anchor_hash,
            "publication BEEFY anchor is not canonical on both Gear nodes"
        );
        let accepted_checkpoint = publication["acceptedCheckpoint"]["block"]
            .as_u64()
            .context("publication accepted checkpoint block is missing")?;
        let accepted_checkpoint_hash: B256 = string(
            &publication["acceptedCheckpoint"]["blockHash"],
            "publication accepted checkpoint hash",
        )?
        .parse()
        .context("parse publication accepted checkpoint hash")?;
        let accepted_checkpoint_header = ctx
            .ethereum
            .api
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(accepted_checkpoint))
            .await?
            .context("publication accepted checkpoint block is unavailable")?;
        ensure!(
            accepted_checkpoint_header.header.hash == accepted_checkpoint_hash,
            "publication accepted checkpoint is not canonical on Hoodi"
        );
        let anchor_tx: B256 =
            string(&publication["acceptedAnchorTx"], "root anchor transaction")?.parse()?;
        let anchor_client: Address =
            string(&publication["acceptedAnchorClient"], "root anchor client")?.parse()?;
        ensure!(
            anchor_client == ctx.ethereum.client_address,
            "root anchor belongs to another active client"
        );
        let finalized_anchor = ctx
            .ethereum
            .api
            .get_finalized_receipt(anchor_tx)
            .await?
            .context("accepted root anchor is not finalized")?;
        ensure!(
            finalized_anchor.receipt.transaction_hash == anchor_tx
                && finalized_anchor.receipt.status()
                && finalized_anchor.receipt.to == Some(anchor_client)
                && finalized_anchor.included_block_number == accepted_checkpoint
                && finalized_anchor.included_block_hash == accepted_checkpoint_hash,
            "finalized anchor differs from the original pinned root checkpoint"
        );
        let (completion_kind, completion) = if publication["publicationReceipt"].is_object() {
            ("publicationReceipt", &publication["publicationReceipt"])
        } else if publication["reconciledAt"].is_object() {
            ("reconciledAt", &publication["reconciledAt"])
        } else {
            bail!("accepted actor root publication has no canonical completion evidence");
        };
        let scan_from = completion["block"]
            .as_u64()
            .context("publication completion block is missing")?;
        let completion_hash: B256 = string(
            &completion["blockHash"],
            "publication completion block hash",
        )?
        .parse()
        .context("parse publication completion block hash")?;
        let completion_block = ctx
            .ethereum
            .api
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(scan_from))
            .await?
            .context("publication completion block is unavailable")?;
        ensure!(
            completion_block.header.hash == completion_hash,
            "publication completion block is not canonical"
        );
        let source_hash = ctx
            .source
            .api
            .block_number_to_hash(root.source_block)
            .await?;
        ensure!(
            source_hash == root.block_hash
                && ctx
                    .witness
                    .api
                    .block_number_to_hash(root.source_block)
                    .await?
                    == root.block_hash,
            "actor root registration is no longer canonical on both Gear nodes"
        );
        let registered = queue
            .getMerkleRoot(EthU256::from(source_block))
            .call()
            .await?;
        if registered.as_slice() == [0; 32] {
            sleep(Duration::from_secs(3)).await;
            continue;
        }
        ensure!(
            registered.as_slice() == root.root,
            "canonical Hoodi root differs from actor's accepted registration"
        );
        let registered_at = u64::try_from(
            queue
                .getMerkleRootTimestampForBlock(EthU256::from(source_block))
                .block(BlockId::hash_canonical(completion_hash))
                .call()
                .await?,
        )?;
        let delay = u64::try_from(
            queue
                .PROCESS_USER_MESSAGE_DELAY()
                .block(BlockId::hash_canonical(completion_hash))
                .call()
                .await?,
        )?;
        ensure!(
            registered_at > 0 && delay == 300,
            "root registration or 300-second maturity is invalid"
        );
        let tx_hash: B256 = string(&publication["txHash"], "root publication txHash")?.parse()?;
        let raw = hex::decode(
            string(&publication["rawTransaction"], "original root signed bytes")?
                .strip_prefix("0x")
                .context("root signed bytes lack hex prefix")?,
        )?;
        let nonce: u64 = string(&publication["nonce"], "original root nonce")?.parse()?;
        let signed = crate::ethereum::signed_transaction_identity(
            &raw,
            tx_hash,
            nonce,
            ctx.ethereum.queue_address,
        )?;
        ensure!(
            signed.from == ctx.publisher_address,
            "original root transaction signer differs from root publisher"
        );
        let receipt = ctx
            .ethereum
            .api
            .raw_provider()
            .get_transaction_receipt(tx_hash)
            .await?
            .context("accepted root publication transaction receipt is missing")?;
        ensure!(
            receipt.transaction_hash == tx_hash
                && receipt.status()
                && receipt.from == ctx.publisher_address
                && receipt.to == Some(ctx.ethereum.queue_address),
            "root publication receipt identity or status is invalid"
        );
        let receipt_block = receipt
            .block_number
            .context("root publication receipt block is missing")?;
        let receipt_hash = receipt
            .block_hash
            .context("root publication receipt hash is missing")?;
        let canonical = ctx
            .ethereum
            .api
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(receipt_block))
            .await?
            .context("root publication block is missing")?;
        ensure!(
            canonical.header.hash == receipt_hash,
            "root publication receipt is not canonical"
        );
        let finalized_el = ctx.ethereum.api.finalized_block_number().await?;
        ensure!(
            receipt.transaction_hash == tx_hash && receipt_block <= finalized_el,
            "accepted root receipt is not finalized on Hoodi"
        );
        if completion_kind == "publicationReceipt" {
            ensure!(
                completion["block"].as_u64() == Some(receipt_block)
                    && completion["blockHash"] == format!("{receipt_hash:#x}")
                    && completion["finalizedBlock"].as_u64().is_some_and(|height| {
                        height >= receipt_block && height <= finalized_el
                    }),
                "finalized root receipt differs from its recorded original inclusion"
            );
            let finality_height = number(&completion["finalizedBlock"])?;
            let finality_hash: B256 = string(
                &completion["finalizedBlockHash"],
                "root publication finalized block hash",
            )?
            .parse()?;
            let canonical_finality = ctx
                .ethereum
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Number(finality_height))
                .await?
                .context("root publication finalized block is unavailable")?;
            ensure!(
                canonical_finality.header.hash == finality_hash,
                "root publication finality head is not canonical"
            );
        }
        return Ok(json!({
            "transactionHash":format!("{tx_hash:#x}"),"receiptBlock":receipt_block,"receiptBlockHash":format!("{receipt_hash:#x}"),
            "registrationTimestampMs":registered_at.checked_mul(1_000).context("root timestamp overflow")?,
            "maturityEligibleAtMs":registered_at.checked_add(delay).and_then(|t| t.checked_mul(1_000)).context("maturity timestamp overflow")?,
            "publicationObservedAtMs":now_ms()?,
            "sourceBlock":source_block,
            "sourceBlockHash":expected_block_hash,
            "queueId":root.queue_id,
            "queueRoot":expected_root,
            "actorState":state_path,
            "publication":publication_path,
            "publicationSchemaVersion":3,
            "acceptedCheckpoint":publication["acceptedCheckpoint"],
            "completionKind":completion_kind,
            "completionBlock":scan_from,
            "completionBlockHash":format!("{completion_hash:#x}"),
            "ethScanStartBlock":scan_from,
        }));
    }
}
fn release_source_block(
    data: &[u8],
    minimum_source_block: u64,
    expected_hash: B256,
    nonce: EthU256,
    destination: Address,
) -> Option<u64> {
    if data.len() != 128 {
        return None;
    }
    let block: u64 = EthU256::from_be_slice(&data[..32]).try_into().ok()?;
    // A later authenticated root can cover the same original queued message.
    (block >= minimum_source_block
        && B256::from_slice(&data[32..64]) == expected_hash
        && EthU256::from_be_slice(&data[64..96]) == nonce
        && Address::from_slice(&data[108..128]) == destination)
        .then_some(block)
}

fn receipt_token_effect(
    receipt: &alloy::rpc::types::TransactionReceipt,
    manager: Address,
    token: Address,
    sender: B256,
    receiver: Address,
    amount: EthU256,
) -> Result<Value> {
    let mut matched = None;
    for log in receipt.as_ref().logs() {
        if log.address() != manager || log.topics().len() != 4 || log.data().data.len() != 32 {
            continue;
        }
        let Ok(event) = CampaignManager::Bridged::decode_raw_log_validate(
            log.topics().iter().copied(),
            log.data().data.as_ref(),
        ) else {
            continue;
        };
        if (event.from, event.to, event.token, event.amount) != (sender, receiver, token, amount) {
            continue;
        }
        ensure!(
            matched.is_none(),
            "original finalized receipt has ambiguous duplicate token effects; HOLD"
        );
        matched = Some(
            json!({"transactionHash":format!("{:#x}",receipt.transaction_hash),"manager":format!("{manager:#x}"),"token":format!("{token:#x}"),"sender":format!("{sender:#x}"),"receiver":format!("{receiver:#x}"),"amountRaw":amount.to_string(),"topics":log.topics().iter().map(|topic|format!("{topic:#x}")).collect::<Vec<_>>(),"data":format!("0x{}",hex::encode(log.data().data.as_ref()))}),
        );
    }
    matched.context("original finalized MessageProcessed receipt lacks matching ERC20Manager.Bridged effect; HOLD")
}

async fn wait_release(
    ctx: &Context,
    nonce: EthU256,
    expected_hash: [u8; 32],
    source_block: u64,
    scan_from: u64,
    deadline: Instant,
    (token, amount): (&Token, EthU256),
) -> Result<Value> {
    let provider = ctx.ethereum.api.raw_provider();
    let queue =
        ethereum_client::abi::IMessageQueue::new(ctx.ethereum.queue_address, provider.clone());
    let signature = B256::from(keccak256(
        b"MessageProcessed(uint256,bytes32,uint256,address)",
    ));
    let expected_hash = B256::from(expected_hash);
    let destination = ctx.ethereum.receiver_address;
    loop {
        ensure!(
            Instant::now() < deadline,
            "campaign deadline expired waiting for paid Gear-to-EVM release"
        );
        let latest = provider.get_block_number().await?;
        let filter = Filter::new()
            .address(ctx.ethereum.queue_address)
            .event_signature(signature)
            .from_block(scan_from)
            .to_block(latest);
        let logs = provider.get_logs(&filter).await?;
        for log in logs {
            if log.topics().len() != 1 {
                continue;
            }
            let Some(delivery_source_block) = release_source_block(
                log.data().data.as_ref(),
                source_block,
                expected_hash,
                nonce,
                destination,
            ) else {
                continue;
            };
            timeout_at(
                deadline.into(),
                ctx.ethereum
                    .wait_maturity(u32::try_from(delivery_source_block)?),
            )
            .await
            .map_err(|_| {
                anyhow!(
                    "campaign deadline expired waiting for the 300-second delivery-root maturity"
                )
            })??;
            let maturity_observed_at = now_ms()?;
            let tx_hash = log
                .transaction_hash
                .context("MessageProcessed log has no transaction hash")?;
            let receipt = wait_receipt_finalized(ctx, tx_hash, deadline).await?;
            let original = ctx
                .ethereum
                .api
                .get_finalized_receipt(tx_hash)
                .await?
                .context("original release receipt lost canonical finality; HOLD")?;
            ensure!(original.receipt.as_ref().logs().contains(&log),"MessageProcessed scan log does not belong to original finalized release receipt; HOLD");
            let token_effect = receipt_token_effect(
                &original.receipt,
                destination,
                token.address,
                B256::from(ctx.campaign_actor.into_bytes()),
                ctx.campaign_address,
                amount,
            )?;
            let finalized = number(&receipt["finalizedBlock"])?;
            let finalized_hash: B256 =
                string(&receipt["finalizedHash"], "release finalized hash")?.parse()?;
            ensure!(
                queue
                    .isProcessed(nonce)
                    .block(BlockId::hash_canonical(finalized_hash))
                    .call()
                    .await?,
                "release nonce is not processed at finalized EVM state"
            );
            return Ok(
                json!({"transactionHash":format!("{tx_hash:#x}"),"sourceBlock":delivery_source_block,"originalSourceBlock":source_block,"messageHash":format!("{expected_hash:#x}"),"nonce":nonce.to_string(),"destination":format!("{destination:#x}"),"receipt":receipt,"finalizedBlock":finalized,"finalizedHash":format!("{finalized_hash:#x}"),"maturityObservedAtMs":maturity_observed_at,"tokenEffect":token_effect}),
            );
        }
        let processed = queue.isProcessed(nonce).call().await?;
        if processed {
            // A processed bit without its matching event is unresolved; never substitute a different message.
            bail!("message nonce {} is processed but its matching MessageProcessed receipt is unavailable",nonce);
        }
        sleep(Duration::from_secs(6)).await;
    }
}

async fn revalidate_passed_windows(ctx: &Context, journal: &Journal) -> Result<()> {
    if journal.warmup.status == "passed" {
        let started = journal
            .warmup
            .started_at_ms
            .context("passed warmup has no original sampling clock; HOLD")?;
        let deadline = started
            .checked_add(ctx.schedule.warmup_secs * 1_000)
            .context("warmup deadline overflow")?;
        ensure!(
            journal.warmup.evidence["deadlineAtMs"].as_u64() == Some(deadline)
                && journal
                    .warmup
                    .completed_at_ms
                    .is_some_and(|at| at >= deadline)
                && journal
                    .readiness
                    .get("warmup_terminal")
                    .is_some_and(|view| view["ready"] == true),
            "passed warmup lacks its original full-hour terminal worker gate; HOLD"
        );
        validate_warmup_samples(
            journal.warmup.evidence["samples"]
                .as_array()
                .context("passed warmup has no original minute observations; HOLD")?,
            started,
            ctx.schedule.warmup_secs / 60,
        )?;
        if ctx.schedule.handovers == 2 {
            let records = journal.warmup.evidence["handoverCommitments"]
                .as_array()
                .context("normal handover evidence missing")?;
            ensure!(
                records.len() == 2,
                "normal warmup lacks both original handovers; HOLD"
            );
            let mut previous = json!({"current":journal.warmup.evidence["initialCurrent"],"next":journal.warmup.evidence["initialNext"]});
            for record in records {
                authenticate_normal_handover(ctx, &record["original"], &previous).await?;
                previous = record.clone();
            }
        }
        let baseline = SnapshotSet::from_json(&journal.warmup.evidence["baseline"])?;
        let terminal = SnapshotSet::from_json(&journal.warmup.evidence["terminalSnapshot"])?;
        ctx.snapshot_at(Some(&baseline)).await?;
        ctx.snapshot_at(Some(&terminal)).await?;
        verify_roundtrip_delta(&baseline, &terminal, &ctx.tokens)?;
    }
    if RECEIPT_PROBES.iter().any(|probe| {
        journal
            .actions
            .get(probe.action_id())
            .is_some_and(|action| action.status == "finalized")
    }) {
        revalidate_receipt_probes(ctx, journal).await?;
    }
    for (label, window) in journal
        .windows
        .iter()
        .filter(|(_, value)| value["status"] == "passed")
    {
        let baseline = SnapshotSet::from_json(&window["baseline"])?;
        ctx.snapshot_at(Some(&baseline)).await?;
        let minted = SnapshotSet::from_json(&window["stages"]["afterMint"])?;
        ctx.snapshot_at(Some(&minted)).await?;
        let returned = SnapshotSet::from_json(&window["stages"]["finalizedReturn"])?;
        ctx.snapshot_at(Some(&returned)).await?;
        let assets = window["assets"]
            .as_object()
            .context("passed window asset evidence missing")?;
        ensure!(
            assets.len() == ctx.tokens.len(),
            "passed window asset set changed; HOLD"
        );
        for token in &ctx.tokens {
            let asset = assets
                .get(token.symbol)
                .context("passed asset evidence missing")?;
            ensure!(
                asset["status"] == "passed",
                "passed window contains incomplete asset evidence; HOLD"
            );
            for saved in [
                &asset["lock"]["receipt"],
                &asset["burn"]["releaseReceipt"]["receipt"],
            ]
            .into_iter()
            .chain(
                journal
                    .actions
                    .get(&format!("{label}/{}-erc20-allowance", token.symbol))
                    .map(|action| &action.evidence["receipt"]),
            ) {
                let hash: B256 =
                    string(&saved["transactionHash"], "saved receipt transaction hash")?.parse()?;
                let receipt = ctx
                    .ethereum
                    .api
                    .get_finalized_receipt(hash)
                    .await?
                    .context("previously passed receipt is not canonically finalized; HOLD")?;
                if saved == &asset["burn"]["releaseReceipt"]["receipt"] {
                    receipt_token_effect(
                        &receipt.receipt,
                        ctx.ethereum.receiver_address,
                        token.address,
                        B256::from(ctx.campaign_actor.into_bytes()),
                        ctx.campaign_address,
                        EthU256::from(token.raw_amount(WINDOW_AMOUNT)?),
                    )?;
                }
                ensure!(
                    receipt.receipt.transaction_hash == hash
                        && receipt.receipt.status()
                        && receipt.included_block_number == number(&saved["receiptBlock"])?
                        && receipt.included_block_hash
                            == string(&saved["receiptBlockHash"], "saved receipt inclusion hash")?
                                .parse::<B256>()?,
                    "passed {label}/{} original receipt changed; HOLD",
                    token.symbol
                );
                let anchor = string(&saved["finalizedHash"], "saved receipt finalized hash")?
                    .parse::<B256>()?;
                ensure!(
                    ctx.ethereum
                        .api
                        .is_finalized_block(number(&saved["finalizedBlock"])?, anchor.0.into())
                        .await?,
                    "passed receipt finality anchor changed; HOLD"
                );
            }
            for (height, hash) in [
                (
                    &asset["lock"]["receiptStatus"]["finalizedBlock"],
                    &asset["lock"]["receiptStatus"]["finalizedHash"],
                ),
                (
                    &asset["burn"]["outboundRequest"]["managerEventBlock"],
                    &asset["burn"]["outboundRequest"]["managerEventBlockHash"],
                ),
                (
                    &asset["burn"]["outboundRequest"]["queueBlock"],
                    &asset["burn"]["outboundRequest"]["queueBlockHash"],
                ),
                (
                    &asset["paid"]["paidEvent"]["block"],
                    &asset["paid"]["paidEvent"]["blockHash"],
                ),
            ] {
                let height = u32::try_from(number(height)?)?;
                let hash: GearHash = string(hash, "passed source block hash")?.parse()?;
                ensure!(
                    height <= returned.gear_height
                        && ctx.source.api.block_number_to_hash(height).await? == hash
                        && ctx.witness.api.block_number_to_hash(height).await? == hash,
                    "passed {label}/{} original source event changed; HOLD",
                    token.symbol
                );
            }
        }
    }
    Ok(())
}

async fn wait_receipt_finalized(ctx: &Context, tx_hash: B256, deadline: Instant) -> Result<Value> {
    loop {
        ensure!(
            Instant::now() < deadline,
            "absolute campaign deadline expired awaiting canonical EVM receipt"
        );
        if let Some(finalized) = ctx.ethereum.api.get_finalized_receipt(tx_hash).await? {
            ensure!(
                finalized.receipt.transaction_hash == tx_hash && finalized.receipt.status(),
                "EVM transaction identity changed or reverted: {tx_hash:#x}"
            );
            let block = ctx
                .ethereum
                .api
                .raw_provider()
                .get_block_by_hash(finalized.included_block_hash)
                .await?
                .context("finalized receipt block is missing")?;
            ensure!(
                block.header.number == finalized.included_block_number
                    && block.header.hash == finalized.included_block_hash,
                "finalized receipt header changed; HOLD"
            );
            return Ok(json!({
                "transactionHash":format!("{tx_hash:#x}"),"receiptBlock":finalized.included_block_number,
                "receiptBlockHash":format!("{:#x}",finalized.included_block_hash),"finalizedBlock":finalized.finalized_block_number,
                "finalizedHash":format!("{:#x}",finalized.finalized_block_hash),
                "blockTimestampMs":block.header.timestamp.checked_mul(1_000).context("receipt timestamp overflow")?,
                "finalityObservedAtMs":now_ms()?,
            }));
        }
        sleep(Duration::from_secs(6)).await;
    }
}

async fn wait_mined_approvals(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    actions: &[(String, B256)],
    deadline: Instant,
) -> Result<()> {
    let provider = ctx.ethereum.api.raw_provider();
    for (id, tx_hash) in actions {
        loop {
            ensure!(
                Instant::now() < deadline,
                "original window deadline expired awaiting mined approval; HOLD"
            );
            let Some(receipt) = provider.get_transaction_receipt(*tx_hash).await? else {
                sleep_until(deadline.min(Instant::now() + Duration::from_secs(3)).into()).await;
                continue;
            };
            let action = journal
                .actions
                .get(id)
                .context("approval action is missing")?;
            let token: Address = string(&action.intent["token"], "approval token")?.parse()?;
            let amount: EthU256 =
                string(&action.intent["amountRaw"], "approval amount")?.parse()?;
            let block = receipt
                .block_number
                .context("approval receipt block is missing")?;
            let hash = receipt
                .block_hash
                .context("approval receipt hash is missing")?;
            let header = provider
                .get_block_by_number(BlockNumberOrTag::Number(block))
                .await?
                .context("approval inclusion block is missing")?;
            ensure!(
                receipt.transaction_hash == *tx_hash
                    && receipt.status()
                    && receipt.from == ctx.campaign_address
                    && receipt.to == Some(token)
                    && header.header.hash == hash,
                "approval reverted, changed identity or lost canonical mined inclusion; HOLD"
            );
            let transaction = ethereum_client::transaction_identity(provider, *tx_hash)
                .await?
                .context("mined approval transaction is missing")?;
            ensure!(
                transaction.from == ctx.campaign_address
                    && transaction.to == Some(token)
                    && Some(transaction.nonce) == action.nonce,
                "approval is not the campaign's original ordered nonce; HOLD"
            );
            let spender: Address = ctx.ethereum.receiver_address().into();
            let event = B256::from(keccak256(b"Approval(address,address,uint256)"));
            ensure!(
                receipt.as_ref().logs().iter().any(|log| {
                    let topics = log.topics();
                    log.address() == token
                        && topics.len() == 3
                        && topics[0] == event
                        && topics[1] == B256::from(address_word(ctx.campaign_address))
                        && topics[2] == B256::from(address_word(spender))
                        && log.data().data.len() == 32
                        && EthU256::from_be_slice(log.data().data.as_ref()) == amount
                }),
                "original approval receipt has no matching Approval event; HOLD"
            );
            let observed = json!({"transactionHash":format!("{tx_hash:#x}"),"receiptBlock":block,
                "receiptBlockHash":format!("{hash:#x}"),
                "blockTimestampMs":header.header.timestamp.checked_mul(1_000).context("approval timestamp overflow")?});
            if let Some(saved) = action.evidence.get("minedReceipt") {
                ensure!(
                    *saved == observed,
                    "original mined approval inclusion changed; HOLD"
                );
            } else {
                set_action_evidence(journal, id, "minedReceipt", observed);
                record_action_milestone(journal, id, "evmMinedObservedAtMs", now_ms()?)?;
                let action = journal.actions.get_mut(id).expect("approval exists");
                if action.status != "finalized" {
                    action.status = "mined".into();
                }
                save_journal(path, journal)?;
            }
            break;
        }
    }
    Ok(())
}

fn ensure_approval_precedes_lock(approval: &Action, lock_nonce: u64) -> Result<()> {
    let original = approval
        .tx_hash
        .as_deref()
        .context("approval has no original transaction identity")?;
    ensure!(matches!(approval.status.as_str(), "mined" | "finalized")
        && approval.evidence["minedReceipt"]["transactionHash"].as_str() == Some(original)
        && approval.evidence["minedReceipt"]["receiptBlock"].as_u64().is_some()
        && approval.nonce.is_some_and(|nonce| nonce < lock_nonce),
        "dependent lock requires the original successful mined approval at an earlier campaign nonce; HOLD");
    Ok(())
}

async fn wait_finalized_actions(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    actions: &[(String, B256)],
    deadline: Instant,
) -> Result<Vec<Value>> {
    let mut pending: FuturesUnordered<_> = actions
        .iter()
        .map(|(id, hash)| async move {
            (
                id,
                *hash,
                wait_receipt_finalized(ctx, *hash, deadline).await,
            )
        })
        .collect();
    let mut receipts = Vec::with_capacity(actions.len());
    while let Some((id, hash, result)) = pending.next().await {
        let observed = result?;
        if let Some(mined) = journal.actions[id].evidence.get("minedReceipt") {
            for field in [
                "transactionHash",
                "receiptBlock",
                "receiptBlockHash",
                "blockTimestampMs",
            ] {
                ensure!(
                    mined[field] == observed[field],
                    "finalized approval differs from original mined inclusion; HOLD"
                );
            }
        }
        let evidence = if let Some(saved) = journal.actions[id].evidence.get("receipt") {
            for field in [
                "transactionHash",
                "receiptBlock",
                "receiptBlockHash",
                "blockTimestampMs",
            ] {
                ensure!(
                    saved[field] == observed[field],
                    "original finalized action receipt changed; HOLD"
                );
            }
            let anchor: B256 =
                string(&saved["finalizedHash"], "saved action finalized hash")?.parse()?;
            ensure!(
                ctx.ethereum
                    .api
                    .is_finalized_block(number(&saved["finalizedBlock"])?, anchor.0.into())
                    .await?,
                "original action finality anchor changed; HOLD"
            );
            saved.clone()
        } else {
            observed
        };
        let action = journal
            .actions
            .get_mut(id)
            .context("finalized EVM action is missing")?;
        action.status = "finalized".into();
        action.tx_hash = Some(format!("{hash:#x}"));
        action.evidence["receipt"] = evidence.clone();
        record_action_milestone(
            journal,
            id,
            "evmBlockTimestampMs",
            number(&evidence["blockTimestampMs"])?,
        )?;
        record_action_milestone(
            journal,
            id,
            "evmFinalityObservedAtMs",
            number(&evidence["finalityObservedAtMs"])?,
        )?;
        save_journal(path, journal)?;
        receipts.push(evidence);
    }
    Ok(receipts)
}

async fn ensure_beacon_through_receipts(
    ctx: &Context,
    receipts: &[Value],
    deadline: Instant,
) -> Result<()> {
    if let Some(block) = receipt_coverage_height(receipts)? {
        let genesis_time = ctx.beacon.get_genesis().await?.data.genesis_time;
        let required_slot = receipts
            .iter()
            .try_fold(0, |highest, receipt| -> Result<u64> {
                let timestamp = number(&receipt["blockTimestampMs"])? / 1_000;
                let slot = timestamp
                    .checked_sub(genesis_time)
                    .context("receipt timestamp precedes Beacon genesis")?
                    / SECONDS_PER_SLOT;
                Ok(highest.max(slot))
            })?;
        wait_beacon_coverage(deadline, || async {
            if !check_beacon_el_finality(&ctx.beacon, &ctx.ethereum, block).await? {
                return Ok(false);
            }
            // Beacon finality can advance while the Gear checkpoint worker is offline.
            let finalized = ctx.source.api.latest_finalized_block().await?;
            match ServiceCheckpointFor::new(ctx.remoting.clone())
                .get(required_slot)
                .at_block(sails_block_hash(finalized))
                .recv(ctx.checkpoint_id)
                .await?
            {
                Ok(_) => Ok(true),
                Err(CheckpointError::NotPresent) => Ok(false),
                Err(error) => bail!("checkpoint cannot cover finalized receipts: {error:?}"),
            }
        })
        .await?;
    }
    Ok(())
}

async fn wait_beacon_coverage<F, Fut>(deadline: Instant, mut check: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    loop {
        ensure!(
            Instant::now() < deadline,
            "absolute campaign deadline expired awaiting Beacon and EL finality"
        );
        if timeout_at(deadline.into(), check()).await.map_err(|_| {
            anyhow!("absolute campaign deadline expired awaiting Beacon and EL finality")
        })?? {
            ensure!(
                Instant::now() < deadline,
                "absolute campaign deadline expired awaiting Beacon and EL finality"
            );
            return Ok(());
        }
        sleep_until(deadline.min(Instant::now() + Duration::from_secs(6)).into()).await;
    }
}

fn receipt_coverage_height(receipts: &[Value]) -> Result<Option<u64>> {
    receipts
        .iter()
        .try_fold(None, |highest: Option<u64>, receipt| {
            let block = number(&receipt["receiptBlock"])?;
            Ok(Some(highest.map_or(block, |previous| previous.max(block))))
        })
}

fn campaign_evm_identity(
    raw: &[u8],
    hash: B256,
    nonce: u64,
    intent: &Value,
    campaign: Address,
    manager: Address,
) -> Result<ethereum_client::TransactionIdentity> {
    let mut bytes = raw;
    let envelope = alloy::consensus::TxEnvelope::decode_2718(&mut bytes)?;
    ensure!(
        bytes.is_empty()
            && envelope.chain_id() == Some(HOODI_CHAIN_ID)
            && envelope.value().is_zero(),
        "original campaign EVM chain/value/framing changed; HOLD"
    );
    let amount = EthU256::from_str(string(&intent["amountRaw"], "original raw EVM amount")?)?;
    let token = Address::from_str(string(&intent["token"], "original EVM token")?)?;
    let destination = if intent["kind"] == "erc20-approve" {
        let call = CampaignToken::approveCall::abi_decode(envelope.input())?;
        let spender = intent["spender"]
            .as_str()
            .map(Address::from_str)
            .transpose()?
            .unwrap_or(manager);
        ensure!(
            call.spender == spender && call.amount == amount,
            "original campaign approval calldata changed"
        );
        token
    } else {
        ensure!(
            intent["kind"] == "erc20-lock",
            "unsupported original campaign EVM intent"
        );
        let recipient =
            B256::from_str(string(&intent["gearRecipient"], "original Gear recipient")?)?;
        let (actual_token, actual_amount, actual_to) = if intent["permit"] == true {
            let call =
                CampaignManager::requestBridgingWithPermitCall::abi_decode(envelope.input())?;
            (call.token, call.amount, call.to)
        } else {
            let call = CampaignManager::requestBridgingCall::abi_decode(envelope.input())?;
            (call.token, call.amount, call.to)
        };
        ensure!(
            (actual_token, actual_amount, actual_to) == (token, amount, recipient),
            "original campaign lock calldata changed"
        );
        manager
    };
    let identity = crate::ethereum::signed_transaction_identity(raw, hash, nonce, destination)?;
    ensure!(
        identity.from == campaign,
        "original campaign EVM signer changed; HOLD"
    );
    Ok(identity)
}

async fn evm_action<F, Fut, R, RFut>(
    ctx: &Context,
    journal: &mut Journal,
    path: &Path,
    id: &str,
    intent: Value,
    prepare: F,
    recover: R,
) -> Result<B256>
where
    F: FnOnce(u64) -> Fut,
    Fut: Future<Output = Result<alloy::rpc::types::TransactionRequest>>,
    R: FnOnce(u64, u64) -> RFut,
    RFut: Future<Output = Result<Option<B256>>>,
{
    if let Some(action) = journal.actions.get(id) {
        ensure!(
            action.intent == intent,
            "saved EVM action intent differs for {id}"
        );
        if action.evidence["rawTransaction"].is_null() {
            if let Some(hash) = &action.tx_hash {
                ensure!(
                    ctx.schedule.handovers != 2,
                    "named campaign cannot adopt legacy unsigned EVM evidence; HOLD"
                );
                return B256::from_str(hash).context("legacy original EVM hash");
            }
            if action.status == "broadcasting" {
                ensure!(
                    ctx.schedule.handovers != 2,
                    "named campaign ambiguous unsigned EVM action must remain HOLD"
                );
                let hash = recover(
                    action.from_block.context("original start block")?,
                    action.nonce.context("original nonce")?,
                )
                .await?
                .context("legacy EVM outcome ambiguous; HOLD without replacement")?;
                let action = journal.actions.get_mut(id).context("EVM intent missing")?;
                action.tx_hash = Some(format!("{hash:#x}"));
                action.status = "submitted".into();
                save_journal(path, journal)?;
                return Ok(hash);
            }
            ensure!(
                action.status == "prepared",
                "EVM action has no recoverable original bytes; HOLD"
            );
        }
    }
    let provider = ctx.ethereum.api.raw_provider();
    if journal
        .actions
        .get(id)
        .is_none_or(|action| action.evidence["rawTransaction"].is_null())
    {
        let pending = provider
            .get_transaction_count(ctx.campaign_address)
            .pending()
            .await?;
        let nonce = journal
            .actions
            .get(id)
            .map(|action| action.nonce.context("prepared nonce missing"))
            .transpose()?
            .unwrap_or(journal.next_campaign_nonce.unwrap_or(pending));
        ensure!(
            pending <= nonce,
            "campaign account has an unjournaled pending nonce; HOLD"
        );
        let start = provider.get_block_number().await?;
        journal.actions.entry(id.into()).or_insert(Action {
            status: "prepared".into(),
            intent: intent.clone(),
            from_block: Some(start),
            nonce: Some(nonce),
            tx_hash: None,
            evidence: json!({}),
        });
        save_journal(path, journal)?;
        let filled = provider.fill(prepare(nonce).await?).await?;
        let raw = filled
            .as_envelope()
            .context("campaign filler did not sign original EVM bytes")?
            .encoded_2718();
        let hash = B256::from(keccak256(&raw));
        campaign_evm_identity(
            &raw,
            hash,
            nonce,
            &intent,
            ctx.campaign_address,
            ctx.ethereum.receiver_address().into(),
        )?;
        let action = journal
            .actions
            .get_mut(id)
            .context("EVM action disappeared")?;
        action.evidence["rawTransaction"] = json!(format!("0x{}", hex::encode(&raw)));
        action.tx_hash = Some(format!("{hash:#x}"));
        action.status = "broadcasting".into();
        journal.next_campaign_nonce = Some(
            nonce
                .checked_add(1)
                .context("campaign EVM nonce overflow")?,
        );
        record_action_milestone(journal, id, "submissionHandoffAtMs", now_ms()?)?;
        save_journal(path, journal)?;
    }
    let action = &journal.actions[id];
    let nonce = action.nonce.context("signed nonce missing")?;
    let raw = hex::decode(
        string(
            &action.evidence["rawTransaction"],
            "original signed EVM bytes",
        )?
        .trim_start_matches("0x"),
    )?;
    let hash = B256::from_str(
        action
            .tx_hash
            .as_deref()
            .context("original signed hash missing")?,
    )?;
    let signed = campaign_evm_identity(
        &raw,
        hash,
        nonce,
        &intent,
        ctx.campaign_address,
        ctx.ethereum.receiver_address().into(),
    )?;
    if let Some(observed) = ethereum_client::transaction_identity(provider, hash).await? {
        crate::ethereum::ensure_same_submission(observed, signed)?;
        return Ok(hash);
    }
    let pending = provider
        .get_transaction_count(ctx.campaign_address)
        .pending()
        .await?;
    let latest = provider.get_transaction_count(ctx.campaign_address).await?;
    ensure!(
        pending <= nonce && latest <= nonce,
        "original signed EVM nonce consumed by unknown transaction; HOLD without replacement"
    );
    let submitted = provider.send_raw_transaction(&raw).await?;
    ensure!(
        *submitted.tx_hash() == hash,
        "provider changed original signed campaign hash; HOLD"
    );
    journal
        .actions
        .get_mut(id)
        .context("EVM action missing")?
        .status = "submitted".into();
    save_journal(path, journal)?;
    Ok(hash)
}

async fn canonical_log_transaction(
    provider: &impl Provider,
    log: &alloy::rpc::types::Log,
    destination: Address,
) -> Result<ethereum_client::TransactionIdentity> {
    let hash = log
        .transaction_hash
        .context("matching event has no transaction hash")?;
    let tx = ethereum_client::transaction_identity(provider, hash)
        .await?
        .context("matching event transaction is missing")?;
    let receipt = provider
        .get_transaction_receipt(hash)
        .await?
        .context("matching event transaction receipt is missing")?;
    let block = log
        .block_number
        .context("matching event has no block number")?;
    let block_hash = log.block_hash.context("matching event has no block hash")?;
    ensure!(
        receipt.transaction_hash == hash
            && receipt.status()
            && receipt.from == tx.from
            && receipt.to == tx.to
            && tx.to == Some(destination)
            && tx.block_number == Some(block)
            && receipt.block_number == Some(block)
            && receipt.block_hash == Some(block_hash)
            && receipt.as_ref().logs().contains(log),
        "matching event differs from its original canonical transaction receipt"
    );
    let canonical = provider
        .get_block_by_number(BlockNumberOrTag::Number(block))
        .await?
        .context("matching event block is missing")?;
    ensure!(
        canonical.header.hash == block_hash,
        "matching event transaction receipt is not canonical"
    );
    Ok(tx)
}

async fn find_lock_tx(
    ctx: &Context,
    token: Address,
    amount: EthU256,
    recipient: B256,
    from: u64,
    expected_nonce: u64,
) -> Result<Option<B256>> {
    let signature = B256::from(keccak256(
        b"BridgingRequested(address,bytes32,address,uint256)",
    ));
    let filter = Filter::new()
        .address(Address::from(ctx.ethereum.receiver_address()))
        .event_signature(signature)
        .from_block(from)
        .to_block(BlockNumberOrTag::Latest);
    let provider = ctx.ethereum.api.raw_provider();
    let logs = provider.get_logs(&filter).await?;
    let mut found = None;
    for log in logs {
        let topics = log.topics();
        if topics.len() != 4 || log.data().data.len() != 32 {
            continue;
        }
        let owner = Address::from_slice(&topics[1].as_slice()[12..]);
        let to = topics[2];
        let actual_token = Address::from_slice(&topics[3].as_slice()[12..]);
        let actual = EthU256::from_be_slice(log.data().data.as_ref());
        if owner != ctx.campaign_address
            || to != recipient
            || actual_token != token
            || actual != amount
        {
            continue;
        }
        let transaction =
            canonical_log_transaction(provider, &log, ctx.ethereum.receiver_address().into())
                .await?;
        if transaction.from != ctx.campaign_address || transaction.nonce != expected_nonce {
            continue;
        }
        ensure!(
            found.replace(transaction.hash).is_none(),
            "multiple EVM lock events match one saved campaign nonce"
        );
    }
    Ok(found)
}

async fn find_approval_tx(
    ctx: &Context,
    token: Address,
    owner: Address,
    spender: Address,
    amount: EthU256,
    from: u64,
    expected_nonce: u64,
) -> Result<Option<B256>> {
    let signature = B256::from(keccak256(b"Approval(address,address,uint256)"));
    let filter = Filter::new()
        .address(token)
        .event_signature(signature)
        .from_block(from)
        .to_block(BlockNumberOrTag::Latest);
    let provider = ctx.ethereum.api.raw_provider();
    let logs = provider.get_logs(&filter).await?;
    let mut found = None;
    for log in logs {
        let topics = log.topics();
        if topics.len() != 3 || log.data().data.len() != 32 {
            continue;
        }
        if Address::from_slice(&topics[1].as_slice()[12..]) != owner
            || Address::from_slice(&topics[2].as_slice()[12..]) != spender
            || EthU256::from_be_slice(log.data().data.as_ref()) != amount
        {
            continue;
        }
        let transaction = canonical_log_transaction(provider, &log, token).await?;
        if transaction.from != owner || transaction.nonce != expected_nonce {
            continue;
        }
        ensure!(
            found.replace(transaction.hash).is_none(),
            "multiple Approval events match one saved campaign nonce"
        );
    }
    Ok(found)
}

async fn circle_permit(
    ctx: &Context,
    token: Address,
    amount: EthU256,
) -> Result<(u64, u8, B256, B256)> {
    let erc20 = CampaignToken::new(token, ctx.ethereum.api.raw_provider().clone());
    let domain = erc20.DOMAIN_SEPARATOR().call().await?;
    let nonce = erc20.nonces(ctx.campaign_address).call().await?;
    let latest = ctx
        .ethereum
        .api
        .raw_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .context("latest Hoodi block is missing")?;
    let deadline = latest
        .header
        .timestamp
        .checked_add(600)
        .context("permit deadline overflow")?;
    let type_hash = keccak256(
        b"Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)",
    );
    let spender: Address = ctx.ethereum.receiver_address().into();
    let mut encoded = Vec::with_capacity(32 * 6);
    encoded.extend_from_slice(&type_hash);
    encoded.extend_from_slice(&address_word(ctx.campaign_address));
    encoded.extend_from_slice(&address_word(spender));
    encoded.extend_from_slice(&amount.to_be_bytes::<32>());
    encoded.extend_from_slice(&nonce.to_be_bytes::<32>());
    encoded.extend_from_slice(&EthU256::from(deadline).to_be_bytes::<32>());
    let structure = keccak256(&encoded);
    let mut digest_input = [0u8; 66];
    digest_input[0] = 0x19;
    digest_input[1] = 0x01;
    digest_input[2..34].copy_from_slice(domain.as_slice());
    digest_input[34..].copy_from_slice(&structure);
    let digest = B256::from(keccak256(&digest_input));
    let key = wallet_private_key(&ctx.campaign_wallet)?;
    let signer = PrivateKeySigner::from_str(&key).context("load protected Circle permit signer")?;
    let signature = signer
        .sign_hash(&digest)
        .await
        .context("sign Circle EIP-2612 permit")?;
    let bytes = signature.as_bytes();
    let v = match bytes[64] {
        0 | 1 => 27 + bytes[64],
        27 | 28 => bytes[64],
        _ => bail!("Circle permit signature has invalid recovery id"),
    };
    Ok((
        deadline,
        v,
        B256::from_slice(&bytes[..32]),
        B256::from_slice(&bytes[32..64]),
    ))
}

fn address_word(address: Address) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address.as_slice());
    word
}

fn wallet_private_key(wallet: &Path) -> Result<String> {
    let value: Value =
        serde_json::from_slice(&fs::read(wallet).context("read protected campaign wallet")?)?;
    let key = string(&value["private_key"], "wallet.private_key")?;
    ensure!(
        !key.trim().is_empty(),
        "campaign wallet private key is empty"
    );
    Ok(key.to_owned())
}

fn eth_u256(value: GearU256) -> Result<EthU256> {
    EthU256::from_str(&value.to_string()).context("convert Gear u256 nonce to EVM u256")
}

fn parse_actor_address(value: &Value, field: &str) -> Result<Address> {
    string(value, field)?
        .parse()
        .with_context(|| format!("parse {field} EVM address"))
}

fn follower_signers(state: &Value, deployment: &Value) -> Result<(Address, Address)> {
    hoodi::validate_token_follower_journal(state, deployment)?;
    ensure!(
        state["localRehearsal"] == false,
        "local-rehearsal follower cannot qualify"
    );
    ensure!(
        matches!(
            state["follower"]["status"].as_str(),
            Some("starting" | "healthy" | "catching-up" | "held-root" | "held-finality")
        ),
        "token follower journal is failed or has an unsupported status"
    );
    state["startupSequence"]
        .as_u64()
        .context("follower startup sequence is missing")?;
    Ok((
        parse_actor_address(&state["followerSigner"], "followerSigner")?,
        parse_actor_address(&state["rootPublisherSigner"], "rootPublisherSigner")?,
    ))
}

fn read_validated_follower_state(path: &Path, deployment: &Value) -> Result<Value> {
    let state = hoodi::read_state_file(path)?;
    follower_signers(&state, deployment)?;
    Ok(state)
}

async fn wait_for_follower_state(
    path: &Path,
    deployment: &Value,
    deadline: Instant,
) -> Result<Value> {
    timeout_at(deadline.into(), async {
        loop {
            if path.exists() {
                return read_validated_follower_state(path, deployment);
            }
            sleep(Duration::from_secs(5)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("token follower state did not appear before readiness deadline"))?
}

fn journal_follower_signers(journal: &Journal) -> Result<(Address, Address)> {
    Ok((
        parse_actor_address(&journal.accounts["followerEvm"], "accounts.followerEvm")?,
        parse_actor_address(
            &journal.accounts["rootPublisherEvm"],
            "accounts.rootPublisherEvm",
        )?,
    ))
}

async fn follower_ready(ctx: &Context, minimum_source: u32, deadline: Instant) -> Result<()> {
    loop {
        ensure!(
            Instant::now() < deadline,
            "follower did not catch up before readiness deadline"
        );
        let (state, checkpoint, mined) = coherent_follower_observation(ctx, deadline).await?;
        let source = ctx
            .source
            .api
            .block_hash_to_number(ctx.source.api.latest_finalized_block().await?)
            .await?;
        let now = now_ms()?;
        if state["follower"]["status"] == "healthy"
            && checkpoint.block >= u64::from(minimum_source)
            && u64::from(source) >= checkpoint.block
            && u64::from(source) - checkpoint.block < ctx.schedule.authority_lag_limit
            && mined.is_some()
            && state["follower"]["freshnessDeadlineMs"]
                .as_u64()
                .is_some_and(|until| until >= now)
        {
            return Ok(());
        }
        sleep_until(deadline.min(Instant::now() + Duration::from_secs(5)).into()).await;
    }
}

fn read_worker_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("malformed worker journal {}", path.display()))
            .map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read worker journal {}", path.display())),
    }
}

fn queued_work_matches(
    queued: &Map<String, Value>,
    allowed: &BTreeMap<String, OutboundRequest>,
) -> Result<bool> {
    for (key, value) in queued {
        let Some(request) = allowed.get(key) else {
            return Ok(false);
        };
        let message: relayer::message_relayer::common::MessageInBlock =
            serde_json::from_value(value.clone()).context("malformed queued worker message")?;
        let mut nonce = [0; 32];
        request.nonce.to_big_endian(&mut nonce);
        if message.message.nonce_be != nonce
            || message.block.0 != request.queue_block
            || message.block_hash.as_bytes() != request.queue_block_hash.as_bytes()
            || relayer::message_relayer::common::message_hash(&message.message)
                != request.message_hash
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn acquire_snapshot(
    source: &gear_rpc_client::GearApi,
    witness: &gear_rpc_client::GearApi,
    ethereum: &Ethereum,
    remoting: &GClientRemoting,
    tokens: &[Token],
    gear_user: ActorId,
    evm_user: Address,
    saved: Option<&SnapshotSet>,
) -> Result<SnapshotSet> {
    let source_finalized = source.latest_finalized_block().await?;
    let gear_hash = saved
        .map(|snapshot| snapshot.gear_hash)
        .unwrap_or(source_finalized);
    let gear_height = source.block_hash_to_number(gear_hash).await?;
    let witness_height = witness
        .block_hash_to_number(witness.latest_finalized_block().await?)
        .await?;
    ensure!(
        source.block_hash_to_number(source_finalized).await? >= gear_height,
        "saved Gear snapshot is ahead of source finality"
    );
    ensure!(
        witness_height >= gear_height,
        "witness is behind the source finalized head"
    );
    ensure!(
        source.block_number_to_hash(gear_height).await? == gear_hash
            && witness.block_number_to_hash(gear_height).await? == gear_hash,
        "source/witness finalized Gear hashes differ"
    );
    let (evm_height, evm_hash) = if let Some(saved) = saved {
        ensure!(
            ethereum
                .api
                .is_finalized_block(saved.evm_height, saved.evm_hash.0.into())
                .await?,
            "saved snapshot is not on the canonical finalized Ethereum ancestry; HOLD"
        );
        (saved.evm_height, saved.evm_hash)
    } else {
        let finalized = ethereum.api.verified_finalized_view().await?;
        (finalized.block_number(), finalized.block_hash())
    };
    let mut assets = BTreeMap::new();
    for token in tokens {
        let evm = CampaignToken::new(token.address, ethereum.api.raw_provider().clone());
        let gear = Vft::new(remoting.clone());
        let gear_user = gear
            .balance_of(gear_user)
            .at_block(sails_block_hash(gear_hash))
            .recv(token.peer)
            .await?;
        let gear_supply = gear
            .total_supply()
            .at_block(sails_block_hash(gear_hash))
            .recv(token.peer)
            .await?;
        let gear_escrow = match token.escrow {
            Some(manager) => Some(
                gear.balance_of(manager)
                    .at_block(sails_block_hash(gear_hash))
                    .recv(token.peer)
                    .await?,
            ),
            None => None,
        };
        let evm_user = evm
            .balanceOf(evm_user)
            .block(BlockId::hash_canonical(evm_hash))
            .call()
            .await?;
        let evm_escrow = evm
            .balanceOf(ethereum.receiver_address().into())
            .block(BlockId::hash_canonical(evm_hash))
            .call()
            .await?;
        let evm_supply = evm
            .totalSupply()
            .block(BlockId::hash_canonical(evm_hash))
            .call()
            .await?;
        assets.insert(
            token.symbol.to_owned(),
            RawSnapshot {
                evm_user,
                evm_escrow,
                evm_supply,
                gear_user,
                gear_supply,
                gear_escrow,
            },
        );
    }
    let observed = SnapshotSet {
        gear_height,
        gear_hash,
        evm_height,
        evm_hash,
        assets,
    };
    if let Some(saved) = saved {
        ensure!(
            observed == *saved,
            "saved balance checkpoint changed at its pinned canonical blocks; HOLD"
        );
    }
    Ok(observed)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn write_read_only_snapshot(
    source_rpc: &str,
    witness_rpc: &str,
    ethereum_rpc: &str,
    beacon_rpc: &str,
    deployment_path: &Path,
    token_stack_path: &Path,
    gear_user: &str,
    evm_user: &str,
    output: &Path,
) -> Result<()> {
    ensure!(
        !output.exists(),
        "refusing to overwrite an original pinned snapshot"
    );
    let deployment: Value = serde_json::from_slice(&fs::read(deployment_path)?)?;
    let stack: Value = serde_json::from_slice(&fs::read(token_stack_path)?)?;
    ensure!(
        deployment["mode"] == "hoodi-token-stack" && stack["lane"] == "beefy-token-hoodi",
        "snapshot manifests are not the isolated Hoodi token lane"
    );
    let (source, witness) = Source::connect_pair(
        gear_rpc_client::GearApi::new(source_rpc, 3).await?,
        gear_rpc_client::GearApi::new(witness_rpc, 3).await?,
    )
    .await?;
    source.validate_attachment(&deployment["anchor"]).await?;
    ensure!(
        source.source_genesis == witness.source_genesis
            && source.bridge_domain == witness.bridge_domain
            && source.source_genesis
                == bytes32(string(
                    &deployment["anchor"]["sourceGenesis"],
                    "snapshot source genesis"
                )?)?
            && source.bridge_domain
                == bytes32(string(
                    &deployment["anchor"]["bridgeDomain"],
                    "snapshot bridge domain"
                )?)?
            && deployment["anchor"]["sourceGenesis"] == stack["sourceGenesis"]
            && deployment["ethereum"]["sourceGenesis"] == stack["sourceGenesis"]
            && deployment["ethereum"]["bridgeDomain"] == deployment["anchor"]["bridgeDomain"],
        "snapshot source/witness/manifests disagree on lane identity"
    );
    let ethereum = Ethereum::connect_hoodi_readonly(ethereum_rpc, &deployment["ethereum"]).await?;
    let parent = output.parent().context("snapshot output has no parent")?;
    ethereum
        .api
        .enable_finality_archive(
            &parent.join("snapshot-finality-history/headers"),
            HOODI_EXECUTION_GENESIS.parse()?,
        )
        .await?;
    ensure!(
        ethereum.receiver_address() == campaign_manager_address(&deployment)?,
        "snapshot ERC20 manager differs from immutable deployment"
    );
    let beacon = BeaconClient::new(beacon_rpc.to_owned(), Some(Duration::from_secs(15))).await?;
    verify_beacon_identity(&beacon, &ethereum).await?;
    // This public development origin is only used by read-state queries; no protected key or send path is opened.
    let api = gclient::GearApi::builder()
        .suri("//Alice")
        .uri(source_rpc)
        .build()
        .await?;
    let remoting = GClientRemoting::new(api);
    let manager_id = actor_id(string(
        &stack["programs"]["vftManager"]["id"],
        "snapshot VFT manager",
    )?)?;
    let gear_hash = source.api.latest_finalized_block().await?;
    let mappings = VftManager::new(remoting.clone())
        .vara_to_eth_addresses()
        .at_block(sails_block_hash(gear_hash))
        .recv(manager_id)
        .await?;
    let mut tokens = Vec::with_capacity(4);
    for (symbol, component) in [
        ("USDC", "circleVft"),
        ("USDT", "tetherVft"),
        ("WETH", "etherVft"),
        ("WBTC", "bitcoinVft"),
    ] {
        let peer = actor_id(string(
            &stack["programs"][component]["id"],
            "snapshot token peer",
        )?)?;
        let mut candidates = mappings
            .iter()
            .filter(|(mapped, _, supply)| *mapped == peer && *supply == TokenSupply::Ethereum);
        let (_, address, _) = candidates
            .next()
            .context("snapshot Ethereum-origin token mapping missing")?;
        ensure!(
            candidates.next().is_none(),
            "snapshot token mapping is ambiguous"
        );
        tokens.push(Token {
            symbol,
            component,
            peer,
            address: Address::from_slice(address.as_bytes()),
            gear_origin: false,
            native_amount: None,
            escrow: None,
        });
    }
    if !stack["programs"]["gearOriginVft"].is_null() || !stack["programs"]["nativeVft"].is_null() {
        ensure!(
            mappings.len() == 6,
            "fresh six-asset snapshot mappings are incomplete; HOLD"
        );
        let deposit = source
            .api
            .api
            .constants()
            .at(&gsdk::ext::subxt::dynamic::constant(
                "Balances",
                "ExistentialDeposit",
            ))?
            .as_type::<u128>()?;
        let witnessed = witness
            .api
            .api
            .constants()
            .at(&gsdk::ext::subxt::dynamic::constant(
                "Balances",
                "ExistentialDeposit",
            ))?
            .as_type::<u128>()?;
        ensure!(
            deposit == witnessed && deposit == 1_000_000_000_000,
            "native snapshot denomination differs from authenticated runtime"
        );
        let wrapper = VftManager::new(remoting.clone())
            .native_wrapper()
            .at_block(sails_block_hash(gear_hash))
            .recv(manager_id)
            .await?;
        for (symbol, component, native) in [
            ("GOT", "gearOriginVft", false),
            ("WTVARA", "nativeVft", true),
        ] {
            let peer = actor_id(string(
                &stack["programs"][component]["id"],
                "snapshot Gear-origin peer",
            )?)?;
            let mut candidates = mappings
                .iter()
                .filter(|(mapped, _, supply)| *mapped == peer && *supply == TokenSupply::Gear);
            let (_, address, _) = candidates
                .next()
                .context("snapshot Gear-origin mapping missing")?;
            ensure!(
                candidates.next().is_none()
                    && !tokens.iter().any(|token| token.peer == peer
                        || token.address.as_slice() == address.as_bytes()),
                "snapshot token identity ambiguous"
            );
            if native {
                ensure!(
                    wrapper == Some(peer)
                        && vft_vara_client::NativeEscrow::new(remoting.clone())
                            .manager()
                            .at_block(sails_block_hash(gear_hash))
                            .recv(peer)
                            .await?
                            == Some(manager_id),
                    "snapshot native binding absent; HOLD"
                );
            } else {
                ensure!(
                    wrapper != Some(peer),
                    "snapshot ordinary Gear token selected as native"
                );
            }
            tokens.push(Token {
                symbol,
                component,
                peer,
                address: Address::from_slice(address.as_bytes()),
                gear_origin: true,
                native_amount: native.then_some(u64::try_from(deposit)?),
                escrow: Some(manager_id),
            });
        }
    }

    let snapshot = acquire_snapshot(
        &source.api,
        &witness.api,
        &ethereum,
        &remoting,
        &tokens,
        actor_id(gear_user)?,
        evm_user.parse()?,
        None,
    )
    .await?;
    atomic_json(output, &snapshot.to_json())
}

fn contained_evidence_path(directory: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    ensure!(
        !relative.is_absolute()
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "bootstrap evidence path escapes its owner; HOLD"
    );
    let directory = fs::canonicalize(directory)?;
    let path = fs::canonicalize(directory.join(relative))?;
    ensure!(
        path.starts_with(&directory),
        "bootstrap evidence symlink escapes its owner; HOLD"
    );
    Ok(path)
}

async fn require_queue_bootstrap(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    deadline: Instant,
) -> Result<()> {
    let evidence_path = ctx.follower_dir.join("queue-bootstrap.json");
    let seed: Value = serde_json::from_slice(&fs::read(&evidence_path).context(
        "canonical finalized non-asset queue bootstrap is missing; HOLD before asset liabilities",
    )?)?;
    let digest = json_digest(&seed)?;
    if let Some(saved) = journal.readiness.get("queueBootstrap") {
        ensure!(
            saved["evidenceDigest"].as_str() == Some(digest.as_str()),
            "original queue bootstrap evidence changed; HOLD"
        );
    }
    let verified = timeout_at(deadline.into(), async {
        ensure!(seed["schemaVersion"] == 1
            && bytes32(string(&seed["sourceGenesis"], "bootstrap source genesis")?)? == ctx.source.source_genesis
            && bytes32(string(&seed["bridgeDomain"], "bootstrap bridge domain")?)? == ctx.source.bridge_domain,
            "bootstrap schema/source identity differs from the pinned lane; HOLD");
        let source_block = u32::try_from(number(&seed["sourceBlock"])?)?;
        let source_hash = GearHash(bytes32(string(&seed["sourceHash"], "bootstrap source hash")?)?);
        let previous = source_block.checked_sub(1).context("bootstrap cannot be at source genesis")?;
        let nonce = GearU256::from_str(string(&seed["message"]["nonce"], "bootstrap nonce")?)?;
        ensure!(nonce.is_zero(), "an earlier source liability exists; nonce-0 bootstrap required; HOLD");
        let message = gear_rpc_client::dto::Message {
            nonce_be: [0; 32],
            source: bytes32(string(&seed["message"]["source"], "bootstrap message source")?)?,
            destination: string(&seed["message"]["destination"], "bootstrap destination")?.parse::<Address>()?.into_array(),
            payload: hex::decode(string(&seed["message"]["payload"], "bootstrap payload")?.trim_start_matches("0x"))?,
        };
        ensure!(message.payload.len() == 33 && message.payload[0] == 0 && message.payload[1..] == message.source,
            "bootstrap is not the same-value non-asset governance update; HOLD");
        let before = SnapshotSet::from_json(&seed["beforeSnapshot"])?;
        let after = SnapshotSet::from_json(&seed["afterSnapshot"])?;
        ctx.snapshot_at(Some(&before)).await?;
        ctx.snapshot_at(Some(&after)).await?;
        verify_roundtrip_delta(&before, &after, &ctx.tokens)?;
        ensure!(before.gear_height < source_block && after.gear_height >= source_block,
            "bootstrap accounting snapshots do not bracket its original source submission; HOLD");
        for token in &ctx.tokens {
            let state = before.assets.get(token.symbol).context("bootstrap asset missing")?;
            ensure!(state.has_no_bridge_liabilities(),
                "{} already has bridged token liabilities before bootstrap; HOLD", token.symbol);
        }
        let submission = &seed["sourceSubmission"];
        let extrinsic_hash = bytes32(string(&submission["extrinsicHash"], "bootstrap extrinsic hash")?)?;
        let extrinsic_index = u32::try_from(number(&submission["extrinsicIndex"])?)?;
        ensure!(number(&submission["block"])? == u64::from(source_block)
            && bytes32(string(&submission["blockHash"], "bootstrap submission block hash")?)? == source_hash.0,
            "bootstrap first snapshot differs from its original source handoff; HOLD");
        for api in [&ctx.source.api, &ctx.witness.api] {
            ensure!(api.block_number_to_hash(source_block).await? == source_hash,
                "bootstrap source inclusion differs across canonical nodes; HOLD");
            let pauser = api.api.constants().at(&gsdk::ext::subxt::dynamic::constant("GearEthBridge", "BridgePauser"))?;
            ensure!(pauser.encoded() == message.source, "bootstrap was not sent by the runtime BridgePauser; HOLD");
            let previous_hash = api.block_number_to_hash(previous).await?;
            let address = gsdk::ext::subxt::dynamic::storage("GearEthBridge", "MessageNonce", Vec::<gsdk::ext::subxt::dynamic::Value>::new());
            let previous_nonce = match api.api.storage().at(previous_hash).fetch(&address).await? {
                Some(value) => {
                    let mut encoded = value.encoded();
                    let nonce = GearU256::decode(&mut encoded)?;
                    ensure!(encoded.is_empty(), "bootstrap predecessor nonce has trailing bytes; HOLD");
                    nonce
                }
                None => GearU256::zero(),
            };
            ensure!(previous_nonce.is_zero(), "source liabilities precede the first bootstrap snapshot; HOLD");
            let raw_block: Value = api.api.rpc().request("chain_getBlock", rpc_params![source_hash]).await?;
            let encoded = hex::decode(string(&raw_block["block"]["extrinsics"][usize::try_from(extrinsic_index)?],
                "bootstrap original extrinsic")?.trim_start_matches("0x"))?;
            ensure!(gsdk::ext::sp_core::blake2_256(&encoded) == extrinsic_hash,
                "bootstrap extrinsic identity changed at its pinned index; HOLD");
            let block = api.api.blocks().at(source_hash).await?;
            let mut sudo_succeeded = false;
            let mut queued = 0;
            for event in block.events().await?.iter() {
                let event = event?;
                if event.phase() != gsdk::ext::subxt::events::Phase::ApplyExtrinsic(extrinsic_index) { continue; }
                if event.pallet_name() == "Sudo" && event.variant_name() == "SudoAsDone" {
                    ensure!(event.field_bytes() == [0], "bootstrap inner sudo_as dispatch failed; HOLD");
                    sudo_succeeded = true;
                }
                if event.pallet_name() == "GearEthBridge" && event.variant_name() == "MessageQueued" {
                    let RuntimeEvent::GearEthBridge(gsdk::gear::runtime_types::pallet_gear_eth_bridge::pallet::Event::MessageQueued {
                        message: observed, ..
                    }) = event.as_gear()? else { bail!("bootstrap queue event could not decode; HOLD"); };
                    ensure!(observed.nonce.0 == [0; 4] && observed.source.0 == message.source
                        && observed.destination.0 == message.destination && observed.payload == message.payload,
                        "bootstrap original queued message changed; HOLD");
                    queued += 1;
                }
            }
            ensure!(sudo_succeeded && queued == 1, "bootstrap has no unique successful sudo_as queue handoff; HOLD");
        }
        let message_hash = relayer::message_relayer::common::message_hash(&message);
        let (queue_id, root) = ctx.source.api.fetch_queue_merkle_root(source_hash).await?;
        ensure!(ctx.witness.api.fetch_queue_merkle_root(source_hash).await? == (queue_id, root)
            && root.0 == message_hash, "first bootstrap snapshot includes another or unauthenticated liability; HOLD");
        let inclusion = ctx.source.api.fetch_message_inclusion_merkle_proof(source_hash, GearHash(message_hash)).await?;
        ensure!(inclusion.root == message_hash && inclusion.num_leaves == 1 && inclusion.leaf_index == 0,
            "bootstrap was not the first and only message in its authenticated snapshot; HOLD");
        let publication_path = contained_evidence_path(&ctx.follower_dir,
            string(&seed["rootPublication"]["path"], "bootstrap publication path")?)?;
        let publication: Value = serde_json::from_slice(&fs::read(&publication_path)?)?;
        ensure!(publication["acceptedAnchorTx"] == seed["acceptedAnchorTx"]
            && publication["txHash"] == seed["rootPublication"]["txHash"],
            "bootstrap publication or original BEEFY anchor identity changed; HOLD");
        let publication_observed = wait_actor_root(ctx, &QueueRoot {
            source_block, block_hash: source_hash, queue_id, root: message_hash,
        }, deadline).await?;
        ensure!(fs::canonicalize(string(&publication_observed["publication"], "verified publication path")?)? == publication_path,
            "bootstrap did not use the independent owner's original publication; HOLD");
        let process_tx: B256 = string(&seed["processTxHash"], "bootstrap worker process tx")?.parse()?;
        let worker_uuid = string(&seed["outboundTransactionUuid"], "bootstrap paid worker UUID")?;
        ensure!(Path::new(worker_uuid).components().count() == 1 && !Path::new(worker_uuid).is_absolute(),
            "bootstrap worker UUID escapes the existing journal; HOLD");
        let worker: relayer::message_relayer::gear_to_eth::tx_manager::Transaction = serde_json::from_slice(
            &fs::read(contained_evidence_path(&ctx.outbound_dir, worker_uuid)?)?)?;
        let events: Value = read_worker_json(&ctx.outbound_dir.join("gear_events.json"))?
            .context("bootstrap paid-worker lane journal is missing; HOLD")?;
        ensure!(events["version"] == 2,
            "bootstrap worker lacks a schema-2 fee-exemption policy; HOLD");
        let lane: relayer::message_relayer::gear_to_eth::storage::OutboundLaneIdentity =
            serde_json::from_value(events["lane_identity"].clone()).context("bootstrap worker lane identity is missing; HOLD")?;
        ensure!(lane.destination_chain_id == HOODI_CHAIN_ID
            && lane.destination_genesis_hash.0 == bytes32("bbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b")?
            && lane.message_queue_address.as_bytes() == ctx.ethereum.queue_address.as_slice()
            && lane.bridging_payment_address == Some(GearHash(ctx.payment_id.into_bytes())),
            "bootstrap worker belongs to another destination/queue/payment lane; HOLD");
        let prepared = worker.ethereum_submission.as_ref().context("bootstrap has no original signed paid-worker handoff; HOLD")?;
        let identity = ethereum_client::prepared_content_message_identity(prepared)?;
        worker.validate_journal()?;
        ensure!(worker.journal_version == 3 && worker.uuid.to_string() == worker_uuid
            && matches!(worker.status, relayer::message_relayer::gear_to_eth::tx_manager::TxStatus::Completed)
            && worker.message.message == message && worker.message.block.0 == source_block
            && worker.message.block_hash.as_bytes() == source_hash.as_bytes() && worker.message_hash == message_hash
            && worker.ethereum_tx_hash == Some(process_tx) && worker.ethereum_tx_attempts.as_slice() == [process_tx]
            && prepared.hash == process_tx && prepared.chain_id == HOODI_CHAIN_ID
            && prepared.contract == ctx.ethereum.queue_address && identity.from != ctx.campaign_address
            && identity.from != ctx.follower_address && identity.from != ctx.publisher_address
            && identity.from.as_slice() == lane.sender_address.as_bytes(),
            "bootstrap delivery is not the independent worker's immutable original nonce-0 submission; HOLD");
        use alloy::{consensus::{Transaction as _, TxEnvelope}, eips::Decodable2718, sol_types::SolCall};
        let transaction = TxEnvelope::decode_2718(&mut prepared.raw_transaction.as_slice())?;
        let call = ethereum_client::abi::IMessageQueue::processMessageCall::abi_decode(transaction.input())?;
        ensure!(call.abi_encode() == transaction.input().as_ref()
            && call.blockNumber == EthU256::from(source_block) && call.totalLeaves == EthU256::from(1)
            && call.leafIndex.is_zero() && call.message.nonce.is_zero() && call.message.source.0 == message.source
            && call.message.destination.as_slice() == message.destination && call.message.payload.as_ref() == message.payload
            && call.proof.is_empty(), "original signed bootstrap calldata/proof changed; HOLD");
        let process = wait_receipt_finalized(ctx, process_tx, deadline).await?;
        let process_height = number(&process["receiptBlock"])?;
        ensure!(before.evm_height < number(&publication_observed["receiptBlock"])? && after.evm_height >= process_height,
            "bootstrap accounting snapshots do not bracket canonical publication/delivery; HOLD");
        let provider = ctx.ethereum.api.raw_provider();
        let receipt = provider.get_transaction_receipt(process_tx).await?.context("bootstrap delivery receipt missing")?;
        ensure!(receipt.transaction_hash == process_tx && receipt.status()
            && receipt.block_number == Some(process_height)
            && receipt.block_hash == Some(string(&process["receiptBlockHash"], "bootstrap receipt hash")?.parse()?)
            && receipt.from == identity.from && receipt.to == Some(ctx.ethereum.queue_address),
            "bootstrap receipt changed original inclusion/worker signer/queue; HOLD");
        let current_view = ctx.ethereum.api.verified_finalized_view().await?;
        let at = current_view.block_id();
        let queue = ethereum_client::abi::IMessageQueue::new(ctx.ethereum.queue_address, provider.clone());
        let pauser = queue.governancePauser().block(at).call().await?;
        ensure!(pauser.as_slice() == message.destination
            && pauser == parse_actor_address(&ctx.deployment["ethereumConfiguration"]["governancePauser"], "deployed governance pauser")?
            && queue.genesisBlock().block(at).call().await? == EthU256::from(source_block)
            && queue.getMerkleRoot(EthU256::from(source_block)).block(at).call().await? == B256::from(message_hash)
            && queue.isProcessed(EthU256::ZERO).block(at).call().await?,
            "canonical finalized bootstrap root/floor/delivery differs; HOLD before asset liabilities");
        let pauser_contract = CampaignPauser::new(pauser, provider.clone());
        for hash in [before.evm_hash, after.evm_hash, current_view.block_hash()] {
            let at = BlockId::hash_canonical(hash);
            ensure!(pauser_contract.governance().block(at).call().await?.0 == message.source
                && pauser_contract.messageQueue().block(at).call().await? == ctx.ethereum.queue_address,
                "bootstrap changed governance or targets another queue; HOLD");
        }
        let registered = queue.getMerkleRootTimestampForBlock(EthU256::from(source_block)).block(at).call().await?;
        let delay = queue.PROCESS_PAUSER_MESSAGE_DELAY().block(at).call().await?;
        let eligible = registered.checked_add(delay).context("bootstrap maturity timestamp overflow; HOLD")?;
        ensure!(!registered.is_zero() && delay == EthU256::from(300)
            && EthU256::from(number(&process["blockTimestampMs"])? / 1_000) >= eligible,
            "bootstrap did not preserve natural pauser maturity; HOLD");
        let processed_event = B256::from(keccak256(b"MessageProcessed(uint256,bytes32,uint256,address)"));
        let expected_words = [EthU256::from(source_block).to_be_bytes::<32>(), message_hash, [0; 32], address_word(pauser)];
        ensure!(receipt.as_ref().logs().iter().any(|log| log.address() == ctx.ethereum.queue_address
            && log.topics() == [processed_event] && log.data().data.len() == 128
            && log.data().data.chunks_exact(32).zip(expected_words).all(|(word, expected)| word == expected.as_slice())),
            "original finalized bootstrap receipt has no matching MessageProcessed event; HOLD");
        Ok::<_, anyhow::Error>(json!({"evidencePath":evidence_path,"evidenceDigest":digest,
            "sourceBlock":source_block,"sourceHash":format!("{source_hash:#x}"),"queueRoot":format!("0x{}",hex::encode(message_hash)),
            "publication":publication_observed,"processReceipt":process,"outboundTransactionUuid":worker_uuid,
            "beforeSnapshot":before.to_json(),"afterSnapshot":after.to_json(),"status":"passed"}))
    }).await.context("bootstrap readiness exceeded its original campaign deadline; HOLD")??;
    journal.readiness.insert("queueBootstrap".into(), verified);
    save_journal(path, journal)
}

async fn lanes_ready(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    phase: &str,
    minimum_source: u32,
    deadline: Instant,
    allowed_queued: &BTreeMap<String, OutboundRequest>,
) -> Result<()> {
    require_queue_bootstrap(ctx, journal, path, deadline).await?;
    timeout_at(deadline.into(), async {
        loop {
            follower_ready(ctx, minimum_source, deadline).await?;
            let mut observation = ctx.worker_readiness(allowed_queued).await?;
            let ready = observation["ready"] == true;
            observation["observedAtMs"] = json!(now_ms()?);
            journal.readiness.insert(phase.to_owned(), observation);
            save_journal(path, journal)?;
            if ready {
                return Ok(());
            }
            sleep(Duration::from_secs(5)).await;
        }
    })
    .await
    .map_err(|_| {
        anyhow!("{phase} actor/checkpoint/worker readiness exceeded its original deadline")
    })?
}

fn warmup_sample_deadline(start: Instant, minute: u64, now: Instant) -> Result<Instant> {
    let end = start + Duration::from_secs((minute + 1) * 60);
    ensure!(
        now < end,
        "warmup minute {minute} was missed; samples cannot be backfilled"
    );
    Ok(end)
}

fn checkpoint_observation_stable(
    before: &DestinationCheckpoint,
    after: &DestinationCheckpoint,
) -> Result<bool> {
    ensure!(
        after.block >= before.block,
        "active BEEFY checkpoint regressed; HOLD"
    );
    if after.block == before.block {
        ensure!(
            after == before,
            "same-height BEEFY checkpoint changed hash/root/authorities; HOLD"
        );
        return Ok(true);
    }
    ensure!(
        after.source_timestamp_ms >= before.source_timestamp_ms
            && after.current_id >= before.current_id
            && after.next_id >= before.next_id,
        "advancing BEEFY checkpoint regressed freshness/authorities; HOLD"
    );
    Ok(false)
}

async fn coherent_follower_observation(
    ctx: &Context,
    deadline: Instant,
) -> Result<(Value, DestinationCheckpoint, Option<MinedFollowerHead>)> {
    timeout_at(deadline.into(), async {
        loop {
            ensure!(
                Instant::now() < deadline,
                "coherent follower observation exceeded original slot; HOLD"
            );
            let before = ctx.ethereum.checkpoint().await?;
            let state = ctx
                .follower_state()?
                .context("token follower state is missing")?;
            let checkpoint = ctx.ethereum.checkpoint().await?;
            if !checkpoint_observation_stable(&before, &checkpoint)? {
                continue;
            }
            if state["follower"]["lastMinedUpdate"]
                .as_u64()
                .is_some_and(|block| block != checkpoint.block)
            {
                sleep_until(deadline.min(Instant::now() + Duration::from_secs(3)).into()).await;
                continue;
            }
            let mined = verified_mined_follower_head(ctx, &state, &checkpoint).await?;
            if mined.is_none() && state["follower"]["lastMinedUpdate"].is_u64() {
                // Only the follower may authenticate and journal a recovered inclusion.
                sleep_until(deadline.min(Instant::now() + Duration::from_secs(3)).into()).await;
                continue;
            }
            let after = ctx.ethereum.checkpoint().await?;
            if checkpoint_observation_stable(&checkpoint, &after)? {
                return Ok((state, checkpoint, mined));
            }
        }
    })
    .await
    .context("coherent follower observation exceeded original slot; HOLD")?
}

fn validate_warmup_samples(samples: &[Value], started: u64, required_minutes: u64) -> Result<()> {
    ensure!(
        matches!(required_minutes, 60 | 300) && samples.len() == usize::try_from(required_minutes)?,
        "warmup requires every original sealed minute, without backfill"
    );
    for (minute, sample) in samples.iter().enumerate() {
        let scheduled = started
            .checked_add(u64::try_from(minute)? * 60_000)
            .context("warmup sample timestamp overflow")?;
        let at = number(&sample["atMs"])?;
        ensure!(
            sample["minute"].as_u64() == Some(u64::try_from(minute)?)
                && sample["scheduledAtMs"].as_u64() == Some(scheduled)
                && at >= scheduled
                && at < scheduled + 60_000,
            "warmup sample shifted, duplicated or missed its original minute; HOLD"
        );
        let workers = &sample["workers"];
        let observed = number(&workers["observedAtMs"])?;
        ensure!(
            workers["ready"] == true && observed >= scheduled && observed <= at,
            "warmup has no healthy finalized worker/checkpoint evidence in this minute; HOLD"
        );
    }
    ensure!(
        number(&samples[samples.len() - 1]["sourceHeight"])? > number(&samples[0]["sourceHeight"])?,
        "warmup cannot qualify observations of a stalled finalized source; HOLD"
    );
    Ok(())
}

fn ensure_actor_history_preserved(before: &Value, after: &Value) -> Result<()> {
    if let Some(before_cursor) = before["rootScan"]["block"].as_u64() {
        let after_cursor = after["rootScan"]["block"]
            .as_u64()
            .context("restarted follower root-scan cursor is missing")?;
        ensure!(
            after_cursor >= before_cursor,
            "follower root-scan cursor regressed"
        );
        if after_cursor == before_cursor {
            ensure!(
                after["rootScan"]["blockHash"] == before["rootScan"]["blockHash"],
                "follower root-scan cursor changed canonical hash"
            );
        }
    }
    let before_commitments = before["commitments"]
        .as_array()
        .context("follower commitment history is missing")?;
    let after_commitments = after["commitments"]
        .as_array()
        .context("restarted follower commitment history is missing")?;
    ensure!(
        after_commitments.len() >= before_commitments.len(),
        "follower commitment history was lost during restart"
    );
    for (old, new) in before_commitments.iter().zip(after_commitments) {
        for field in [
            "block",
            "blockHash",
            "rawScale",
            "current",
            "next",
            "freshnessProof",
            "destinationBlock",
            "destinationHash",
            "txHash",
            "submission",
        ] {
            ensure!(
                old[field] == new[field],
                "follower commitment history changed at {field}"
            );
        }
        ensure!(
            old["finalized"].as_bool().is_some()
                && new["finalized"].as_bool().is_some()
                && (old["finalized"] != true || new["finalized"] == true),
            "finalized commitment regressed during restart"
        );
    }
    for field in [
        "lastMinedUpdate",
        "lastSuccessfulUpdate",
        "lastFinalizedUpdate",
    ] {
        if let Some(previous) = before["follower"][field].as_u64() {
            ensure!(
                after["follower"][field]
                    .as_u64()
                    .is_some_and(|next| next >= previous),
                "follower {field} regressed during restart"
            );
        }
    }
    let before_roots = before["roots"]
        .as_object()
        .context("follower root registration history is missing")?;
    let after_roots = after["roots"]
        .as_object()
        .context("restarted follower root registration history is missing")?;
    for (key, old) in before_roots {
        let new = after_roots
            .get(key)
            .with_context(|| format!("follower root registration {key} was lost during restart"))?;
        for field in ["block", "blockHash", "queueId", "queueRoot", "publication"] {
            ensure!(
                old[field] == new[field],
                "follower root registration {key} changed at {field}"
            );
        }
        for field in [
            "nonce",
            "txHash",
            "rawTransaction",
            "acceptedAnchorTx",
            "acceptedAnchorClient",
            "acceptedCheckpoint",
        ] {
            if !old[field].is_null() {
                ensure!(
                    old[field] == new[field],
                    "follower root registration {key} changed signed {field}"
                );
            }
        }
        ensure!(
            matches!(
                (old["status"].as_str(), new["status"].as_str()),
                (Some("pending"), Some("pending" | "mined" | "accepted"))
                    | (Some("mined"), Some("mined" | "accepted"))
                    | (Some("accepted"), Some("accepted"))
            ),
            "follower root registration {key} regressed during restart"
        );
    }
    Ok(())
}

// The follower's mined head is capacity evidence, never settlement or root acceptance.
#[derive(Clone, Copy)]
struct MinedFollowerHead {
    source_block: u64,
    source_hash: [u8; 32],
    destination_block: u64,
    destination_hash: B256,
    tx_hash: B256,
}

fn observed_mined_inclusion(
    mined: MinedFollowerHead,
    finalized: bool,
    inclusion: (u64, B256),
) -> Result<Option<MinedFollowerHead>> {
    if inclusion != (mined.destination_block, mined.destination_hash) {
        ensure!(
            !finalized,
            "finalized mined commitment inclusion changed; HOLD"
        );
        return Ok(None);
    }
    Ok(Some(mined))
}

fn mined_follower_entry<'a>(
    state: &'a Value,
    client: Address,
    checkpoint: &DestinationCheckpoint,
) -> Result<Option<(&'a Value, MinedFollowerHead)>> {
    let recorded: Address = string(
        &state["activeEthereum"]["client"],
        "follower active BEEFY client",
    )?
    .parse()?;
    ensure!(
        recorded == client,
        "follower active BEEFY client differs from the campaign"
    );
    let Some(mined_block) = state["follower"]["lastMinedUpdate"].as_u64() else {
        return Ok(None);
    };
    if mined_block < checkpoint.block {
        return Ok(None);
    }
    ensure!(
        mined_block == checkpoint.block,
        "follower mined head is ahead of the active BEEFY client; refusing a stale or reorged anchor"
    );
    let entry = state["commitments"]
        .as_array()
        .context("follower commitment journal is missing")?
        .iter()
        .rev()
        .find(|entry| {
            entry["block"].as_u64() == Some(mined_block)
                && entry["clientAddress"]
                    .as_str()
                    .is_some_and(|address| address.parse::<Address>().ok() == Some(client))
        })
        .context("follower mined head has no commitment for the active client")?;
    ensure!(
        entry["finalized"].is_boolean(),
        "mined commitment has no finality status"
    );
    let source_hash = bytes32(string(&entry["blockHash"], "mined source block hash")?)?;
    let destination_block = number(&entry["destinationBlock"])?;
    let destination_hash: B256 =
        string(&entry["destinationHash"], "mined inclusion hash")?.parse()?;
    let tx_hash: B256 = string(&entry["txHash"], "mined transaction hash")?.parse()?;
    let submission = &entry["submission"];
    ensure!(
        submission["block"] == entry["block"]
            && submission["blockHash"] == entry["blockHash"]
            && submission["txHash"] == entry["txHash"]
            && submission["nonce"].as_u64().is_some()
            && submission["clientAddress"]
                .as_str()
                .is_some_and(|address| address.parse::<Address>().ok() == Some(client)),
        "mined commitment changed its original signed submission identity"
    );
    let raw = hex::decode(
        string(
            &submission["rawTransaction"],
            "original signed commitment transaction",
        )?
        .strip_prefix("0x")
        .context("signed commitment transaction is not 0x-prefixed")?,
    )?;
    ensure!(
        !raw.is_empty() && B256::from(keccak256(&raw)) == tx_hash,
        "mined commitment transaction hash differs from its original signed bytes"
    );
    Ok(Some((
        entry,
        MinedFollowerHead {
            source_block: mined_block,
            source_hash,
            destination_block,
            destination_hash,
            tx_hash,
        },
    )))
}

async fn verified_mined_follower_head(
    ctx: &Context,
    state: &Value,
    checkpoint: &DestinationCheckpoint,
) -> Result<Option<MinedFollowerHead>> {
    let Some((entry, mined)) =
        mined_follower_entry(state, ctx.ethereum.client_address, checkpoint)?
    else {
        return Ok(None);
    };
    let block: u32 = mined.source_block.try_into()?;
    let raw = hex::decode(
        string(&entry["rawScale"], "signed commitment SCALE bytes")?
            .strip_prefix("0x")
            .context("signed commitment SCALE is not 0x-prefixed")?,
    )?;
    let signed = ctx.source.recapture(block, raw.clone()).await?;
    let witnessed = ctx.witness.recapture(block, raw).await?;
    crate::hoodi::require_saved_submission(
        &entry["submission"],
        &signed,
        mined.tx_hash.into(),
        ctx.ethereum.client_address,
        ctx.ethereum.client_address,
    )?;
    ensure!(
        signed.block_hash == mined.source_hash
            && witnessed.block_hash == signed.block_hash
            && witnessed.validated.commitment_hash == signed.validated.commitment_hash
            && witnessed.current == signed.current
            && witnessed.next == signed.next
            && entry["current"] == serde_json::to_value(&signed.current)?
            && entry["next"] == serde_json::to_value(&signed.next)?
            && bytes32(string(&entry["submission"]["root"], "submitted MMR root")?)?
                == signed.validated.mmr_root,
        "mined commitment is not canonical and independently signed on both source nodes"
    );
    let source_block = block
        .checked_sub(1)
        .context("mined anchor has no preceding MMR leaf")?;
    let (proof, witness_proof) = tokio::try_join!(
        ctx.source.proof(source_block, &signed),
        ctx.witness.proof(source_block, &witnessed),
    )?;
    ensure!(
        proof.snapshot == witness_proof.snapshot
            && proof.source_hash == witness_proof.source_hash
            && proof.raw_leaf == witness_proof.raw_leaf
            && entry["freshnessProof"] == crate::rehearsal::freshness_evidence(&proof)?,
        "mined commitment freshness proof differs from independently witnessed source history"
    );
    ensure!(
        checkpoint.block == mined.source_block
            && checkpoint.root == signed.validated.mmr_root
            && checkpoint.source_timestamp_ms == proof.snapshot.source_timestamp_ms
            && state["follower"]["freshnessDeadlineMs"].as_u64()
                == checkpoint.source_timestamp_ms.checked_add(86_400_000),
        "active BEEFY checkpoint or freshness differs from the mined source commitment"
    );
    let inclusion = ctx
        .ethereum
        .verify_accepted_commitment(mined.tx_hash, &signed, &proof.snapshot)
        .await?;
    observed_mined_inclusion(mined, entry["finalized"] == true, inclusion)
}
fn follower_checkpoint(state: &Value) -> Result<u64> {
    let active_client = state["activeEthereum"]["client"]
        .as_str()
        .or_else(|| state["deployment"]["ethereum"]["client"].as_str());
    let accepted = state["commitments"]
        .as_array()
        .and_then(|values| {
            values.iter().rev().find(|value| {
                value["finalized"] == true
                    && active_client
                        .is_some_and(|client| value["clientAddress"].as_str() == Some(client))
            })
        })
        .and_then(|value| value["block"].as_u64());
    let bootstrap = (active_client == state["deployment"]["ethereum"]["client"].as_str())
        .then(|| state["bootstrap"]["block"].as_u64())
        .flatten();
    accepted
        .or(bootstrap)
        .context("follower journal has no finalized checkpoint for its active BEEFY client")
}

fn verify_mint_delta(
    base: &SnapshotSet,
    after: &SnapshotSet,
    tokens: &[Token],
    evm_amount: EthU256,
    gear_amount: GearU256,
) -> Result<()> {
    for token in tokens {
        let before = base
            .assets
            .get(token.symbol)
            .context("baseline asset missing")?;
        let minted = after
            .assets
            .get(token.symbol)
            .context("mint asset missing")?;
        if token.gear_origin {
            ensure!(
                before == minted,
                "Gear-origin inventory changed before its first Gear-to-Ethereum leg; HOLD"
            );
            continue;
        }
        ensure!(
            before.evm_user.checked_sub(evm_amount) == Some(minted.evm_user),
            "{} EVM user balance did not fall by exact lock amount",
            token.symbol
        );
        ensure!(
            before.evm_escrow.checked_add(evm_amount) == Some(minted.evm_escrow),
            "{} manager escrow did not rise by exact lock amount",
            token.symbol
        );
        ensure!(
            before.evm_supply == minted.evm_supply,
            "{} ERC20 supply changed during lock/mint",
            token.symbol
        );
        ensure!(
            before.gear_user.checked_add(gear_amount) == Some(minted.gear_user),
            "{} Gear user balance did not rise by exact mint amount",
            token.symbol
        );
        ensure!(
            before.gear_supply.checked_add(gear_amount) == Some(minted.gear_supply),
            "{} Gear VFT supply did not rise by exact mint amount",
            token.symbol
        );
    }
    Ok(())
}

fn verify_gear_export_delta(
    base: &SnapshotSet,
    after: &SnapshotSet,
    tokens: &[Token],
    amount: u64,
) -> Result<()> {
    for token in tokens {
        let before = base
            .assets
            .get(token.symbol)
            .context("export baseline asset missing")?;
        let exported = after
            .assets
            .get(token.symbol)
            .context("export asset missing")?;
        if !token.gear_origin {
            ensure!(
                before == exported,
                "Ethereum-origin roundtrip changed its original baseline"
            );
            continue;
        }
        let raw = token.raw_amount(amount)?;
        ensure!(
            before.evm_user.checked_add(EthU256::from(raw)) == Some(exported.evm_user)
                && before.evm_supply.checked_add(EthU256::from(raw)) == Some(exported.evm_supply)
                && before.evm_escrow == exported.evm_escrow
                && before.gear_user.checked_sub(GearU256::from(raw)) == Some(exported.gear_user)
                && before.gear_supply == exported.gear_supply
                && before
                    .gear_escrow
                    .context("Gear-origin escrow baseline missing")?
                    .checked_add(GearU256::from(raw))
                    == exported.gear_escrow,
            "{} source escrow/EVM mint did not settle exactly; HOLD",
            token.symbol
        );
    }
    Ok(())
}

fn verify_settlement_delta(
    base: &SnapshotSet,
    after: &SnapshotSet,
    tokens: &[Token],
    amount: u64,
) -> Result<()> {
    for token in tokens {
        let before = base
            .assets
            .get(token.symbol)
            .context("settlement baseline asset missing")?;
        let settled = after
            .assets
            .get(token.symbol)
            .context("settlement asset missing")?;
        let mut expected = before.clone();
        if token.native_amount.is_some() {
            let raw = GearU256::from(token.raw_amount(amount)?);
            expected.gear_user = expected
                .gear_user
                .checked_sub(raw)
                .context("native wrapper debit underflow")?;
            expected.gear_supply = expected
                .gear_supply
                .checked_sub(raw)
                .context("native wrapper burn underflow")?;
        }
        ensure!(
            settled == &expected,
            "{} ordinary escrow/native redemption settlement differs; HOLD",
            token.symbol
        );
    }
    Ok(())
}

fn verify_roundtrip_delta(base: &SnapshotSet, after: &SnapshotSet, tokens: &[Token]) -> Result<()> {
    for token in tokens {
        ensure!(
            base.assets
                .get(token.symbol)
                .is_some_and(|before| after.assets.get(token.symbol) == Some(before)),
            "{} finalized balances/supplies/manager escrow changed",
            token.symbol
        );
    }
    Ok(())
}

impl SnapshotSet {
    fn to_json(&self) -> Value {
        json!({
            "gearHeight":self.gear_height,"gearHash":format!("{:#x}",self.gear_hash),
            "evmHeight":self.evm_height,"evmHash":format!("{:#x}",self.evm_hash),
            "assets":self.assets.iter().map(|(symbol,state)|(symbol.clone(),state.to_json())).collect::<BTreeMap<_,_>>(),
        })
    }

    fn from_json(value: &Value) -> Result<Self> {
        let assets = value["assets"]
            .as_object()
            .context("saved campaign baseline has no assets")?
            .iter()
            .map(|(symbol, state)| {
                Ok((
                    symbol.clone(),
                    RawSnapshot {
                        gear_escrow: state["gearManagerEscrow"]
                            .as_str()
                            .map(GearU256::from_dec_str)
                            .transpose()?,
                        evm_user: EthU256::from_str(
                            state["evmUser"]
                                .as_str()
                                .context("baseline EVM user balance missing")?,
                        )?,
                        evm_escrow: EthU256::from_str(
                            state["evmManagerEscrow"]
                                .as_str()
                                .context("baseline escrow balance missing")?,
                        )?,
                        evm_supply: EthU256::from_str(
                            state["erc20Supply"]
                                .as_str()
                                .context("baseline ERC20 supply missing")?,
                        )?,
                        gear_user: GearU256::from_dec_str(
                            state["gearUser"]
                                .as_str()
                                .context("baseline Gear user balance missing")?,
                        )?,
                        gear_supply: GearU256::from_dec_str(
                            state["vftSupply"]
                                .as_str()
                                .context("baseline VFT supply missing")?,
                        )?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        ensure!(
            matches!(assets.len(), 4 | 6),
            "saved campaign baseline must contain the complete four- or six-asset manifest"
        );
        Ok(Self {
            gear_height: u32::try_from(
                value["gearHeight"]
                    .as_u64()
                    .context("baseline Gear height missing")?,
            )?,
            gear_hash: GearHash::from_str(
                value["gearHash"]
                    .as_str()
                    .context("baseline Gear hash missing")?,
            )?,
            evm_height: value["evmHeight"]
                .as_u64()
                .context("baseline EVM height missing")?,
            evm_hash: B256::from_str(
                value["evmHash"]
                    .as_str()
                    .context("baseline EVM hash missing")?,
            )?,
            assets,
        })
    }
}

fn record_balance_checkpoint(
    journal: &mut Journal,
    path: &Path,
    window: &str,
    name: &str,
    snapshot: &SnapshotSet,
) -> Result<()> {
    let record = journal
        .windows
        .entry(window.to_owned())
        .or_insert_with(|| json!({"status":"running","assets":{},"stages":{}}));
    if let Some(saved) = record["stages"].get(name) {
        ensure!(
            SnapshotSet::from_json(saved)? == *snapshot,
            "balance checkpoint {name} changed; preserve original evidence"
        );
        return Ok(());
    }
    let mut evidence = snapshot.to_json();
    evidence["observedAtMs"] = json!(now_ms()?);
    record["stages"][name] = evidence;
    save_journal(path, journal)
}

fn prepare_gear_action(
    journal: &mut Journal,
    path: &Path,
    id: &str,
    intent: Value,
) -> Result<bool> {
    if let Some(action) = journal.actions.get(id) {
        ensure!(
            action.intent == intent,
            "Gear action intent changed for {id}"
        );
        return match action.status.as_str() {
            "broadcasting" | "ambiguous" | "submitted" | "reconciled" | "finalized"
            | "processed" | "released" => Ok(false),
            status => bail!("Gear action {id} has non-reconcilable journal status {status:?}"),
        };
    }
    journal.actions.insert(
        id.into(),
        Action {
            status: "broadcasting".into(),
            intent,
            from_block: None,
            nonce: None,
            tx_hash: None,
            evidence: json!({"milestones":{"submissionHandoffAtMs":now_ms()?}}),
        },
    );
    save_journal(path, journal)?;
    Ok(true)
}

fn set_action_evidence(journal: &mut Journal, id: &str, key: &str, value: Value) {
    if let Some(action) = journal.actions.get_mut(id) {
        if !action.evidence.is_object() {
            action.evidence = json!({});
        }
        action.evidence[key] = value;
    }
}

fn record_action_milestone(journal: &mut Journal, id: &str, name: &str, at_ms: u64) -> Result<()> {
    ensure!(at_ms > 0, "operation milestone has no timestamp");
    let action = journal
        .actions
        .get_mut(id)
        .context("milestone action is missing")?;
    let evidence = action
        .evidence
        .as_object_mut()
        .context("action evidence is malformed")?;
    let milestones = evidence
        .entry("milestones")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("action milestones are malformed")?;
    milestones.entry(name).or_insert(json!(at_ms));
    Ok(())
}

fn record_worker_receipt(
    journal: &mut Journal,
    id: &str,
    tx: &InboundTransaction,
    key: (u64, u64),
) -> Result<bool> {
    let Some(receipt) = tx.receipt.as_ref() else {
        return Ok(false);
    };
    ensure!(
        receipt.receipt_key == key,
        "worker receipt key differs from the observed lock proof"
    );
    let proof = json!({"receiptKey":key,"workerTransaction":tx.uuid,
        "payloadKeccak256":format!("0x{}",hex::encode(keccak256(&receipt.payload)))});
    let action = journal
        .actions
        .get_mut(id)
        .context("inbound action is missing")?;
    if let Some(original) = action.evidence.get("workerProof") {
        ensure!(
            *original == proof,
            "worker changed the original receipt proof identity"
        );
    } else {
        action.evidence["workerProof"] = proof;
    }
    action.evidence["workerStatus"] = serde_json::to_value(&tx.status)?;
    action.evidence["workerReceiptObservation"] = serde_json::to_value(&tx.receipt_observation)?;
    if let Some(response) = &receipt.initial_response {
        let value = serde_json::to_value(response)?;
        if let Some(original) = action.evidence.get("initialSubmission") {
            ensure!(
                *original == value,
                "worker changed its first submission response"
            );
        } else {
            action.evidence["initialSubmission"] = value;
        }
        record_action_milestone(
            journal,
            id,
            "gearReplyObservedAtMs",
            response.observed_at_ms,
        )?;
    }
    record_action_milestone(
        journal,
        id,
        "workerProofComposedAtMs",
        receipt.composed_at_ms,
    )?;
    if let Some(at) = receipt.handed_off_at_ms {
        record_action_milestone(journal, id, "workerProofHandoffAtMs", at)?;
    }
    Ok(receipt.handed_off_at_ms.is_some())
}

fn record_incident(journal: &mut Journal, phase: &str, error: &str) -> Result<()> {
    journal
        .incidents
        .push(json!({"atMs":now_ms()? ,"phase":phase,"error":error}));
    Ok(())
}

fn fail_campaign(journal: &mut Journal, path: &Path, error: &str) -> Result<()> {
    journal.status = "failed".into();
    record_incident(journal, "campaign", error)?;
    for value in journal.windows.values_mut() {
        if value["status"] == "running" || value["status"] == "scheduled" {
            value["status"] = json!("failed");
            value["failure"] = json!(error);
        }
    }
    save_journal(path, journal)
}

async fn observe_to_terminal(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    terminal_deadline: Instant,
    clock: ClockAnchor,
) -> Result<()> {
    let mut recorded = BTreeSet::new();
    while Instant::now() < terminal_deadline {
        if let Err(error) = clock.check() {
            let message =
                format!("system clock discontinuity during terminal observation: {error:#}");
            if recorded.insert(message.clone()) {
                if journal.status == "failed" {
                    record_incident(journal, "terminal-observation", &message)?;
                    save_journal(path, journal)?;
                } else {
                    fail_campaign(journal, path, &message)?;
                }
            }
        }
        let actor_issue = match ctx.follower_state() {
            Ok(Some(state)) if state["follower"]["status"] == "healthy" => None,
            Ok(Some(state)) => Some(format!(
                "token follower status is {}",
                state["follower"]["status"].as_str().unwrap_or("invalid")
            )),
            Ok(None) => Some("token follower state is missing".to_owned()),
            Err(error) => Some(format!("cannot validate token follower state: {error:#}")),
        };
        if let Some(issue) = actor_issue {
            let message = format!("actor observation during terminal period: {issue}");
            if recorded.insert(message.clone()) {
                if journal.status == "failed" {
                    record_incident(journal, "terminal-observation", &message)?;
                    save_journal(path, journal)?;
                } else {
                    fail_campaign(journal, path, &message)?;
                }
            }
        }
        match timeout_at(terminal_deadline.into(), observe_sample(ctx, journal, path)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let message = format!("terminal observation failed: {error:#}");
                if recorded.insert(message.clone()) {
                    if journal.status == "failed" {
                        record_incident(journal, "terminal-observation", &message)?;
                        save_journal(path, journal)?;
                    } else {
                        fail_campaign(journal, path, &message)?;
                    }
                }
            }
            Err(_) => break,
        }
        sleep_until(
            (Instant::now() + Duration::from_secs(30))
                .min(terminal_deadline)
                .into(),
        )
        .await;
    }
    journal.status = "failed".into();
    save_journal(path, journal)
}

async fn wait_observing(
    ctx: &mut Context,
    journal: &mut Journal,
    path: &Path,
    until: u64,
    clock: ClockAnchor,
) -> Result<()> {
    while now_ms()? < until {
        clock.check()?;
        observe_sample(ctx, journal, path).await?;
        let wake = (Instant::now() + Duration::from_secs(30)).min(instant_deadline(until)?);
        sleep_until(wake.into()).await;
    }
    Ok(())
}

async fn observe_sample(ctx: &Context, journal: &mut Journal, path: &Path) -> Result<()> {
    let source_hash = ctx.source.api.latest_finalized_block().await?;
    let source_height = ctx.source.api.block_hash_to_number(source_hash).await?;
    let witness_height = ctx
        .witness
        .api
        .block_hash_to_number(ctx.witness.api.latest_finalized_block().await?)
        .await?;
    ensure!(
        witness_height >= source_height
            && ctx.witness.api.block_number_to_hash(source_height).await? == source_hash,
        "source/witness finality diverged during campaign"
    );
    let checkpoint = ctx.ethereum.checkpoint().await?;
    ensure!(
        u64::from(source_height) >= checkpoint.block,
        "accepted BEEFY checkpoint is ahead of finalized source"
    );
    let lag = u64::from(source_height) - checkpoint.block;
    ensure!(
        lag < ctx.schedule.authority_lag_limit,
        "accepted BEEFY checkpoint lag exceeded the sealed epoch bound"
    );
    let actor_state = ctx.follower_state()?;
    let actor_status = actor_state
        .as_ref()
        .and_then(|state| state["follower"]["status"].as_str())
        .unwrap_or("missing");
    let actor_sequence = actor_state
        .as_ref()
        .and_then(|state| state["startupSequence"].as_u64());
    journal.windows.entry("observation".into()).or_insert_with(||json!({"samples":[]}))["samples"].as_array_mut().context("observation journal is malformed")?.push(json!({
        "atMs":now_ms()?, "sourceHeight":source_height, "sourceHash":format!("{source_hash:#x}"),
        "acceptedBlock":checkpoint.block, "lag":lag, "followerStatus":actor_status,
        "startupSequence":actor_sequence, "followerState":ctx.follower_dir.join("state.json"),
    }));
    save_journal(path, journal)
}

fn write_terminal_report(journal: &mut Journal, path: &Path) -> Result<()> {
    let all = (0..QUALIFICATION_HOURS).all(|hour| window_complete(journal, hour));
    if journal.status != "failed" {
        journal.status = if all { "passed" } else { "failed" }.into();
    }
    let candidate = json!({
        "schemaVersion":SCHEMA_VERSION,"runId":journal.run_id,"deploymentManifestDigest":journal.deployment_manifest_digest,
        "sourceGenesis":journal.source_genesis,"bridgeDomain":journal.bridge_domain,"rawSpecSha256":journal.raw_spec_sha256,
        "t0Ms":journal.t0_ms,"qualificationVerdict":if all&&journal.status=="passed"{"PASS"}else{"FAIL"},
        "protectedQueueMigrationVerdict":"NOT ESTABLISHED","windows":journal.windows,
        "preflight":journal.preflight,"warmup":journal.warmup,"incidents":journal.incidents,
        "observedThroughMs":now_ms()?,
    });
    let report_path = path
        .parent()
        .context("journal has no parent")?
        .join("qualification-report.json");
    let matches = |existing: &Value| {
        [
            "schemaVersion",
            "runId",
            "deploymentManifestDigest",
            "sourceGenesis",
            "bridgeDomain",
            "rawSpecSha256",
            "t0Ms",
            "qualificationVerdict",
            "protectedQueueMigrationVerdict",
            "windows",
            "preflight",
            "warmup",
            "incidents",
        ]
        .iter()
        .all(|field| existing[*field] == candidate[*field])
    };
    let report = if report_path.exists() {
        let existing: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
        ensure!(matches(&existing), "terminal report already exists with different campaign evidence; refusing to rewrite it");
        if let Some(saved) = journal.terminal_report.as_ref() {
            ensure!(
                &existing == saved,
                "terminal report file differs from the immutable journal copy"
            );
        }
        existing
    } else if let Some(saved) = journal.terminal_report.as_ref() {
        ensure!(
            matches(saved),
            "saved terminal report differs from current immutable campaign evidence"
        );
        atomic_json(&report_path, saved)?;
        saved.clone()
    } else {
        atomic_json(&report_path, &candidate)?;
        candidate
    };
    journal.terminal_report = Some(report);
    save_journal(path, journal)
}

fn window_complete(journal: &Journal, hour: u8) -> bool {
    let Some(t0) = journal.t0_ms else {
        return false;
    };
    let Some(start) = t0.checked_add(u64::from(hour) * HOUR_MS) else {
        return false;
    };
    let Some(deadline) = start.checked_add(HOUR_MS) else {
        return false;
    };
    journal
        .windows
        .get(&window_key(hour))
        .is_some_and(|record| {
            let fee_type = if hour.is_multiple_of(2) {
                "normal"
            } else {
                "priority"
            };
            record["status"] == "passed"
                && record["scheduledStartMs"].as_u64() == Some(start)
                && record["deadlineMs"].as_u64() == Some(deadline)
                && record["boundaryIntentPersistedAtMs"]
                    .as_u64()
                    .is_some_and(|at| at <= start)
                && started_on_schedule(record, start)
                && record["completedAtMs"]
                    .as_u64()
                    .is_some_and(|at| at < deadline)
                && record["baseline"].is_object()
                && record["stages"]["afterMint"].is_object()
                && record["stages"]["finalizedReturn"].is_object()
                && record["assets"].as_object().is_some_and(|assets| {
                    assets.len() == 4
                        && ["USDC", "USDT", "WETH", "WBTC"].iter().all(|symbol| {
                            assets.get(*symbol).is_some_and(|asset| {
                                let milestones: [(&str, &[&str]); 3] = [
                                    (
                                        "lock",
                                        &[
                                            "submissionHandoffAtMs",
                                            "evmBlockTimestampMs",
                                            "evmFinalityObservedAtMs",
                                            "beaconCoverageObservedAtMs",
                                            "workerProofComposedAtMs",
                                            "workerProofHandoffAtMs",
                                            "gearFinalizedStatusObservedAtMs",
                                            "mintDeltaObservedAtMs",
                                        ],
                                    ),
                                    (
                                        "burn",
                                        &[
                                            "submissionHandoffAtMs",
                                            "burnFinalizedObservedAtMs",
                                            "rootRegisteredAtMs",
                                            "rootMaturityEligibleAtMs",
                                            "rootPublicationObservedAtMs",
                                            "rootMaturityObservedAtMs",
                                            "releaseFinalityObservedAtMs",
                                        ],
                                    ),
                                    (
                                        "paid",
                                        &["submissionHandoffAtMs", "paymentFinalizedObservedAtMs"],
                                    ),
                                ];
                                asset["status"] == "passed"
                                    && milestones.iter().all(|(operation, names)| {
                                        names.iter().all(|name| {
                                            asset[*operation]["milestones"][*name]
                                                .as_u64()
                                                .is_some_and(|at| at > 0)
                                        })
                                    })
                                    && asset["amountRaw"].as_str() == Some("1")
                                    && asset["lock"]["receipt"]["receiptBlockHash"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty())
                                    && asset["lock"]["incomingReceipt"]["slot"].as_u64().is_some()
                                    && asset["lock"]["incomingReceipt"]["transactionIndex"]
                                        .as_u64()
                                        .is_some()
                                    && asset["lock"]["receiptStatus"]["status"] == "Processed"
                                    && asset["lock"]["receiptStatus"]["finalizedHash"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty())
                                    && asset["burn"]["outboundRequest"]["managerEventBlockHash"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty())
                                    && asset["burn"]["rootPublication"]["sourceBlockHash"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty())
                                    && asset["burn"]["releaseReceipt"]["receipt"]
                                        ["receiptBlockHash"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty())
                                    && asset["burn"]["releaseReceipt"]["finalizedHash"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty())
                                    && asset["paid"]["paidEvent"]["type"] == fee_type
                                    && asset["paid"]["paidEvent"]["blockHash"]
                                        .as_str()
                                        .is_some_and(|s| !s.is_empty())
                            })
                        })
                })
        })
}
fn scheduled_window(hour: u8, start: u64, deadline: u64, persisted_at: u64) -> Value {
    json!({"status":"scheduled","hour":hour,"scheduledStartMs":start,"deadlineMs":deadline,
        "feeMode":if hour.is_multiple_of(2){FeeMode::Normal}else{FeeMode::Priority},"amountRaw":"1",
        "boundaryIntentPersistedAtMs":persisted_at,"assets":{},"stages":{}})
}
fn started_on_schedule(record: &Value, start: u64) -> bool {
    record["startedAtMs"]
        .as_u64()
        .is_some_and(|at| at <= start.saturating_add(CLOCK_DRIFT_MS))
}

fn can_start_window(prior: Option<&Value>, started_at: u64, start: u64) -> bool {
    started_at <= start.saturating_add(CLOCK_DRIFT_MS)
        || prior.is_some_and(|record| {
            record["status"] == "running" && started_on_schedule(record, start)
        })
}

fn window_has_boundary_intent(record: &Value, start: u64) -> bool {
    match record["status"].as_str() {
        Some("running") => {
            record["startedAtMs"].as_u64().is_some_and(|at| at <= start)
                || record["boundaryIntentPersistedAtMs"]
                    .as_u64()
                    .is_some_and(|at| at <= start)
        }
        Some("scheduled") => record["boundaryIntentPersistedAtMs"]
            .as_u64()
            .is_some_and(|at| at <= start),
        _ => false,
    }
}
fn window_key(hour: u8) -> String {
    format!("hour-{hour:02}")
}

fn mean_lag(samples: &[Value]) -> Result<f64> {
    let values: Vec<f64> = samples
        .iter()
        .map(|value| {
            value["acceptedLag"]
                .as_u64()
                .map(|n| n as f64)
                .context("missing lag sample")
        })
        .collect::<Result<_>>()?;
    ensure!(!values.is_empty(), "no lag samples");
    Ok(values.iter().sum::<f64>() / values.len() as f64)
}

#[derive(Clone, Copy)]
struct ClockAnchor {
    wall_ms: u64,
    monotonic: Instant,
}
impl ClockAnchor {
    fn new(wall_ms: u64) -> Result<Self> {
        Ok(Self {
            wall_ms,
            monotonic: Instant::now(),
        })
    }
    fn check(&self) -> Result<()> {
        let elapsed = u64::try_from(self.monotonic.elapsed().as_millis()).unwrap_or(u64::MAX);
        let expected = self.wall_ms.saturating_add(elapsed);
        let actual = now_ms()?;
        ensure!(
            actual.abs_diff(expected) <= CLOCK_DRIFT_MS,
            "system clock discontinuity exceeded five seconds"
        );
        Ok(())
    }
}

fn instant_deadline(unix_ms: u64) -> Result<Instant> {
    let now = now_ms()?;
    ensure!(now < unix_ms, "absolute schedule deadline has passed");
    Ok(Instant::now() + Duration::from_millis(unix_ms - now))
}
fn now_ms() -> Result<u64> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_millis(),
    )
    .context("Unix milliseconds overflow")
}
fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path.parent().context("output has no parent directory")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("evidence"),
        std::process::id(),
        now_ms()?
    ));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn save_journal(path: &Path, journal: &mut Journal) -> Result<()> {
    let now = now_ms()?;
    journal.validate_wall_time(now)?;
    journal.last_wall_ms = Some(now);
    let value = serde_json::to_value(&*journal)?;
    atomic_json(path, &value)
}
fn json_digest(value: &Value) -> Result<String> {
    let canonical = canonical_value(value);
    Ok(format!(
        "0x{}",
        hex::encode(keccak256(&serde_json::to_vec(&canonical)?))
    ))
}
fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort();
            let mut result = Map::new();
            for key in keys {
                result.insert(key.clone(), canonical_value(&object[key]));
            }
            Value::Object(result)
        }
        other => other.clone(),
    }
}
fn endpoint_digest(value: &str) -> String {
    format!("0x{}", hex::encode(keccak256(value.as_bytes())))
}
fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value
        .as_str()
        .with_context(|| format!("{name} missing or not a string"))
}
fn bytes32(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value.trim_start_matches("0x"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("expected a 32-byte hex value"))
}
fn actor_id(value: &str) -> Result<ActorId> {
    let bytes = hex::decode(value.trim_start_matches("0x"))?;
    Ok(ActorId::from(<[u8; 32]>::try_from(bytes).map_err(
        |_| anyhow!("expected a 32-byte Gear actor ID"),
    )?))
}
fn campaign_manager_address(deployment: &Value) -> Result<[u8; 20]> {
    let text = string(&deployment["manager"], "manager")?;
    let bytes = hex::decode(text.trim_start_matches("0x"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("deployment manager address must be 20 bytes"))
}

fn inbound_deployment_start_block(deployment: &Value, finalized: &Value) -> Result<u64> {
    ensure!(
        finalized["phase"] == "finalized" && number(&finalized["chainId"])? == HOODI_CHAIN_ID,
        "inbound discovery requires finalized Hoodi deployment evidence; HOLD"
    );
    let manager = format!(
        "{:#x}",
        Address::from(campaign_manager_address(deployment)?)
    );
    let mut creations = finalized["receipts"]
        .as_array()
        .context("finalized deployment receipts missing; HOLD")?
        .iter()
        .filter(|receipt| {
            receipt["contractAddress"]
                .as_str()
                .is_some_and(|address| address.eq_ignore_ascii_case(&manager))
        });
    let receipt = creations
        .next()
        .context("original ERC20 manager creation receipt missing; HOLD")?;
    ensure!(
        creations.next().is_none() && receipt["status"] == "0x1",
        "original ERC20 manager creation receipt is ambiguous or failed; HOLD"
    );
    let encoded = string(&receipt["blockNumber"], "original manager creation block")?;
    let block = u64::from_str_radix(
        encoded
            .strip_prefix("0x")
            .context("creation block is not an Ethereum quantity")?,
        16,
    )?;
    ensure!(
        block <= number(&finalized["finalizedBlock"])?,
        "manager creation is not covered by finalized deployment evidence; HOLD"
    );
    Ok(block)
}

fn sails_block_hash(hash: GearHash) -> H256 {
    H256::from_slice(hash.as_bytes())
}

pub async fn write_source_launch_state(
    raw_spec_path: &Path,
    source_rpc: &str,
    witness_rpc: &str,
    output: &Path,
) -> Result<()> {
    ensure!(
        crate::local_source_rpc(source_rpc) && crate::local_source_rpc(witness_rpc),
        "source readiness endpoints must be loopback"
    );
    ensure!(
        source_rpc != witness_rpc,
        "two independent Gear authorities are required"
    );

    let raw_spec = fs::read(raw_spec_path)
        .with_context(|| format!("read raw chain spec {}", raw_spec_path.display()))?;
    let raw_spec_value: Value =
        serde_json::from_slice(&raw_spec).context("raw chain spec is not valid JSON")?;
    ensure!(
        raw_spec_value["genesis"]["raw"].is_object(),
        "raw chain spec must contain genesis.raw storage"
    );
    let raw_spec_sha256 = sha256_digest(&raw_spec);
    let relay_binary = std::env::current_exe().context("locate running relay binary")?;
    let relay_binary_sha256 = sha256_digest(
        &fs::read(&relay_binary)
            .with_context(|| format!("read relay binary {}", relay_binary.display()))?,
    );

    let (alice, bob) = Source::connect_pair(
        gear_rpc_client::GearApi::new(source_rpc, 3).await?,
        gear_rpc_client::GearApi::new(witness_rpc, 3).await?,
    )
    .await?;
    let genesis_at = GearHash::from(alice.source_genesis);
    let (alice_genesis_runtime, bob_genesis_runtime) = tokio::try_join!(
        source::runtime_identity_at(&alice.api, genesis_at),
        source::runtime_identity_at(&bob.api, genesis_at),
    )?;
    ensure!(
        alice_genesis_runtime == bob_genesis_runtime,
        "source nodes disagree on raw genesis runtime identity"
    );
    let alice_runtime = alice.identity.runtime.clone();
    let profile = raw_spec_value.get("runtimeProfile");
    validate_source_runtime_profile(profile, &alice_runtime)?;
    if profile.is_none() {
        tokio::try_join!(
            source::validate_legacy_runtime(&alice.api),
            source::validate_legacy_runtime(&bob.api)
        )?;
    }
    let runtime_profile = profile
        .cloned()
        .unwrap_or_else(|| json!({"name": "legacy-fast-hoodi"}));

    let raw_storage = &raw_spec_value["genesis"]["raw"]["top"];
    let raw_code = raw_storage["0x3a636f6465"]
        .as_str()
        .context("raw chain spec has no genesis runtime :code")?;
    let raw_code = hex::decode(raw_code.trim_start_matches("0x"))?;
    ensure!(
        !raw_code.is_empty()
            && alice_genesis_runtime["runtimeCodeKeccak256"]
                == format!("0x{}", hex::encode(sp_core::hashing::keccak_256(&raw_code))),
        "raw chain spec runtime :code does not match source genesis runtime"
    );
    let domain_key = format!(
        "0x{}{}",
        hex::encode(sp_core::twox_128(b"GearEthBridge")),
        hex::encode(sp_core::twox_128(b"BridgeDomain"))
    );
    let raw_domain = raw_storage
        .get(domain_key.as_str())
        .and_then(Value::as_str)
        .map(bytes32)
        .transpose()?
        .unwrap_or([0; 32]);
    let (alice_genesis_domain, bob_genesis_domain) = tokio::try_join!(
        source::bridge_domain_at(&alice.api, genesis_at),
        source::bridge_domain_at(&bob.api, genesis_at)
    )?;
    ensure!(
        raw_domain == alice_genesis_domain && raw_domain == bob_genesis_domain,
        "raw spec domain differs from source genesis storage"
    );
    if alice.identity.domain_binding_block == 0 {
        ensure!(
            raw_domain == alice.bridge_domain,
            "genesis-bound source domain changed; HOLD"
        );
    } else {
        ensure!(
            raw_domain == [0; 32],
            "upgraded domain binding conflicts with nonzero genesis domain; HOLD"
        );
    }

    let common_height = alice.identity.finalized_height;
    let common_hash = format!("0x{}", hex::encode(alice.identity.finalized_hash));
    let runtime_identity =
        json!({"genesis": alice_genesis_runtime, "commonFinalized": alice_runtime});
    let (alice_binary, bob_binary) = tokio::try_join!(
        source_node_binary_identity(&alice),
        source_node_binary_identity(&bob),
    )?;
    ensure!(
        alice_binary == bob_binary,
        "source nodes report different Gear binary identities"
    );
    let (alice_peer, bob_peer) =
        tokio::try_join!(source_peer_state(&alice), source_peer_state(&bob))?;
    let (alice_peer_id, alice_peers) = alice_peer;
    let (bob_peer_id, bob_peers) = bob_peer;
    ensure!(
        alice_peer_id != bob_peer_id
            && alice_peers.len() == 1
            && bob_peers.len() == 1
            && alice_peers.first().is_some_and(|peer| peer == &bob_peer_id)
            && bob_peers.first().is_some_and(|peer| peer == &alice_peer_id),
        "source authorities must be reciprocal peers with no extra connections"
    );

    let (alice_current, alice_next) = (&alice.identity.current, &alice.identity.next);
    let (bob_current, bob_next) = (&bob.identity.current, &bob.identity.next);
    ensure_two_authorities(alice_current, "Alice current")?;
    ensure_two_authorities(alice_next, "Alice next")?;
    ensure_two_authorities(bob_current, "Bob current")?;
    ensure_two_authorities(bob_next, "Bob next")?;
    ensure!(
        alice_current == bob_current && alice_next == bob_next,
        "source nodes disagree on current or next BEEFY authorities at block {common_height}"
    );

    let genesis_hash = format!("0x{}", hex::encode(alice.source_genesis));
    let bridge_domain = format!("0x{}", hex::encode(alice.bridge_domain));
    let endpoints = json!({"alice": source_rpc, "bob": witness_rpc});
    let peer_ids = json!({"alice": alice_peer_id, "bob": bob_peer_id});
    let pinned = json!({
        "rawSpecSha256": raw_spec_sha256,
        "relayBinarySha256": relay_binary_sha256,
        "sourceNodeBinary": alice_binary.clone(),
        "runtime": runtime_identity.clone(),
        "runtimeProfile": runtime_profile,
        "endpoints": endpoints,
        "peerIds": peer_ids,
        "sourceIdentity": {
            "genesisHash": genesis_hash,
            "bridgeDomain": bridge_domain,
            "mmrStartBlock": alice.mmr_start_block,
            "domainBindingBlock": alice.identity.domain_binding_block,
            "beefyActivationBlock": alice.beefy_activation_block
        }
    });
    let authority_sets = json!({
        "current": authority_set_json(alice_current),
        "next": authority_set_json(alice_next)
    });
    let readiness = json!({
        "genesisHash": genesis_hash,
        "validatorCount": 2,
        "authorityScope": "localTestOnlyNotPublicRosterQualification",
        "sourceProofIdentity": alice.identity,
        "peerCounts": {"alice": alice_peers.len(), "bob": bob_peers.len()},
        "commonFinalized": {"height": common_height, "hash": common_hash},
        "authoritySets": authority_sets,
        "sourceNodeBinary": alice_binary,
        "runtime": runtime_identity,
        "peerIds": peer_ids
    });
    let state = json!({
        "schemaVersion": 1,
        "phase": "ready",
        "aliceRpc": source_rpc,
        "bobRpc": witness_rpc,
        "identity": {
            "bridgeDomain": bridge_domain,
            "validatorCount": 2,
            "runtimeProfile": runtime_profile,
            "domainBindingBlock": alice.identity.domain_binding_block,
            "runtimeCodeKeccak256": alice_runtime["runtimeCodeKeccak256"],
            "runtimeCodeBlake2b256": alice_runtime["runtimeCodeBlake2b256"],
            "rawSpecSha256": raw_spec_sha256,
            "relayBinarySha256": relay_binary_sha256,
            "runtimeCodeSha256": alice_runtime["runtimeCodeSha256"],
            "mmrStartBlock": alice.mmr_start_block,
            "beefyActivationBlock": alice.beefy_activation_block
        },
        "readiness": readiness,
        "pinned": pinned
    });

    let output_path = if output
        .parent()
        .is_some_and(|parent| parent.as_os_str().is_empty())
    {
        std::env::current_dir()?.join(output)
    } else {
        output.to_path_buf()
    };
    if output_path.exists() {
        let previous: Value =
            serde_json::from_slice(&fs::read(&output_path).with_context(|| {
                format!("read saved source launch state {}", output_path.display())
            })?)
            .context("decode saved source launch state")?;
        let previous_height =
            validate_launch_resume(&previous, &pinned, common_height, &common_hash)?;
        let previous_hash = bytes32(string(
            &previous["readiness"]["commonFinalized"]["hash"],
            "saved readiness.commonFinalized.hash",
        )?)?;
        let (alice_previous_hash, bob_previous_hash) = tokio::try_join!(
            alice.api.block_number_to_hash(previous_height),
            bob.api.block_number_to_hash(previous_height),
        )?;
        ensure!(
            alice_previous_hash.0 == previous_hash && bob_previous_hash.0 == previous_hash,
            "saved common-finalized checkpoint is no longer canonical on both source nodes"
        );
    }
    atomic_json(&output_path, &state)?;
    println!(
        "source launch readiness pinned at {}: genesis={} block={} hash={}",
        output_path.display(),
        genesis_hash,
        common_height,
        common_hash
    );
    Ok(())
}

fn validate_source_runtime_profile(profile: Option<&Value>, runtime: &Value) -> Result<()> {
    let Some(profile) = profile else {
        ensure!(
            runtime["babe"]["slotDuration"] == 3000 && runtime["babe"]["epochLength"] == 64,
            "named runtime requires an explicitly approved runtime profile; HOLD"
        );
        return Ok(());
    };
    ensure!(!profile.is_null(), "explicit runtime profile is null; HOLD");
    CampaignSchedule::from_profile(profile)?;
    ensure!(
        runtime["babe"]["slotDuration"] == profile["slotDurationMs"]
            && runtime["babe"]["epochLength"] == profile["epochDurationBlocks"],
        "common finalized runtime differs from approved profile cadence; HOLD"
    );
    for name in [
        "runtimeCodeSha256",
        "runtimeCodeKeccak256",
        "runtimeCodeBlake2b256",
    ] {
        let expected = bytes32(string(&profile[name], name)?)?;
        let observed = bytes32(string(&runtime[name], name)?)?;
        ensure!(
            expected == observed,
            "common finalized {name} differs from qualified source code; HOLD"
        );
    }
    Ok(())
}

async fn source_node_binary_identity(source: &Source) -> Result<Value> {
    let rpc = source.api.api.rpc();
    let name: String = rpc.request("system_name", rpc_params![]).await?;
    let version: String = rpc.request("system_version", rpc_params![]).await?;
    let chain: String = rpc.request("system_chain", rpc_params![]).await?;
    ensure!(
        !name.trim().is_empty() && !version.trim().is_empty() && !chain.trim().is_empty(),
        "source node binary identity is incomplete"
    );
    Ok(json!({"name": name, "version": version, "chain": chain}))
}

async fn source_peer_state(source: &Source) -> Result<(String, Vec<String>)> {
    let rpc = source.api.api.rpc();
    let peer_id: String = rpc.request("system_localPeerId", rpc_params![]).await?;
    ensure!(
        !peer_id.trim().is_empty(),
        "source node has no local peer ID"
    );
    let peers: Vec<Value> = rpc.request("system_peers", rpc_params![]).await?;
    let peer_ids = peers
        .iter()
        .map(|peer| {
            peer["peerId"]
                .as_str()
                .map(str::to_owned)
                .context("system_peers entry has no peerId")
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        peer_ids.iter().all(|peer| !peer.trim().is_empty())
            && peer_ids.iter().collect::<BTreeSet<_>>().len() == peer_ids.len(),
        "source node reports invalid or duplicate peer IDs"
    );
    Ok((peer_id, peer_ids))
}

fn ensure_two_authorities(set: &AuthoritySet, label: &str) -> Result<()> {
    ensure!(
        set.keys.len() == 2 && set.keys[0] != set.keys[1],
        "{label} BEEFY authority set must contain exactly two distinct keys"
    );
    Ok(())
}

fn authority_set_json(set: &AuthoritySet) -> Value {
    json!({
        "id": set.id,
        "length": set.keys.len(),
        "root": format!("0x{}", hex::encode(set.root)),
        "keys": set.keys.iter().map(|key| format!("0x{}", hex::encode(key))).collect::<Vec<_>>()
    })
}

fn validate_launch_resume(
    previous: &Value,
    pinned: &Value,
    current_height: u32,
    current_hash: &str,
) -> Result<u32> {
    ensure!(
        previous["phase"] == "ready",
        "saved source launch state is not ready"
    );
    ensure!(
        previous.get("pinned") == Some(pinned),
        "raw spec, binary, runtime, endpoint, peer, or source identity changed; refusing resume"
    );
    let previous_height: u32 = previous["readiness"]["commonFinalized"]["height"]
        .as_u64()
        .context("saved common-finalized height is missing")?
        .try_into()
        .context("saved common-finalized height exceeds u32")?;
    ensure!(
        current_height >= previous_height,
        "common-finalized height regressed from {previous_height} to {current_height}"
    );
    if current_height == previous_height {
        let previous_hash = string(
            &previous["readiness"]["commonFinalized"]["hash"],
            "saved readiness.commonFinalized.hash",
        )?;
        ensure!(
            previous_hash.eq_ignore_ascii_case(current_hash),
            "common-finalized hash changed at the saved height"
        );
    }
    Ok(previous_height)
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(sp_core::hashing::sha2_256(bytes)))
}

fn verify_raw_spec_digest(raw_spec: &[u8], recorded: &Value) -> Result<String> {
    let recorded = string(recorded, "launch.identity.rawSpecSha256")?;
    let expected = recorded.strip_prefix("0x").unwrap_or(recorded);
    ensure!(
        expected.len() == 64,
        "source launch state has an invalid raw chain-spec SHA256"
    );
    hex::decode(expected).context("source launch state has an invalid raw chain-spec SHA256")?;
    let actual = sha256_digest(raw_spec);
    ensure!(
        expected.eq_ignore_ascii_case(&actual[2..]),
        "raw chain-spec bytes do not match the recorded source launch digest"
    );
    Ok(actual)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn campaign_unit_acknowledgements_are_empty_not_routed() -> Result<()> {
        let route = vft_manager_client::vft_manager::io::Pause::ROUTE;
        decode_campaign_reply::<()>(&[], route)?;
        assert!(decode_campaign_reply::<()>(route, route).is_err());
        assert!(decode_campaign_reply::<()>(&[0], route).is_err());
        assert!(decode_campaign_reply::<bool>(&[], route).is_err());
        assert!(
            decode_campaign_reply::<Result<(), vft_manager_client::Error>>(&[], route).is_err()
        );
        Ok(())
    }
    #[test]
    fn bootstrap_assets_distinguish_inventory_from_bridge_liabilities() {
        let funded = RawSnapshot {
            evm_user: EthU256::ZERO,
            evm_escrow: EthU256::ZERO,
            evm_supply: EthU256::ZERO,
            gear_user: GearU256::from(100),
            gear_supply: GearU256::from(100),
            gear_escrow: Some(GearU256::zero()),
        };
        assert!(funded.has_no_bridge_liabilities());
        for field in 0..5 {
            let mut changed = funded.clone();
            match field {
                0 => changed.evm_escrow = EthU256::from(1),
                1 => changed.evm_user = EthU256::from(1),
                2 => changed.evm_supply = EthU256::from(1),
                3 => changed.gear_escrow = Some(GearU256::from(1)),
                _ => changed.gear_supply = GearU256::from(99),
            }
            assert!(!changed.has_no_bridge_liabilities());
        }
        let mut ethereum = funded;
        ethereum.gear_escrow = None;
        ethereum.gear_user = GearU256::zero();
        ethereum.gear_supply = GearU256::zero();
        ethereum.evm_user = EthU256::from(100);
        ethereum.evm_supply = EthU256::from(100);
        assert!(ethereum.has_no_bridge_liabilities());
        ethereum.gear_supply = GearU256::from(1);
        assert!(!ethereum.has_no_bridge_liabilities());
        ethereum.gear_supply = GearU256::zero();
        ethereum.gear_user = GearU256::from(1);
        assert!(!ethereum.has_no_bridge_liabilities());
    }

    #[test]
    fn normal_source_admission_requires_qualified_upgraded_code_and_normal_cadence() {
        let sha = format!("0x{}", "11".repeat(32));
        let keccak = format!("0x{}", "22".repeat(32));
        let blake = format!("0x{}", "33".repeat(32));
        let runtime = json!({"runtimeCodeSha256": sha, "runtimeCodeKeccak256": keccak,
            "runtimeCodeBlake2b256": blake, "babe": {"slotDuration": 3000, "epochLength": 2400}});
        let profile = json!({"name":"normal-runtime-hoodi", "runtimeCommit":"19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c",
            "runtimePullRequest":5642, "slotDurationMs":3000, "epochDurationBlocks":2400,
            "warmupDurationMs":18000000, "requiredAuthorityHandovers":2, "tokenBatchDurationMs":3600000,
            "applicationAttemptDurationMs":2640000, "testOnly":true, "executionAuthorized":false, "releaseQualified":false,
            "gearBinarySha256":"44".repeat(32), "approvalSha256":"55".repeat(32), "runtimeCiStatus":"unresolved",
            "runtimeCodeSha256":sha, "runtimeCodeKeccak256":keccak, "runtimeCodeBlake2b256":blake});
        assert!(validate_source_runtime_profile(Some(&profile), &runtime).is_ok());
        assert!(validate_source_runtime_profile(None, &runtime).is_err());
        let mut stale = profile.clone();
        stale["runtimeCommit"] = json!("f961bed815dd4ab0802703620605ea3b3659ac60");
        assert!(validate_source_runtime_profile(Some(&stale), &runtime).is_err());
        let mut fast = runtime.clone();
        fast["babe"]["epochLength"] = json!(64);
        assert!(validate_source_runtime_profile(Some(&profile), &fast).is_err());
        assert!(validate_source_runtime_profile(None, &fast).is_ok());
        for name in [
            "runtimeCodeSha256",
            "runtimeCodeKeccak256",
            "runtimeCodeBlake2b256",
        ] {
            let mut mismatch = profile.clone();
            mismatch[name] = json!(format!("0x{}", "44".repeat(32)));
            assert!(validate_source_runtime_profile(Some(&mismatch), &runtime).is_err());
        }
        let mut mislabeled = profile.clone();
        mislabeled["runtimeCodeSha256"] = runtime["runtimeCodeKeccak256"].clone();
        assert!(validate_source_runtime_profile(Some(&mislabeled), &runtime).is_err());
    }

    #[test]
    fn normal_schedule_keeps_five_hours_and_original_batch_deadlines() -> Result<()> {
        let profile = json!({"name":"normal-runtime-hoodi","runtimeCommit":"19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c","runtimePullRequest":5642,"slotDurationMs":3000,"epochDurationBlocks":2400,"warmupDurationMs":18000000,"requiredAuthorityHandovers":2,"tokenBatchDurationMs":3600000,"applicationAttemptDurationMs":2640000,"testOnly":true,"executionAuthorized":false,"releaseQualified":false,"approvalSha256":"11".repeat(32),"gearBinarySha256":"22".repeat(32),"runtimeCodeSha256":"33".repeat(32),"runtimeCodeKeccak256":"44".repeat(32),"runtimeCodeBlake2b256":"55".repeat(32),"runtimeCiStatus":"unresolved"});
        let schedule = CampaignSchedule::from_profile(&profile)?;
        assert_eq!(
            (
                schedule.warmup_secs,
                schedule.handovers,
                schedule.authority_lag_limit
            ),
            (18000, 2, 2400)
        );
        assert_eq!(
            CampaignSchedule::from_profile(&Value::Null)?.warmup_secs,
            3600
        );
        for (field, value) in [
            (
                "runtimeCommit",
                json!("f961bed815dd4ab0802703620605ea3b3659ac60"),
            ),
            ("runtimePullRequest", json!(5644)),
            ("epochDurationBlocks", json!(64)),
            ("warmupDurationMs", json!(3600000)),
            ("requiredAuthorityHandovers", json!(1)),
            ("tokenBatchDurationMs", json!(7200000)),
            ("applicationAttemptDurationMs", json!(3000000)),
            ("executionAuthorized", json!(true)),
            ("approvalSha256", json!("00".repeat(32))),
            ("functionalOnly", json!(true)),
            ("functionalOnly", json!(false)),
            ("functionalOnly", Value::Null),
            ("cadencePatchSha256", json!("55".repeat(32))),
            ("cadencePatchSha256", Value::Null),
        ] {
            let mut changed = profile.clone();
            changed[field] = value;
            assert!(CampaignSchedule::from_profile(&changed).is_err(), "{field}");
        }
        let started = 1000;
        let samples:Vec<_>=(0_u64..300).map(|minute|json!({"minute":minute,"scheduledAtMs":started+minute*60000,"atMs":started+minute*60000+2000,"sourceHeight":1000+minute,"workers":{"ready":true,"observedAtMs":started+minute*60000+1000}})).collect();
        validate_warmup_samples(&samples, started, 300)?;
        assert!(validate_warmup_samples(&samples[..60], started, 300).is_err());
        assert!(validate_warmup_samples(&samples[..299], started, 300).is_err());
        Ok(())
    }

    #[test]
    fn fast_profile_keeps_named_campaign_authentication_and_original_deadlines() -> Result<()> {
        let profile = json!({"name":"fast-runtime-hoodi", "runtimeCommit":"19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c",
            "runtimePullRequest":5642, "slotDurationMs":3000, "epochDurationBlocks":64,
            "warmupDurationMs":3600000, "requiredAuthorityHandovers":2,
            "tokenBatchDurationMs":3600000, "applicationAttemptDurationMs":2640000,
            "functionalOnly":true, "cadencePatchSha256":"55".repeat(32),
            "testOnly":true, "executionAuthorized":false, "releaseQualified":false,
            "gearBinarySha256":"44".repeat(32), "approvalSha256":"66".repeat(32), "runtimeCiStatus":"unresolved",
            "runtimeCodeSha256":"11".repeat(32), "runtimeCodeKeccak256":"22".repeat(32), "runtimeCodeBlake2b256":"33".repeat(32)});
        let schedule = CampaignSchedule::from_profile(&profile)?;
        assert_eq!(
            (
                schedule.warmup_secs,
                schedule.authority_lag_limit,
                schedule.handovers
            ),
            (3600, 64, 2)
        );
        let runtime = json!({"babe":{"slotDuration":3000,"epochLength":64},
            "runtimeCodeSha256":profile["runtimeCodeSha256"], "runtimeCodeKeccak256":profile["runtimeCodeKeccak256"],
            "runtimeCodeBlake2b256":profile["runtimeCodeBlake2b256"]});
        validate_source_runtime_profile(Some(&profile), &runtime)?;
        assert!(validate_source_runtime_profile(Some(&Value::Null), &runtime).is_err());
        for (field, bad) in [
            ("name", json!("normal-runtime-hoodi")),
            ("name", json!("unknown")),
            ("epochDurationBlocks", json!(2400)),
            ("slotDurationMs", json!(6000)),
            ("warmupDurationMs", json!(18000000)),
            ("functionalOnly", json!(false)),
            ("functionalOnly", Value::Null),
            ("cadencePatchSha256", json!("")),
            ("cadencePatchSha256", json!("00".repeat(32))),
            ("cadencePatchSha256", json!("not-a-hash")),
            ("cadencePatchSha256", Value::Null),
            (
                "runtimeCommit",
                json!("f961bed815dd4ab0802703620605ea3b3659ac60"),
            ),
            ("runtimePullRequest", json!(5644)),
            ("requiredAuthorityHandovers", json!(1)),
            ("tokenBatchDurationMs", json!(18000000)),
            ("applicationAttemptDurationMs", json!(18000000)),
            ("testOnly", json!(false)),
            ("executionAuthorized", json!(true)),
            ("releaseQualified", json!(true)),
            ("runtimeCiStatus", Value::Null),
        ] {
            let mut changed = profile.clone();
            changed[field] = bad;
            assert!(CampaignSchedule::from_profile(&changed).is_err(), "{field}");
            assert!(
                validate_source_runtime_profile(Some(&changed), &runtime).is_err(),
                "{field}"
            );
        }
        for field in [
            "gearBinarySha256",
            "approvalSha256",
            "runtimeCodeSha256",
            "runtimeCodeKeccak256",
            "runtimeCodeBlake2b256",
        ] {
            for bad in [Value::Null, json!(""), json!("00".repeat(32))] {
                let mut changed = profile.clone();
                changed[field] = bad;
                assert!(CampaignSchedule::from_profile(&changed).is_err(), "{field}");
            }
        }
        for field in [
            "runtimeCodeSha256",
            "runtimeCodeKeccak256",
            "runtimeCodeBlake2b256",
        ] {
            let mut changed = runtime.clone();
            changed[field] = json!("77".repeat(32));
            assert!(
                validate_source_runtime_profile(Some(&profile), &changed).is_err(),
                "{field}"
            );
        }
        let mut normal_runtime = runtime.clone();
        normal_runtime["babe"]["epochLength"] = json!(2400);
        assert!(validate_source_runtime_profile(Some(&profile), &normal_runtime).is_err());
        let started = 1000;
        let samples: Vec<_> = (0_u64..60)
            .map(|minute| {
                json!({"minute":minute, "scheduledAtMs":started+minute*60000,
            "atMs":started+minute*60000+2000, "sourceHeight":1000+minute,
            "workers":{"ready":true,"observedAtMs":started+minute*60000+1000}})
            })
            .collect();
        validate_warmup_samples(&samples, started, 60)?;
        assert!(validate_warmup_samples(&samples[..59], started, 60).is_err());
        Ok(())
    }

    #[test]
    fn six_asset_settlement_uses_explicit_native_policy_and_original_escrow() -> Result<()> {
        let mut tokens: Vec<_> = ["USDC", "USDT", "WETH", "WBTC"]
            .into_iter()
            .map(|symbol| Token {
                symbol,
                component: symbol,
                address: Address::ZERO,
                peer: ActorId::zero(),
                gear_origin: false,
                native_amount: None,
                escrow: None,
            })
            .collect();
        tokens.push(Token {
            symbol: "WTVARA",
            component: "ordinary",
            address: Address::ZERO,
            peer: ActorId::zero(),
            gear_origin: true,
            native_amount: None,
            escrow: Some(ActorId::zero()),
        });
        tokens.push(Token {
            symbol: "NOT_A_NATIVE_SYMBOL",
            component: "explicit-native",
            address: Address::ZERO,
            peer: ActorId::zero(),
            gear_origin: true,
            native_amount: Some(1000000000000),
            escrow: Some(ActorId::zero()),
        });
        assert_eq!(tokens[4].raw_amount(1)?, 1);
        assert_eq!(tokens[5].raw_amount(1)?, 1000000000000);
        assert!(tokens[5].raw_amount(u64::MAX).is_err());
        let baseline = SnapshotSet {
            gear_height: 12,
            gear_hash: [1; 32].into(),
            evm_height: 34,
            evm_hash: [2; 32].into(),
            assets: tokens
                .iter()
                .map(|token| {
                    (
                        token.symbol.into(),
                        if token.gear_origin {
                            let raw = GearU256::from(token.raw_amount(2).unwrap());
                            RawSnapshot {
                                evm_user: EthU256::ZERO,
                                evm_escrow: EthU256::ZERO,
                                evm_supply: EthU256::ZERO,
                                gear_user: raw,
                                gear_supply: raw,
                                gear_escrow: Some(GearU256::zero()),
                            }
                        } else {
                            RawSnapshot {
                                evm_user: EthU256::from(100),
                                evm_escrow: EthU256::ZERO,
                                evm_supply: EthU256::from(100),
                                gear_user: GearU256::zero(),
                                gear_supply: GearU256::zero(),
                                gear_escrow: None,
                            }
                        },
                    )
                })
                .collect(),
        };
        let mut exported = SnapshotSet::from_json(&baseline.to_json())?;
        assert_eq!(
            exported.assets, baseline.assets,
            "persisted Gear quantities are decimal, never implicit hexadecimal"
        );
        for token in tokens.iter().filter(|token| token.gear_origin) {
            let raw = token.raw_amount(1)?;
            let state = exported.assets.get_mut(token.symbol).unwrap();
            state.evm_user += EthU256::from(raw);
            state.evm_supply += EthU256::from(raw);
            state.gear_user -= GearU256::from(raw);
            state.gear_escrow = Some(GearU256::from(raw));
        }
        verify_gear_export_delta(&baseline, &exported, &tokens, 1)?;
        let mut wrong = SnapshotSet::from_json(&exported.to_json())?;
        wrong.assets.get_mut("WTVARA").unwrap().gear_supply -= GearU256::from(1);
        assert!(verify_gear_export_delta(&baseline, &wrong, &tokens, 1).is_err());
        let mut returned = SnapshotSet::from_json(&baseline.to_json())?;
        let state = returned.assets.get_mut("NOT_A_NATIVE_SYMBOL").unwrap();
        state.gear_user -= GearU256::from(1000000000000u64);
        state.gear_supply -= GearU256::from(1000000000000u64);
        verify_settlement_delta(&baseline, &returned, &tokens, 1)?;
        assert!(
            verify_settlement_delta(&baseline, &baseline, &tokens, 1).is_err(),
            "queued native value cannot become settled by retaining wrapped tokens"
        );
        returned.assets.get_mut("WTVARA").unwrap().gear_supply -= GearU256::from(1);
        assert!(
            verify_settlement_delta(&baseline, &returned, &tokens, 1).is_err(),
            "ordinary escrow return must not burn by symbol"
        );
        let route = vft_client::vft::io::Approve::ROUTE;
        let payload = [route, true.encode().as_slice()].concat();
        assert!(decode_campaign_reply::<bool>(&payload, route)?);
        let mut trailing = payload.clone();
        trailing.push(0);
        assert!(decode_campaign_reply::<bool>(&trailing, route).is_err());
        assert!(decode_campaign_reply::<bool>(&payload, SubmitReceipt::ROUTE).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn original_campaign_signed_bytes_bind_chain_nonce_token_recipient_and_amount(
    ) -> Result<()> {
        use alloy::{
            consensus::{SignableTransaction, TxEip1559, TxEnvelope},
            network::TxSigner,
            primitives::TxKind,
        };
        let signer = PrivateKeySigner::from_bytes(&B256::from([7; 32]))?;
        let manager = Address::from([3; 20]);
        let token = Address::from([4; 20]);
        let recipient = B256::from([5; 32]);
        let mut tx = TxEip1559 {
            chain_id: HOODI_CHAIN_ID,
            nonce: 3,
            gas_limit: 100000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(manager),
            input: CampaignManager::requestBridgingCall {
                token,
                amount: EthU256::from(1),
                to: recipient,
            }
            .abi_encode()
            .into(),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut tx).await?;
        let raw = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        let hash = B256::from(keccak256(&raw));
        let intent = json!({"kind":"erc20-lock","token":format!("{token:#x}"),"amountRaw":"1","gearRecipient":format!("{recipient:#x}"),"permit":false});
        campaign_evm_identity(&raw, hash, 3, &intent, signer.address(), manager)?;
        assert!(campaign_evm_identity(&raw, hash, 4, &intent, signer.address(), manager).is_err());
        assert!(campaign_evm_identity(&raw, hash, 3, &intent, Address::ZERO, manager).is_err());
        for (field, value) in [
            ("token", json!(format!("{:#x}", Address::ZERO))),
            ("amountRaw", json!("2")),
            ("gearRecipient", json!(format!("{:#x}", B256::ZERO))),
        ] {
            let mut changed = intent.clone();
            changed[field] = value;
            assert!(
                campaign_evm_identity(&raw, hash, 3, &changed, signer.address(), manager).is_err(),
                "{field}"
            );
        }
        let mut trailing = raw.clone();
        trailing.push(0);
        assert!(campaign_evm_identity(
            &trailing,
            B256::from(keccak256(&trailing)),
            3,
            &intent,
            signer.address(),
            manager
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn token_completion_requires_the_same_original_receipt_application_effect() -> Result<()> {
        let manager = Address::from([3; 20]);
        let token = Address::from([4; 20]);
        let sender = B256::from([5; 32]);
        let receiver = Address::from([6; 20]);
        let hash = B256::from([7; 32]);
        let data = CampaignManager::Bridged {
            from: sender,
            to: receiver,
            token,
            amount: EthU256::from(1),
        }
        .encode_log_data();
        let log = json!({"address":manager,"topics":data.topics(),"data":data.data,"blockHash":B256::from([8;32]),"blockNumber":"0x1","transactionHash":hash,"transactionIndex":"0x0","logIndex":"0x0","removed":false});
        let original = json!({"transactionHash":hash,"transactionIndex":"0x0","blockHash":B256::from([8;32]),"blockNumber":"0x1","from":receiver,"to":manager,"gasUsed":"0x1","cumulativeGasUsed":"0x1","effectiveGasPrice":"0x1","contractAddress":null,"logs":[log.clone()],"logsBloom":format!("0x{}","00".repeat(256)),"status":"0x1","type":"0x0"});
        let receipt = serde_json::from_value(original.clone())?;
        receipt_token_effect(&receipt, manager, token, sender, receiver, EthU256::from(1))?;
        for case in [
            "missing",
            "wrong-manager",
            "wrong-token",
            "wrong-sender",
            "wrong-receiver",
            "wrong-amount",
            "duplicate",
            "trailing",
        ] {
            let mut changed = original.clone();
            match case {
                "missing" => changed["logs"] = json!([]),
                "wrong-manager" => changed["logs"][0]["address"] = json!(Address::ZERO),
                "duplicate" => changed["logs"].as_array_mut().unwrap().push(log.clone()),
                "trailing" => {
                    changed["logs"][0]["data"] = json!(format!(
                        "{}00",
                        changed["logs"][0]["data"].as_str().unwrap()
                    ))
                }
                _ => {}
            };
            let receipt = serde_json::from_value(changed)?;
            assert!(
                receipt_token_effect(
                    &receipt,
                    manager,
                    if case == "wrong-token" {
                        Address::ZERO
                    } else {
                        token
                    },
                    if case == "wrong-sender" {
                        B256::ZERO
                    } else {
                        sender
                    },
                    if case == "wrong-receiver" {
                        Address::ZERO
                    } else {
                        receiver
                    },
                    EthU256::from(if case == "wrong-amount" { 2 } else { 1 })
                )
                .is_err(),
                "{case}"
            );
        }
        Ok(())
    }

    #[test]
    fn raw_spec_integrity_rejects_changed_bytes_and_malformed_hashes() {
        let raw_spec = br#"{"genesis":{"raw":{"top":{}}}}"#;
        let recorded = json!(sha256_digest(raw_spec));
        assert_eq!(
            verify_raw_spec_digest(raw_spec, &recorded).unwrap(),
            recorded.as_str().unwrap()
        );
        assert!(verify_raw_spec_digest(b"different raw spec", &recorded).is_err());
        assert!(verify_raw_spec_digest(raw_spec, &json!("not-a-sha256")).is_err());
    }

    #[test]
    fn source_launch_resume_pins_raw_spec_binary_runtime_endpoints_and_peers() {
        let pinned = json!({
            "rawSpecSha256": "0x11",
            "relayBinarySha256": "0x22",
            "runtime": {
                "genesis": {"babe": {"slotDuration": 3000, "epochLength": 64}},
                "commonFinalized": {"runtimeCodeSha256": "0x33"}
            },
            "sourceNodeBinary": {"name": "Gear", "version": "1", "chain": "Local"},
            "endpoints": {"alice": "ws://127.0.0.1:9948", "bob": "ws://127.0.0.1:9949"},
            "peerIds": {"alice": "alice-peer", "bob": "bob-peer"}
        });
        let hash = format!("0x{}", "44".repeat(32));
        let saved = json!({
            "phase": "ready",
            "pinned": pinned,
            "readiness": {"commonFinalized": {"height": 42, "hash": hash}}
        });
        assert_eq!(
            validate_launch_resume(&saved, &pinned, 43, &hash).unwrap(),
            42
        );
        assert!(validate_launch_resume(&saved, &pinned, 41, &hash).is_err());
        let other_hash = format!("0x{}", "55".repeat(32));
        assert!(validate_launch_resume(&saved, &pinned, 42, &other_hash).is_err());

        for field in ["rawSpecSha256", "relayBinarySha256"] {
            let mut changed = pinned.clone();
            changed[field] = json!("changed");
            assert!(validate_launch_resume(&saved, &changed, 43, &hash).is_err());
        }
        let mut changed_runtime = pinned.clone();
        changed_runtime["runtime"]["commonFinalized"]["runtimeCodeSha256"] = json!("changed");
        assert!(validate_launch_resume(&saved, &changed_runtime, 43, &hash).is_err());
        let mut changed_babe = pinned.clone();
        changed_babe["runtime"]["genesis"]["babe"]["epochLength"] = json!(32);
        assert!(validate_launch_resume(&saved, &changed_babe, 43, &hash).is_err());
        let mut changed_node_binary = pinned.clone();
        changed_node_binary["sourceNodeBinary"]["version"] = json!("2");
        assert!(validate_launch_resume(&saved, &changed_node_binary, 43, &hash).is_err());
        let mut changed_endpoint = pinned.clone();
        changed_endpoint["endpoints"]["alice"] = json!("ws://127.0.0.1:9947");
        assert!(validate_launch_resume(&saved, &changed_endpoint, 43, &hash).is_err());
        let mut changed_peer = pinned.clone();
        changed_peer["peerIds"]["alice"] = json!("replacement-peer");
        assert!(validate_launch_resume(&saved, &changed_peer, 43, &hash).is_err());
    }

    #[test]
    fn source_readiness_requires_two_distinct_current_and_next_authorities() {
        let two = AuthoritySet {
            id: 0,
            keys: vec![vec![1], vec![2]],
            root: [0; 32],
        };
        let one = AuthoritySet {
            id: 0,
            keys: vec![vec![1]],
            root: [0; 32],
        };
        let duplicate = AuthoritySet {
            id: 0,
            keys: vec![vec![1], vec![1]],
            root: [0; 32],
        };
        assert!(ensure_two_authorities(&two, "current").is_ok());
        assert!(ensure_two_authorities(&one, "current").is_err());
        assert!(ensure_two_authorities(&duplicate, "next").is_err());
    }
    fn test_journal() -> Journal {
        Journal::new(
            "test-run".into(),
            "manifest".into(),
            "tokens".into(),
            "launch".into(),
            "genesis".into(),
            "domain".into(),
            "spec".into(),
            json!({}),
            json!({}),
        )
    }
    #[test]
    fn empty_inbound_journal_requires_the_original_lane_and_an_independent_signer() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("state.json");
        let manager = format!("{:#x}", Address::from([3; 20]));
        let deployment = json!({"manager":manager});
        let finalized = json!({"phase":"finalized","chainId":HOODI_CHAIN_ID,"finalizedBlock":210,
            "receipts":[{"contractAddress":manager,"status":"0x1","blockNumber":"0xc8"}]});
        let identity = InboundRuntimeIdentity {
            ethereum_chain_id: HOODI_CHAIN_ID,
            ethereum_genesis_hash: bytes32(HOODI_EXECUTION_GENESIS)?.into(),
            ethereum_start_block: 200,
            erc20_manager_address: Some([3; 20].into()),
            bridging_payment_address: None,
            gear_genesis_hash: [4; 32].into(),
            vft_manager_address: [5; 32].into(),
            checkpoint_light_client_address: [6; 32].into(),
            historical_proxy_address: [7; 32].into(),
            gear_sender: [8; 32],
        };
        let mut expected = identity.clone();
        expected.ethereum_start_block = inbound_deployment_start_block(&deployment, &finalized)?;
        let original = json!({"schema_version":5,"runtime_identity":identity,
            "transactions":{},"completed":{},"failed":{}});
        for case in [
            "correct",
            "wrong-chain",
            "wrong-evm-genesis",
            "wrong-start",
            "wrong-manager",
            "paid-mode",
            "wrong-source",
            "wrong-vft-manager",
            "wrong-checkpoint",
            "wrong-proxy",
            "campaign-signer",
            "governance-signer",
            "zero-signer",
            "missing-signer",
            "missing-identity",
            "manual-mode",
            "old-schema",
        ] {
            let mut state = original.clone();
            let runtime = &mut state["runtime_identity"];
            match case {
                "correct" => {}
                "wrong-chain" => runtime["ethereum_chain_id"] = json!(1),
                "wrong-evm-genesis" => {
                    runtime["ethereum_genesis_hash"] = json!(H256::repeat_byte(9))
                }
                "wrong-start" => runtime["ethereum_start_block"] = json!(201),
                "wrong-manager" => runtime["erc20_manager_address"] = json!(H160::repeat_byte(9)),
                "paid-mode" => {
                    runtime["erc20_manager_address"] = Value::Null;
                    runtime["bridging_payment_address"] = json!(H160::repeat_byte(9));
                }
                "wrong-source" => runtime["gear_genesis_hash"] = json!(H256::repeat_byte(9)),
                "wrong-vft-manager" => runtime["vft_manager_address"] = json!(H256::repeat_byte(9)),
                "wrong-checkpoint" => {
                    runtime["checkpoint_light_client_address"] = json!(H256::repeat_byte(9))
                }
                "wrong-proxy" => runtime["historical_proxy_address"] = json!(H256::repeat_byte(9)),
                "campaign-signer" => runtime["gear_sender"] = serde_json::to_value([10u8; 32])?,
                "governance-signer" => runtime["gear_sender"] = serde_json::to_value([11u8; 32])?,
                "zero-signer" => runtime["gear_sender"] = serde_json::to_value([0u8; 32])?,
                "missing-signer" => {
                    runtime.as_object_mut().unwrap().remove("gear_sender");
                }
                "missing-identity" => {
                    state.as_object_mut().unwrap().remove("runtime_identity");
                }
                "manual-mode" => {
                    state["manual_identity"] = json!({
                        "tx_hash":H256::repeat_byte(12),"slot_number":123,
                        "ethereum_block_hash":H256::repeat_byte(13),"transaction_index":0,"receiver_route":[1,2,3],
                    })
                }
                "old-schema" => state["schema_version"] = json!(4),
                _ => unreachable!(),
            }
            let bytes = serde_json::to_vec(&state)?;
            fs::write(&path, &bytes)?;
            let accepted = read_worker_json::<InboundJournal>(&path)
                .and_then(|journal| {
                    let journal = journal.context("inbound state missing")?;
                    // The worker owns its credential; the campaign authenticates the lane and role separation.
                    let mut lane = expected.clone();
                    lane.gear_sender = journal.runtime_identity.gear_sender;
                    journal.validate_identity(&lane, [10; 32].into(), [11; 32].into())
                })
                .is_ok();
            assert_eq!(
                accepted,
                case == "correct",
                "{case}: empty work cannot bypass runtime binding"
            );
            assert_eq!(fs::read(&path)?, bytes);
        }
        for case in [
            "wrong-chain",
            "unfinalized",
            "failed-creation",
            "duplicate-creation",
            "missing-creation",
        ] {
            let mut changed = finalized.clone();
            match case {
                "wrong-chain" => changed["chainId"] = json!(1),
                "unfinalized" => changed["phase"] = json!("mined"),
                "failed-creation" => changed["receipts"][0]["status"] = json!("0x0"),
                "duplicate-creation" => changed["receipts"]
                    .as_array_mut()
                    .unwrap()
                    .push(finalized["receipts"][0].clone()),
                "missing-creation" => changed["receipts"] = json!([]),
                _ => unreachable!(),
            }
            assert!(
                inbound_deployment_start_block(&deployment, &changed).is_err(),
                "{case}"
            );
        }
        Ok(())
    }

    #[test]
    fn receipt_probe_lifecycle_requires_durable_canonical_live_rejections() {
        let baseline = SnapshotSet {
            gear_height: 12,
            gear_hash: [1; 32].into(),
            evm_height: 34,
            evm_hash: [2; 32].into(),
            assets: ["USDC", "USDT", "WETH", "WBTC"]
                .into_iter()
                .map(|symbol| {
                    (
                        symbol.into(),
                        RawSnapshot {
                            gear_escrow: None,
                            evm_user: EthU256::from(100),
                            evm_escrow: EthU256::ZERO,
                            evm_supply: EthU256::from(100),
                            gear_user: GearU256::zero(),
                            gear_supply: GearU256::zero(),
                        },
                    )
                })
                .collect(),
        };
        for state in ["passed", "burn-without-probes"] {
            let mut journal = test_journal();
            journal.preflight.evidence["governancePause"] = json!({
                "status":"passed", "pausedBlockHash":"0x01", "unpausedBlockHash":"0x02",
                "rejectedAs":"Paused", "balancesInFlight":false,
            });
            if state == "passed" {
                journal.preflight.status = "passed".into();
            } else {
                let directory = tempfile::tempdir().unwrap();
                prepare_gear_action(
                    &mut journal,
                    &directory.path().join("campaign.json"),
                    "preflight-normal/USDC-gear-burn",
                    json!({"kind":"vft-manager-request"}),
                )
                .unwrap();
            }
            assert!(
                validate_preflight_setup(&journal, &baseline).is_err(),
                "{state}: missing live receipt probes must HOLD before campaign operations"
            );
        }
        let tokens: Vec<_> = ["USDC", "USDT", "WETH", "WBTC"]
            .into_iter()
            .map(|symbol| Token {
                symbol,
                component: symbol,
                address: Address::ZERO,
                peer: ActorId::zero(),
                gear_origin: false,
                native_amount: None,
                escrow: None,
            })
            .collect();
        let receipt = vec![1, 2, 3];
        let mut complete_journal = test_journal();
        for probe in RECEIPT_PROBES {
            for case in [
                "canonical",
                "missing-reply",
                "wrong-application",
                "runtime-failure",
                "trailing-reply",
                "wrong-message",
                "changed-enqueue",
                "changed-before",
                "changed-accounting",
                "reserved",
                "deadline",
                "absolute-deadline",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("campaign.json");
                let mut journal = test_journal();
                let before = SnapshotSet::from_json(&baseline.to_json()).unwrap();
                let mut after = SnapshotSet::from_json(&baseline.to_json()).unwrap();
                after.gear_height = 16;
                after.gear_hash = [3; 32].into();
                let now = now_ms().unwrap();
                let intent = json!({"sender":format!("0x{}",hex::encode([4;32])),
                    "proxy":format!("0x{}",hex::encode([5;32])),"receiptRlp":format!("0x{}",hex::encode(&receipt)),
                    "originalReceiptKey":[200,3],"batchDeadlineMs":if case == "absolute-deadline" { now - 1 } else { now + PRELIGHT_MAX_SECS * 1_000 },
                    "expectedRejection":probe.rejection(),"preSendFinalizedHeight":before.gear_height,
                    "preSendFinalizedHash":format!("{:#x}",before.gear_hash)});
                assert!(
                    prepare_receipt_probe(&mut journal, &path, probe, intent.clone(), &before)
                        .unwrap()
                );
                let durable: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert_eq!(
                    durable["actions"][probe.action_id()]["status"],
                    "broadcasting"
                );
                assert_eq!(durable["actions"][probe.action_id()]["intent"], intent);
                assert_eq!(
                    durable["actions"][probe.action_id()]["evidence"]["before"],
                    before.to_json()
                );
                for unresolved in ["broadcasting", "ambiguous", "submitted"] {
                    journal.actions.get_mut(probe.action_id()).unwrap().status = unresolved.into();
                    assert!(
                        !prepare_receipt_probe(&mut journal, &path, probe, intent.clone(), &before)
                            .unwrap(),
                        "{unresolved}: a second network handoff must be suppressed"
                    );
                }
                journal.actions.get_mut(probe.action_id()).unwrap().status = "broadcasting".into();
                record_probe_submission(&mut journal, &path, probe, [6; 32].into(), [7; 32].into())
                    .unwrap();
                let persisted: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert_eq!(
                    persisted["actions"][probe.action_id()]["evidence"]["submission"]["messageId"],
                    format!("0x{}", hex::encode([6; 32]))
                );
                journal.actions.get_mut(probe.action_id()).unwrap().evidence["submission"]
                    ["enqueueHeight"] = json!(14);
                let proxy_result: Result<(Vec<u8>, Vec<u8>), historical_proxy_client::ProxyError> =
                    match probe {
                        ReceiptProbe::InvalidReceiptProof => {
                            Err(historical_proxy_client::ProxyError::EthereumEventClient(
                                if case == "wrong-application" {
                                    historical_proxy_client::Error::MissingCheckpoint
                                } else {
                                    historical_proxy_client::Error::InvalidReceiptProof
                                },
                            ))
                        }
                        ReceiptProbe::ProcessedReceiptReplay => {
                            let result: Result<
                                Vec<(ActorId, GearU256)>,
                                vft_manager_client::Error,
                            > = Err(if case == "wrong-application" {
                                vft_manager_client::Error::Paused
                            } else {
                                vft_manager_client::Error::AlreadyProcessed
                            });
                            let mut manager = SubmitReceipt::ROUTE.to_vec();
                            manager.extend(result.encode());
                            Ok((receipt.clone(), manager))
                        }
                    };
                let mut raw = Redirect::ROUTE.to_vec();
                raw.extend(proxy_result.encode());
                if case == "trailing-reply" {
                    raw.push(0);
                }
                let mut reply = json!({"messageId":format!("0x{}",hex::encode([6;32])),"enqueueHeight":14,
                    "enqueueBlockHash":format!("0x{}",hex::encode([7;32])),"replyId":format!("0x{}",hex::encode([8;32])),
                    "finalizedHeight":15,"finalizedHash":format!("0x{}",hex::encode([9;32])),
                    "source":intent["proxy"],"destination":intent["sender"],"runtimeSuccess":case != "runtime-failure",
                    "rawReply":format!("0x{}",hex::encode(raw)),"observedAtMs":now});
                match case {
                    "missing-reply" => reply = Value::Null,
                    "wrong-message" => {
                        reply["messageId"] = json!(format!("0x{}", hex::encode([10; 32])))
                    }
                    "changed-enqueue" => {
                        reply["enqueueBlockHash"] = json!(format!("0x{}", hex::encode([11; 32])))
                    }
                    "changed-before" => {
                        journal.actions.get_mut(probe.action_id()).unwrap().evidence["before"]
                            ["assets"]["USDC"]["gearUser"] = json!("1")
                    }
                    "changed-accounting" => {
                        after.assets.get_mut("WBTC").unwrap().gear_supply = GearU256::from(1)
                    }
                    _ => {}
                }
                let deadline = if case == "deadline" {
                    Instant::now() - Duration::from_secs(1)
                } else {
                    Instant::now() + Duration::from_secs(PRELIGHT_MAX_SECS)
                };
                let status = if case == "reserved" {
                    GearReceiptStatus::Reserved
                } else {
                    GearReceiptStatus::Processed
                };
                let result = finalize_receipt_probe(
                    &mut journal,
                    &path,
                    probe,
                    reply.clone(),
                    &before,
                    &after,
                    &tokens,
                    status,
                    deadline,
                );
                if case != "canonical" {
                    assert!(
                        result.is_err(),
                        "{case}: an unproven/changed receipt probe must HOLD"
                    );
                    assert_ne!(journal.actions[probe.action_id()].status, "finalized");
                    assert!(journal.preflight.evidence["receiptProbes"][probe.key()].is_null());
                    continue;
                }
                result.unwrap();
                let first_evidence = journal.actions[probe.action_id()].evidence.clone();
                assert!(
                    !prepare_receipt_probe(&mut journal, &path, probe, intent, &before).unwrap()
                );
                finalize_receipt_probe(
                    &mut journal,
                    &path,
                    probe,
                    reply.clone(),
                    &before,
                    &after,
                    &tokens,
                    GearReceiptStatus::Processed,
                    deadline,
                )
                .unwrap();
                assert_eq!(journal.actions[probe.action_id()].evidence, first_evidence);
                reply["finalizedHash"] = json!(format!("0x{}", hex::encode([12; 32])));
                assert!(finalize_receipt_probe(
                    &mut journal,
                    &path,
                    probe,
                    reply,
                    &before,
                    &after,
                    &tokens,
                    GearReceiptStatus::Processed,
                    deadline
                )
                .is_err());
                assert_eq!(journal.actions[probe.action_id()].evidence, first_evidence);
                complete_journal.actions.insert(
                    probe.action_id().into(),
                    journal.actions[probe.action_id()].clone(),
                );
                complete_journal.preflight.evidence["receiptProbes"][probe.key()] =
                    journal.preflight.evidence["receiptProbes"][probe.key()].clone();
            }
        }
        assert!(receipt_probes_completed(&complete_journal).unwrap());
        complete_journal
            .actions
            .get_mut(ReceiptProbe::ProcessedReceiptReplay.action_id())
            .unwrap()
            .evidence["reply"] = Value::Null;
        assert!(receipt_probes_completed(&complete_journal).is_err());
    }

    #[test]
    fn resumed_preflight_admits_only_its_exact_unpaid_source_message() {
        use relayer::message_relayer::common::{AuthoritySetId, GearBlockNumber, MessageInBlock};
        let mut nonce = [0; 32];
        GearU256::from(7).to_big_endian(&mut nonce);
        let message = MessageInBlock {
            message: gear_rpc_client::dto::Message {
                nonce_be: nonce,
                source: [1; 32],
                destination: [2; 20],
                payload: vec![3, 4],
            },
            block: GearBlockNumber(12),
            block_hash: [5; 32].into(),
            authority_set_id: AuthoritySetId(1),
        };
        let request = OutboundRequest {
            nonce: GearU256::from(7),
            queue_id: 0,
            message_hash: relayer::message_relayer::common::message_hash(&message.message),
            queue_block: 12,
            queue_block_hash: [5; 32].into(),
        };
        let key = hex::encode(nonce);
        let allowed = BTreeMap::from([(key.clone(), request)]);
        let mut queued = Map::from_iter([(key.clone(), serde_json::to_value(&message).unwrap())]);
        assert!(!queued_work_matches(&queued, &BTreeMap::new()).unwrap());
        assert!(queued_work_matches(&queued, &allowed).unwrap());
        let mut changed = message.clone();
        changed.block_hash = [6; 32].into();
        queued.insert(key.clone(), serde_json::to_value(&changed).unwrap());
        assert!(!queued_work_matches(&queued, &allowed).unwrap());
        changed = message.clone();
        changed.message.payload.push(9);
        queued.insert(key.clone(), serde_json::to_value(&changed).unwrap());
        assert!(!queued_work_matches(&queued, &allowed).unwrap());
        queued.insert(key, serde_json::to_value(message).unwrap());
        queued.insert("unrelated".into(), json!({}));
        assert!(!queued_work_matches(&queued, &allowed).unwrap());
    }

    #[test]
    fn saved_balance_checkpoint_cannot_be_overwritten_after_burn_or_return() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("campaign.json");
        let mut journal = test_journal();
        let mut snapshot = SnapshotSet {
            gear_height: 12,
            gear_hash: [1; 32].into(),
            evm_height: 34,
            evm_hash: [2; 32].into(),
            assets: ["USDC", "USDT", "WETH", "WBTC"]
                .into_iter()
                .map(|symbol| {
                    (
                        symbol.into(),
                        RawSnapshot {
                            gear_escrow: None,
                            evm_user: EthU256::from(99),
                            evm_escrow: EthU256::from(1),
                            evm_supply: EthU256::from(100),
                            gear_user: GearU256::from(1),
                            gear_supply: GearU256::from(1),
                        },
                    )
                })
                .collect(),
        };
        record_balance_checkpoint(
            &mut journal,
            &path,
            "preflight-normal",
            "afterMint",
            &snapshot,
        )
        .unwrap();
        let original = fs::read(&path).unwrap();
        record_balance_checkpoint(
            &mut journal,
            &path,
            "preflight-normal",
            "afterMint",
            &snapshot,
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        snapshot.assets.get_mut("USDC").unwrap().gear_user = GearU256::zero();
        assert!(record_balance_checkpoint(
            &mut journal,
            &path,
            "preflight-normal",
            "afterMint",
            &snapshot
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            journal.windows["preflight-normal"]["stages"]["afterMint"]["assets"]["USDC"]
                ["gearUser"],
            "1"
        );
    }

    #[test]
    fn preflight_resume_preserves_in_flight_balances_and_completed_governance() {
        let mut journal = test_journal();
        let mut baseline = SnapshotSet {
            gear_height: 1,
            gear_hash: GearHash::zero(),
            evm_height: 1,
            evm_hash: B256::ZERO,
            assets: BTreeMap::from([(
                "USDC".into(),
                RawSnapshot {
                    gear_escrow: None,
                    evm_user: EthU256::from(100),
                    evm_escrow: EthU256::ZERO,
                    evm_supply: EthU256::from(100),
                    gear_user: GearU256::zero(),
                    gear_supply: GearU256::zero(),
                },
            )]),
        };
        assert!(validate_preflight_setup(&journal, &baseline).unwrap());
        let original_baseline = baseline.to_json();
        let state = baseline.assets.get_mut("USDC").unwrap();
        state.evm_escrow = EthU256::from(1);
        state.gear_user = GearU256::from(1);
        state.gear_supply = GearU256::from(1);
        assert!(validate_preflight_setup(&journal, &baseline).is_err());
        start_preflight_window(&mut journal, "preflight-normal", FeeMode::Normal, 1_000).unwrap();
        journal.windows.get_mut("preflight-normal").unwrap()["baseline"] =
            original_baseline.clone();
        assert!(validate_preflight_setup(&journal, &baseline).is_err());
        journal.preflight.evidence["governancePause"] = json!({
            "status":"passed","pausedBlockHash":"0x01","unpausedBlockHash":"0x02",
            "rejectedAs":"Paused","balancesInFlight":false,
        });
        let saved = serde_json::to_value(&journal).unwrap();
        assert!(!validate_preflight_setup(&journal, &baseline).unwrap());
        assert_eq!(serde_json::to_value(&journal).unwrap(), saved);
        assert_eq!(
            journal.windows["preflight-normal"]["baseline"],
            original_baseline
        );
        journal.windows.get_mut("preflight-normal").unwrap()["status"] = json!("passed");
        assert!(validate_preflight_setup(&journal, &baseline).is_err());
    }

    #[test]
    fn interrupted_governance_probe_holds_each_handoff_without_replacing_evidence() {
        let mut journal = test_journal();
        for stage in ["pause-intent", "paused-request-intent", "unpause-intent"] {
            journal.preflight.evidence["governancePause"] =
                json!({"status":stage,"pausedBlockHash":"original"});
            let original = journal.preflight.evidence.clone();
            assert!(governance_pause_completed(&journal)
                .unwrap_err()
                .to_string()
                .contains("HOLD"));
            assert_eq!(journal.preflight.evidence, original);
        }
    }

    #[test]
    fn resumed_preflight_and_warmup_readiness_reject_a_rollback_without_rewriting_identity(
    ) -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("campaign.json");
        let expected = test_journal();
        for phase in ["preflight", "warmup-readiness"] {
            let mut journal = expected.clone();
            if phase == "preflight" {
                journal.preflight.status = "running".into();
                journal.preflight.started_at_ms = Some(1_000);
                start_preflight_window(&mut journal, "preflight-normal", FeeMode::Normal, 1_000)?;
            } else {
                journal.preflight.status = "passed".into();
                journal.warmup.status = "running".into();
                journal.warmup.evidence = json!({
                    "readinessStartedAtMs":1_000,"readinessDeadlineAtMs":1_000 + 30 * 60_000,
                    "rotationIntent":{"nonce":7,"signed_extrinsic":[3,4,5]},
                    "rotationHandoffAtMs":2_000,"samples":[],
                });
            }
            journal.last_wall_ms = Some(9_000);
            let original = serde_json::to_vec_pretty(&journal)?;
            fs::write(&path, &original)?;
            let mut recovered: Journal = serde_json::from_slice(&fs::read(&path)?)?;
            for now in [9_000, 9_001] {
                recovered.validate_identity(&expected, now)?;
            }
            // These times are after the original phase start, but before its last durable observation.
            for now in [8_999, 2_001] {
                assert!(
                    recovered.validate_identity(&expected, now).is_err(),
                    "{phase}: {now}"
                );
            }
            assert_eq!(serde_json::to_vec_pretty(&recovered)?, original);
            assert_eq!(fs::read(&path)?, original);
            // A clock rollback during readiness must not erase the persisted high-water mark either.
            recovered.last_wall_ms = Some(u64::MAX);
            let held = serde_json::to_vec_pretty(&recovered)?;
            fs::write(&path, &held)?;
            assert!(save_journal(&path, &mut recovered).is_err(), "{phase}");
            assert_eq!(serde_json::to_vec_pretty(&recovered)?, held);
            assert_eq!(fs::read(&path)?, held);
        }
        Ok(())
    }

    #[test]
    fn resumed_preflight_keeps_its_absolute_deadline_and_cannot_reopen_a_closed_window() {
        let mut journal = test_journal();
        let first =
            start_preflight_window(&mut journal, "preflight-normal", FeeMode::Normal, 1_000)
                .unwrap();
        assert_eq!(first, (1_000, 1_000 + 60 * 60 * 1_000));
        let original = journal.windows["preflight-normal"].clone();
        assert_eq!(
            start_preflight_window(
                &mut journal,
                "preflight-normal",
                FeeMode::Normal,
                first.1 - 1
            )
            .unwrap(),
            first
        );
        assert!(
            start_preflight_window(&mut journal, "preflight-normal", FeeMode::Normal, first.1)
                .is_err()
        );
        assert!(start_preflight_window(
            &mut journal,
            "preflight-normal",
            FeeMode::Normal,
            first.0 - 1
        )
        .is_err());
        assert_eq!(journal.windows["preflight-normal"], original);
        journal.windows.get_mut("preflight-normal").unwrap()["status"] = json!("failed");
        assert!(start_preflight_window(
            &mut journal,
            "preflight-normal",
            FeeMode::Normal,
            first.0 + 1
        )
        .is_err());
        // A previously admitted 44-minute batch keeps its original deadline after an upgrade.
        let legacy_deadline = first.0 + 44 * 60 * 1_000;
        journal.windows.insert(
            "preflight-normal".into(),
            json!({
                "status":"running", "startedAtMs":first.0, "deadlineMs":legacy_deadline
            }),
        );
        let legacy = journal.windows["preflight-normal"].clone();
        assert_eq!(
            start_preflight_window(
                &mut journal,
                "preflight-normal",
                FeeMode::Normal,
                legacy_deadline - 1
            )
            .unwrap(),
            (first.0, legacy_deadline)
        );
        assert!(start_preflight_window(
            &mut journal,
            "preflight-normal",
            FeeMode::Normal,
            legacy_deadline
        )
        .is_err());
        assert_eq!(journal.windows["preflight-normal"], legacy);
    }

    #[test]
    fn warmup_work_cannot_shift_minutes_or_backfill_a_missed_sample() {
        let start = Instant::now();
        assert_eq!(
            warmup_sample_deadline(start, 1, start + Duration::from_secs(95)).unwrap(),
            start + Duration::from_secs(120)
        );
        assert!(warmup_sample_deadline(start, 1, start + Duration::from_secs(120)).is_err());
        assert!(warmup_sample_deadline(start, 31, start + Duration::from_secs(33 * 60)).is_err());
        assert_eq!(
            warmup_sample_deadline(start, 59, start + Duration::from_secs(59 * 60 + 20)).unwrap(),
            start + Duration::from_secs(WARMUP_SECS)
        );
    }

    #[test]
    fn manifest_digest_ignores_object_insertion_order() {
        let a = json!({"ethereum":{"queue":"q","chainId":560048},"source":"s"});
        let b = json!({"source":"s","ethereum":{"chainId":560048,"queue":"q"}});
        assert_eq!(json_digest(&a).unwrap(), json_digest(&b).unwrap());
    }
    #[test]
    fn supervised_restart_preserves_commitments_and_pending_root_registrations() {
        let key = "42-0303030303030303030303030303030303030303030303030303030303030303";
        let mut roots = Map::new();
        roots.insert(
            key.to_owned(),
            json!({
                "block":42,"blockHash":"0x4242","queueId":7,"queueRoot":"0x0303",
                "status":"pending","publication":"/actor/root-publications/42-0303.json"
            }),
        );
        let before = json!({
            "rootScan":{"block":41,"blockHash":"0x0101"},
            "commitments":[{
                "block":12,"blockHash":"0x1212","rawScale":"0xab",
                "current":{"id":1},"next":{"id":2},"freshnessProof":{"leaf":"0x01"},
                "destinationBlock":100,"destinationHash":"0x6464","txHash":"0xaaaa","finalized":false
            }],
            "roots":roots
        });
        let mut after = before.clone();
        after["rootScan"]["block"] = json!(42);
        after["rootScan"]["blockHash"] = json!("0x4242");
        after["commitments"][0]["finalized"] = json!(true);
        after["roots"][key]["status"] = json!("accepted");
        assert!(ensure_actor_history_preserved(&before, &after).is_ok());
        assert!(after["roots"]
            .as_object_mut()
            .unwrap()
            .remove(key)
            .is_some());
        assert!(ensure_actor_history_preserved(&before, &after).is_err());
    }

    #[test]
    fn release_event_accepts_later_covering_root_without_changing_message_identity() {
        // Finalized Hoodi tx 09ce45a5...603c3d8: message block74716, delivery root74727.
        let data = hex::decode(concat!(
            "00000000000000000000000000000000000000000000000000000000000123e7",
            "a08c1df16afc9b5ac8744a6d569bfab0540f4940b745242a47fcd6489f65d438",
            "0000000000000000000000000000000000000000000000000000000000000005",
            "0000000000000000000000004c764da0087ac18eb06a1347e2e6da17a6570e05",
        ))
        .unwrap();
        let hash: B256 = "0xa08c1df16afc9b5ac8744a6d569bfab0540f4940b745242a47fcd6489f65d438"
            .parse()
            .unwrap();
        let manager: Address = "0x4c764da0087ac18eb06a1347e2e6da17a6570e05"
            .parse()
            .unwrap();
        let nonce = EthU256::from(5);
        assert_eq!(
            release_source_block(&data, 74716, hash, nonce, manager),
            Some(74727)
        );
        assert_eq!(
            release_source_block(&data, 74727, hash, nonce, manager),
            Some(74727)
        );
        assert_eq!(
            release_source_block(&data, 74728, hash, nonce, manager),
            None
        );
        assert_eq!(
            release_source_block(&data, 74716, B256::ZERO, nonce, manager),
            None
        );
        assert_eq!(
            release_source_block(&data, 74716, hash, EthU256::from(6), manager),
            None
        );
        let beneficiary: Address = "0x78f0625BcE3eCD3dd4189E12608A1334028e7e11"
            .parse()
            .unwrap();
        assert_eq!(
            release_source_block(&data, 74716, hash, nonce, beneficiary),
            None
        );
        assert_eq!(
            release_source_block(&data[..127], 74716, hash, nonce, manager),
            None
        );
        let mut overflowing = data.clone();
        overflowing[0] = 1;
        assert_eq!(
            release_source_block(&overflowing, 74716, hash, nonce, manager),
            None
        );
    }

    #[test]
    fn qualification_accepts_transient_following_but_rejects_failed_or_local_state() {
        let deployment = json!({"ethereum":{"chainId":560048}});
        let mut state = json!({
            "schemaVersion":3,"mode":"hoodi-token-follow","deployment":deployment,
            "activeEthereum":deployment["ethereum"].clone(),
            "localRehearsal":false,"startupSequence":1,
            "followerSigner":"0x0000000000000000000000000000000000000001",
            "rootPublisherSigner":"0x0000000000000000000000000000000000000002",
            "follower":{"status":"healthy"}
        });
        let signers = follower_signers(&state, &deployment).unwrap();
        for status in [
            "starting",
            "catching-up",
            "held-root",
            "held-finality",
            "healthy",
        ] {
            state["follower"]["status"] = json!(status);
            assert_eq!(follower_signers(&state, &deployment).unwrap(), signers);
        }
        for status in ["failed", "unknown"] {
            state["follower"]["status"] = json!(status);
            assert!(follower_signers(&state, &deployment).is_err());
        }
        state["follower"]["status"] = json!("healthy");
        state["localRehearsal"] = json!(true);
        assert!(follower_signers(&state, &deployment).is_err());
        state["localRehearsal"] = json!(false);
        state["deployment"]["ethereum"]["chainId"] = json!(1);
        assert!(follower_signers(&state, &deployment).is_err());
    }

    #[tokio::test]
    async fn beacon_coverage_waits_for_progress_but_never_renews_the_deadline() {
        let mut observations = 0;
        wait_beacon_coverage(Instant::now() + Duration::from_secs(12), || {
            observations += 1;
            std::future::ready(Ok(observations == 2))
        })
        .await
        .unwrap();
        assert_eq!(observations, 2);

        let deadline = Instant::now() + Duration::from_millis(30);
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            wait_beacon_coverage(deadline, || std::future::ready(Ok(false))),
        )
        .await
        .expect("the original deadline must also bound the polling sleep");
        assert!(result.is_err());

        let mut observations = 0;
        let result = wait_beacon_coverage(Instant::now() + Duration::from_secs(12), || {
            observations += 1;
            std::future::ready(beacon_coverage(101, 100, [1; 32], 100, Some([2; 32])))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(observations, 1);

        let deadline = Instant::now() + Duration::from_millis(30);
        let result = wait_beacon_coverage(deadline, || async {
            sleep_until(deadline.into()).await;
            Ok(true)
        })
        .await;
        assert!(result.is_err());
    }

    #[test]
    fn beacon_lag_never_hides_a_canonical_hash_disagreement() {
        assert!(!beacon_coverage(101, 100, [1; 32], 100, Some([1; 32])).unwrap());
        assert!(beacon_coverage(101, 100, [1; 32], 100, Some([2; 32])).is_err());
        assert!(!beacon_coverage(100, 100, [1; 32], 99, None).unwrap());
        assert!(beacon_coverage(100, 100, [1; 32], 100, None).is_err());
        assert!(beacon_coverage(100, 100, [1; 32], 100, Some([1; 32])).unwrap());
    }

    #[test]
    fn receipt_coverage_rejects_malformed_receipts_instead_of_skipping_them() {
        assert_eq!(receipt_coverage_height(&[]).unwrap(), None);
        assert_eq!(
            receipt_coverage_height(&[json!({"receiptBlock":12}), json!({"receiptBlock":14})])
                .unwrap(),
            Some(14)
        );
        assert!(receipt_coverage_height(&[json!({"receiptBlock":12}), json!({})]).is_err());
    }

    #[test]
    fn completed_window_requires_four_complete_asset_roundtrips_and_the_scheduled_fee_mode() {
        let t0 = 1_800_000_000_000u64;
        let asset = |fee_type: &str| {
            json!({
                "status":"passed","amountRaw":"1",
                "lock":{"milestones":{"submissionHandoffAtMs":1,"evmBlockTimestampMs":1,"evmFinalityObservedAtMs":1,"beaconCoverageObservedAtMs":1,"workerProofComposedAtMs":1,"workerProofHandoffAtMs":1,"gearFinalizedStatusObservedAtMs":1,"mintDeltaObservedAtMs":1},"receipt":{"receiptBlockHash":"0x1"},
                    "incomingReceipt":{"slot":1,"transactionIndex":0},
                    "receiptStatus":{"status":"Processed","finalizedHash":"0x2"}},
                "burn":{"milestones":{"submissionHandoffAtMs":1,"burnFinalizedObservedAtMs":1,"rootRegisteredAtMs":1,"rootMaturityEligibleAtMs":1,"rootPublicationObservedAtMs":1,"rootMaturityObservedAtMs":1,"releaseFinalityObservedAtMs":1},"outboundRequest":{"managerEventBlockHash":"0x3"},
                    "rootPublication":{"sourceBlockHash":"0x4"},
                    "releaseReceipt":{"receipt":{"receiptBlockHash":"0x5"},"finalizedHash":"0x6"}},
                "paid":{"milestones":{"submissionHandoffAtMs":1,"paymentFinalizedObservedAtMs":1},"paidEvent":{"type":fee_type,"blockHash":"0x7"}}
            })
        };
        let assets = |fee_type: &str| {
            json!({
                "USDC":asset(fee_type),"USDT":asset(fee_type),"WETH":asset(fee_type),"WBTC":asset(fee_type)
            })
        };
        let window = |hour: u8, fee_type: &str| {
            let start = t0 + u64::from(hour) * HOUR_MS;
            json!({"status":"passed","scheduledStartMs":start,"deadlineMs":start+HOUR_MS,
                "boundaryIntentPersistedAtMs":start,"startedAtMs":start,"completedAtMs":start+1,"baseline":{},
                "stages":{"afterMint":{},"finalizedReturn":{}},"assets":assets(fee_type)})
        };
        let mut journal = test_journal();
        journal.t0_ms = Some(t0);
        journal
            .windows
            .insert("hour-00".into(), window(0, "normal"));
        assert!(window_complete(&journal, 0));
        journal
            .windows
            .insert("hour-01".into(), window(1, "priority"));
        assert!(window_complete(&journal, 1));

        let mut unattributed = window(0, "normal");
        unattributed["assets"]["USDC"]["lock"]["milestones"]
            .as_object_mut()
            .unwrap()
            .remove("workerProofHandoffAtMs");
        journal.windows.insert("hour-00".into(), unattributed);
        assert!(!window_complete(&journal, 0));

        let mut missing = window(0, "normal");
        missing["assets"].as_object_mut().unwrap().remove("WBTC");
        missing["assets"]["OTHER"] = asset("normal");
        journal.windows.insert("hour-00".into(), missing);
        assert!(!window_complete(&journal, 0));

        journal
            .windows
            .insert("hour-00".into(), window(0, "priority"));
        assert!(!window_complete(&journal, 0));
        let mut late = window(0, "normal");
        late["completedAtMs"] = late["deadlineMs"].clone();
        journal.windows.insert("hour-00".into(), late);
        assert!(!window_complete(&journal, 0));
        let mut late_start = window(0, "normal");
        late_start["startedAtMs"] = json!(t0 + CLOCK_DRIFT_MS + 1);
        journal.windows.insert("hour-00".into(), late_start);
        assert!(!window_complete(&journal, 0));
    }

    #[test]
    fn scheduled_intent_does_not_allow_late_start_but_running_window_can_resume() {
        let t0 = 1_800_000_000_000u64;
        let scheduled = scheduled_window(0, t0, t0 + HOUR_MS, t0);
        assert_eq!(scheduled["boundaryIntentPersistedAtMs"].as_u64(), Some(t0));
        assert!(window_has_boundary_intent(&scheduled, t0));
        let late = scheduled_window(0, t0, t0 + HOUR_MS, t0 + 1);
        assert!(!window_has_boundary_intent(&late, t0));
        assert!(can_start_window(Some(&scheduled), t0 + CLOCK_DRIFT_MS, t0));
        assert!(!can_start_window(
            Some(&scheduled),
            t0 + CLOCK_DRIFT_MS + 1,
            t0
        ));
        let mut running = scheduled;
        running["status"] = json!("running");
        running["startedAtMs"] = json!(t0);
        assert!(can_start_window(Some(&running), t0 + HOUR_MS / 2, t0));
        running["startedAtMs"] = json!(t0 + CLOCK_DRIFT_MS + 1);
        assert!(!can_start_window(Some(&running), t0 + HOUR_MS / 2, t0));
    }

    #[test]
    fn ambiguous_gear_action_is_never_resubmitted() {
        let mut journal = test_journal();
        let intent = json!({"kind":"vft-manager-request","startBlock":17});
        journal.actions.insert(
            "request".into(),
            Action {
                status: "ambiguous".into(),
                intent: intent.clone(),
                from_block: None,
                nonce: None,
                tx_hash: None,
                evidence: json!({"dispatch":"sent without an extrinsic hash"}),
            },
        );
        assert!(
            !prepare_gear_action(&mut journal, Path::new("unused"), "request", intent).unwrap()
        );
        assert_eq!(journal.actions["request"].status, "ambiguous");
    }

    #[test]
    fn terminal_report_survives_resume_without_timestamp_rewrite() {
        let directory = std::env::temp_dir().join(format!(
            "token-soak-report-{}-{}",
            std::process::id(),
            now_ms().unwrap()
        ));
        fs::create_dir_all(&directory).unwrap();
        let journal_path = directory.join("campaign.json");
        let report_path = directory.join("qualification-report.json");
        let mut journal = test_journal();
        journal.t0_ms = Some(now_ms().unwrap());
        journal.status = "failed".into();
        write_terminal_report(&mut journal, &journal_path).unwrap();
        let original = fs::read(&report_path).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        write_terminal_report(&mut journal, &journal_path).unwrap();
        assert_eq!(fs::read(&report_path).unwrap(), original);
        assert_eq!(
            journal.terminal_report.as_ref().unwrap()["observedThroughMs"],
            serde_json::from_slice::<Value>(&original).unwrap()["observedThroughMs"]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn follower_checkpoint_uses_only_finalized_commitment_for_active_client() {
        let state = json!({
            "activeEthereum": {"client": "new"},
            "deployment": {"ethereum": {"client": "old"}},
            "bootstrap": {"block": 3},
            "commitments": [
                {"block": 40, "clientAddress": "new", "finalized": true},
                {"block": 50, "clientAddress": "new", "finalized": false},
                {"block": 60, "clientAddress": "old", "finalized": true}
            ]
        });
        assert_eq!(follower_checkpoint(&state).unwrap(), 40);
    }

    #[test]
    fn follower_checkpoint_never_falls_back_to_old_client_bootstrap() {
        let state = json!({
            "activeEthereum": {"client": "new"},
            "deployment": {"ethereum": {"client": "old"}},
            "bootstrap": {"block": 3},
            "commitments": [
                {"block": 40, "clientAddress": "old", "finalized": true}
            ]
        });
        assert!(follower_checkpoint(&state).is_err());
    }
    #[test]
    fn mined_readiness_does_not_complete_finalized_history_or_accept_other_clients() -> Result<()> {
        let client = Address::repeat_byte(1);
        let hash = B256::from(keccak256(&[1, 2, 3]));
        let mut state = json!({"activeEthereum":{"client":format!("{client:#x}")},
            "deployment":{"ethereum":{"client":format!("{client:#x}")}},
            "follower":{"lastMinedUpdate":10,"lastSuccessfulUpdate":5,"lastFinalizedUpdate":5},
            "commitments":[{"block":5,"clientAddress":format!("{client:#x}"),"finalized":true},
                {"block":10,"blockHash":format!("{:#x}",B256::repeat_byte(2)),
                 "clientAddress":format!("{client:#x}"),"finalized":false,"destinationBlock":100,
                 "destinationHash":format!("{:#x}",B256::repeat_byte(3)),"txHash":format!("{hash:#x}"),
                 "submission":{"block":10,"blockHash":format!("{:#x}",B256::repeat_byte(2)),
                 "clientAddress":format!("{client:#x}"),"nonce":7,"rawTransaction":"0x010203","txHash":format!("{hash:#x}")}}]});
        let mut checkpoint = DestinationCheckpoint {
            block: 10,
            root: [4; 32],
            source_timestamp_ms: 123,
            current_id: 1,
            current_len: 2,
            current_root: [5; 32],
            next_id: 2,
            next_len: 2,
            next_root: [6; 32],
        };
        let captured = checkpoint.clone();
        assert!(checkpoint_observation_stable(&captured, &captured)?);
        let mut advancing = captured.clone();
        advancing.block += 1;
        advancing.root = [7; 32];
        assert!(!checkpoint_observation_stable(&captured, &advancing)?);
        let mut same_height_conflict = captured.clone();
        same_height_conflict.root = [7; 32];
        assert!(checkpoint_observation_stable(&captured, &same_height_conflict).is_err());
        let mut regressed = captured.clone();
        regressed.block -= 1;
        assert!(checkpoint_observation_stable(&captured, &regressed).is_err());
        let (_, head) =
            mined_follower_entry(&state, client, &checkpoint)?.context("missing mined head")?;
        assert_eq!(head.source_block, 10);
        let original = (head.destination_block, head.destination_hash);
        assert!(observed_mined_inclusion(head, false, original)?.is_some());
        assert!(observed_mined_inclusion(head, true, original)?.is_some());
        for changed in [
            (original.0, B256::repeat_byte(8)),
            (original.0 + 1, original.1),
        ] {
            assert!(observed_mined_inclusion(head, false, changed)?.is_none());
            assert!(observed_mined_inclusion(head, true, changed).is_err());
            let recovered = MinedFollowerHead {
                destination_block: changed.0,
                destination_hash: changed.1,
                ..head
            };
            assert!(observed_mined_inclusion(recovered, true, changed)?.is_some());
        }
        assert_eq!(follower_checkpoint(&state)?, 5);
        assert!(mined_follower_entry(&state, Address::repeat_byte(9), &checkpoint).is_err());
        checkpoint.block = 11;
        assert!(mined_follower_entry(&state, client, &checkpoint)?.is_none());
        checkpoint.block = 9;
        assert!(mined_follower_entry(&state, client, &checkpoint).is_err());
        checkpoint.block = 10;
        state["commitments"][1]["submission"]["rawTransaction"] = json!("0x010204");
        assert!(mined_follower_entry(&state, client, &checkpoint).is_err());
        Ok(())
    }

    #[test]
    fn mined_root_does_not_satisfy_finalized_acceptance() -> Result<()> {
        assert!(!actor_root_accepted(&json!("pending"))?);
        assert!(!actor_root_accepted(&json!("mined"))?);
        assert!(actor_root_accepted(&json!("accepted"))?);
        assert!(actor_root_accepted(&json!("unknown")).is_err());
        Ok(())
    }

    #[test]
    fn warmup_requires_every_worker_sample_and_unchanged_terminal_accounting() -> Result<()> {
        let started = 1_000;
        let samples: Vec<_> = (0_u64..60).map(|minute| {
            let scheduled = started + minute * 60_000;
            json!({"minute":minute,"scheduledAtMs":scheduled,"atMs":scheduled + 2_000,"sourceHeight":1_000 + minute,
                "workers":{"ready":true,"observedAtMs":scheduled + 1_000}})
        }).collect();
        validate_warmup_samples(&samples, started, 60)?;
        for case in [
            "worker-stopped",
            "old-worker-observation",
            "missing-minute",
            "duplicate-minute",
            "late-minute",
            "stalled-source",
        ] {
            let mut changed = samples.clone();
            match case {
                "worker-stopped" => changed[12]["workers"]["ready"] = json!(false),
                "stalled-source" => changed
                    .iter_mut()
                    .for_each(|sample| sample["sourceHeight"] = json!(1_000)),
                "old-worker-observation" => changed[12]["workers"]["observedAtMs"] = json!(started),
                "missing-minute" => {
                    changed.remove(12);
                }
                "duplicate-minute" => changed[12] = changed[11].clone(),
                "late-minute" => changed[12]["atMs"] = json!(started + 13 * 60_000),
                _ => unreachable!(),
            }
            assert!(
                validate_warmup_samples(&changed, started, 60).is_err(),
                "{case}"
            );
        }
        let tokens: Vec<_> = ["USDC", "USDT", "WETH", "WBTC"]
            .into_iter()
            .map(|symbol| Token {
                symbol,
                component: symbol,
                address: Address::ZERO,
                peer: ActorId::zero(),
                gear_origin: false,
                native_amount: None,
                escrow: None,
            })
            .collect();
        let baseline = SnapshotSet {
            gear_height: 12,
            gear_hash: [1; 32].into(),
            evm_height: 34,
            evm_hash: [2; 32].into(),
            assets: tokens
                .iter()
                .map(|token| {
                    (
                        token.symbol.into(),
                        RawSnapshot {
                            gear_escrow: None,
                            evm_user: EthU256::from(100),
                            evm_escrow: EthU256::ZERO,
                            evm_supply: EthU256::from(100),
                            gear_user: GearU256::zero(),
                            gear_supply: GearU256::zero(),
                        },
                    )
                })
                .collect(),
        };
        let mut terminal = SnapshotSet::from_json(&baseline.to_json())?;
        terminal.gear_height += 1;
        terminal.evm_height += 1;
        verify_roundtrip_delta(&baseline, &terminal, &tokens)?;
        for token in &tokens {
            for field in 0..5 {
                let mut changed = SnapshotSet::from_json(&terminal.to_json())?;
                let state = changed.assets.get_mut(token.symbol).unwrap();
                match field {
                    0 => state.evm_user -= EthU256::from(1),
                    1 => state.evm_escrow += EthU256::from(1),
                    2 => state.evm_supply += EthU256::from(1),
                    3 => state.gear_user += GearU256::from(1),
                    4 => state.gear_supply += GearU256::from(1),
                    _ => unreachable!(),
                }
                assert!(
                    verify_roundtrip_delta(&baseline, &changed, &tokens).is_err(),
                    "{} field {field}",
                    token.symbol
                );
            }
        }
        Ok(())
    }

    #[test]
    fn dependent_lock_uses_original_mined_approval_without_beacon_delay() -> Result<()> {
        let hash = format!("{:#x}", B256::repeat_byte(1));
        let approval = Action {
            status: "mined".into(),
            intent: json!({"kind":"erc20-approve"}),
            from_block: Some(10),
            nonce: Some(7),
            tx_hash: Some(hash.clone()),
            evidence: json!({"minedReceipt":{"transactionHash":hash,"receiptBlock":11,
                "receiptBlockHash":format!("{:#x}",B256::repeat_byte(2)),"blockTimestampMs":123}}),
        };
        ensure_approval_precedes_lock(&approval, 8)?;
        assert!(approval.evidence.get("receipt").is_none());
        for case in [
            "broadcasting",
            "same-nonce",
            "earlier-nonce",
            "missing-receipt",
            "different-original-hash",
        ] {
            let mut changed = approval.clone();
            let mut lock_nonce = 8;
            match case {
                "broadcasting" => changed.status = "broadcasting".into(),
                "same-nonce" => lock_nonce = 7,
                "earlier-nonce" => lock_nonce = 6,
                "missing-receipt" => changed.evidence = json!({}),
                "different-original-hash" => {
                    changed.evidence["minedReceipt"]["transactionHash"] =
                        json!(B256::repeat_byte(3))
                }
                _ => unreachable!(),
            }
            assert!(
                ensure_approval_precedes_lock(&changed, lock_nonce).is_err(),
                "{case}"
            );
        }
        Ok(())
    }

    #[test]
    fn bootstrap_evidence_cannot_escape_its_independent_owner() -> Result<()> {
        let owner = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        fs::write(owner.path().join("original"), b"original evidence")?;
        fs::write(other.path().join("foreign"), b"foreign evidence")?;
        std::os::unix::fs::symlink(other.path().join("foreign"), owner.path().join("link"))?;
        assert_eq!(
            contained_evidence_path(owner.path(), "original")?,
            fs::canonicalize(owner.path().join("original"))?
        );
        for path in ["../foreign", "/foreign", "link"] {
            assert!(
                contained_evidence_path(owner.path(), path).is_err(),
                "{path}"
            );
        }
        Ok(())
    }

    #[test]
    fn rotation_handoff_is_durable_once_and_cannot_reset_its_deadline_or_sampling_clock(
    ) -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("campaign.json");
        let mut journal = test_journal();
        journal.warmup.status = "running".into();
        journal.warmup.evidence =
            json!({"readinessStartedAtMs":1_000,"readinessDeadlineAtMs":100_000,"samples":[]});
        let intent = json!({"authority":"Alice","nonce":7,"extrinsic_hash":vec![1;32],
            "beefy_key":vec![2;33],"signed_extrinsic":[3,4,5]});
        assert!(prepare_rotation_handoff(
            &mut journal,
            &path,
            intent.clone(),
            2_000
        )?);
        let original_bytes = fs::read(&path)?;
        let mut recovered: Journal = serde_json::from_slice(&original_bytes)?;
        assert_eq!(recovered.warmup.evidence["rotationIntent"], intent);
        assert_eq!(recovered.warmup.evidence["rotationHandoffAtMs"], 2_000);
        assert!(!prepare_rotation_handoff(
            &mut recovered,
            &path,
            intent.clone(),
            3_000
        )?);
        assert_eq!(fs::read(&path)?, original_bytes);
        assert_eq!(recovered.warmup.evidence["readinessDeadlineAtMs"], 100_000);
        for field in ["nonce", "extrinsic_hash", "beefy_key", "signed_extrinsic"] {
            let mut altered = intent.clone();
            altered[field] = json!("changed-original-identity");
            assert!(
                prepare_rotation_handoff(&mut recovered, &path, altered, 3_000).is_err(),
                "{field}"
            );
            assert_eq!(fs::read(&path)?, original_bytes);
        }
        assert!(prepare_rotation_handoff(&mut recovered, &path, intent.clone(), 100_000).is_err());
        recovered.warmup.started_at_ms = Some(3_000);
        assert!(prepare_rotation_handoff(&mut recovered, &path, intent, 4_000).is_err());
        assert_eq!(fs::read(&path)?, original_bytes);
        Ok(())
    }

    #[test]
    fn empty_event_buffers_cannot_hide_signed_payouts_or_an_incomplete_save() -> Result<()> {
        let mut status: OutboundTransactionStatus =
            serde_json::from_value(json!({"version":1,"active":{},"failed":[]}))?;
        assert!(outbound_status_idle(&status)?);
        for state in [
            "WaitForMerkleRoot",
            "SendMessage",
            "WaitConfirmations",
            "Failed",
            "NeedsReconciliation",
        ] {
            status
                .active
                .insert("original-worker-UUID".into(), state.into());
            assert!(
                !outbound_status_idle(&status)?,
                "{state}: paired/empty gear_events must not qualify an outstanding payout"
            );
            status.active.clear();
        }
        status.failed.push("original-failed-UUID".into());
        assert!(!outbound_status_idle(&status)?);
        status.failed.clear();
        status.version = 2;
        assert!(outbound_status_idle(&status).is_err());
        assert!(serde_json::from_value::<OutboundTransactionStatus>(
            json!({"version":1,"failed":[]})
        )
        .is_err());
        let directory = tempfile::tempdir()?;
        fs::create_dir(directory.path().join(".state"))?;
        assert!(outbound_save_complete(directory.path())?);
        for fragment in [
            ".state/save.pending",
            ".state/original-uuid.new",
            "blocks.json.new",
        ] {
            fs::write(
                directory.path().join(fragment),
                b"uncommitted original snapshot",
            )?;
            assert!(!outbound_save_complete(directory.path())?, "{fragment}");
            fs::remove_file(directory.path().join(fragment))?;
        }
        assert!(outbound_save_complete(directory.path())?);
        Ok(())
    }
}
