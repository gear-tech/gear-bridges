use alloy::{
    consensus::{transaction::SignerRecoverable as _, Transaction as _, TxEnvelope},
    eips::eip2718::{Decodable2718, Encodable2718},
    providers::Provider,
    rpc::types::{Block, TransactionReceipt},
    sol_types::SolEvent,
};
use alloy_eips::BlockId;
use alloy_primitives::B256;
use anyhow::{anyhow, bail, ensure, Context, Result};
use clap::Args;
use cli_utils::GearConnectionArgs;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{
    abi::{BridgingPayment, IERC20Manager},
    PollingEthApi,
};
use ethereum_common::tree_hash::TreeHash;
use gclient::GearApi;
use gear_common::api_provider::{ApiProvider, ApiProviderConnection};
use gear_core::{
    ids::{ActorId, CodeId},
    program::ProgramState,
};
use primitive_types::{H160, H256};
use relayer::message_relayer::eth_to_gear::{
    cutover::{
        self as seal, is_undispatched_old_admission as never_dispatched, CutoverBoundary,
        CutoverSeal, FinalizedCutoverStatus,
    },
    message_sender::{
        FinalizedReceiptObservation, MessageSender, MessageStatus, Response, SignedSubmission,
        SubmissionEvidence,
    },
    proof_composer::{authenticate_cutover_boundary, compose, validate_receipt_set},
    storage::{InboundRuntimeIdentity, JSONStorage, Storage},
    tx_manager::{Transaction, TransactionManager},
};
use sails_rs::{
    calls::{ActionIo, Query},
    gclient::calls::{GClientRemoting, QueryExtGClient},
    Decode, Encode,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;
use vft_manager_client::{
    traits::VftManager as _, vft_manager::io, CutoverPhase, CutoverStatus, TokenSupply, VftManager,
};

#[derive(Args)]
pub struct PrepareCutover {
    #[arg(long)]
    pub old_vft_manager: ActorId,
    #[arg(long)]
    pub new_vft_manager: ActorId,
    #[arg(long)]
    pub new_manager_code_id: CodeId,
    #[arg(long)]
    pub old_transactions: PathBuf,
    #[arg(long)]
    pub old_discovery: PathBuf,
    #[arg(long)]
    pub ethereum_rpc: String,
    #[arg(long)]
    pub beacon_rpc: String,
    #[arg(long)]
    pub boundary_slot: u64,
    #[arg(long)]
    pub cutover_dir: PathBuf,
    #[arg(long)]
    pub sponsor_unpaid: bool,
}
#[derive(Args)]
pub struct FinalizeCutover {
    #[arg(long)]
    pub cutover_dir: PathBuf,
    #[arg(long)]
    pub new_transactions: PathBuf,
    #[arg(long)]
    pub new_discovery: PathBuf,
    #[arg(long)]
    pub ethereum_rpc: String,
    #[arg(long)]
    pub beacon_rpc: String,
    /// Actual authorized registration transaction; no EVM signing is performed here.
    #[arg(long)]
    pub evm_registration_transaction: Option<B256>,
    #[arg(long)]
    pub sponsor_unpaid: bool,
}
#[derive(Args)]
pub struct VerifyCutover {
    #[arg(long)]
    pub seal: PathBuf,
    #[arg(long)]
    pub ethereum_rpc: String,
    #[arg(long)]
    pub beacon_rpc: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    old_identity: InboundRuntimeIdentity,
    new_identity: InboundRuntimeIdentity,
    new_manager_code_id: H256,
    old_transactions: PathBuf,
    old_discovery: PathBuf,
    source_receipt_floor: u64,
    boundary: CutoverBoundary,
    boundary_execution_timestamp: u64,
    config_fingerprint: H256,
    erc20_manager: H160,
    native_wrapper: Option<H256>,
    initial_role_pins: Vec<RolePin>,
    mappings: Vec<(H256, H160, u8)>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CutoverStep {
    BeginCutover,
    SourcePause,
    PrepareCutover,
    Upgrade(u32),
    WrapperPause,
    WrapperConfigureManager,
    WrapperSetBurner,
    WrapperResume,
    TokenMinter(H256),
    TokenBurner(H256),
    SourceRefund(H256, u32),
    FinalizeCutover,
    DestinationUnpause,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinalizedCutoverCall {
    step: CutoverStep,
    attempt: u32,
    target: H256,
    payload: Vec<u8>,
    gas: u64,
    value: u128,
    original: SignedSubmission,
    /// A failed original is not a successful lifecycle operation.
    archived_no_effect: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoveredBlock {
    number: u64,
    hash: H256,
    parent_hash: H256,
    receipts_root: H256,
    transactions: Vec<B256>,
    receipts: Vec<TransactionReceipt>,
    header: alloy::consensus::Header,
    raw_transactions: Vec<Vec<u8>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    receipt_key: (u64, u64),
    original_tx_hash: B256,
    original_block_hash: H256,
    fee_emitter: Option<H160>,
    admission: String,
    fee_event_observed: bool,
    proof: Vec<u8>,
    signed: Option<SignedSubmission>,
    attempts: Vec<SignedSubmission>,
    settlement: Option<FinalizedReceiptObservation>,
    original_lane_uuid: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_original: Option<seal::CanonicalExternalOriginal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    undispatched_old_admission: Option<Uuid>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    hash: B256,
    block: Block,
    raw_transactions: Vec<Vec<u8>>,
    receipts: Vec<TransactionReceipt>,
    included_number: u64,
    included_hash: B256,
    finalized_number: u64,
    finalized_hash: B256,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RolePin {
    token: H256,
    block_number: u32,
    block_hash: H256,
    minter: H256,
    burner: H256,
    manager: Option<H256>,
    outstanding_redemptions: Option<u64>,
    paused: bool,
}
async fn read_role_pin(
    api: &GearApi,
    token: H256,
    wrapper: bool,
    at: Option<H256>,
) -> Result<RolePin> {
    use vft_client::traits::VftAdmin as _;
    use vft_vara_client::traits::NativeEscrow as _;
    let at = match at {
        Some(hash) => hash.0.into(),
        None => api.last_block_hash().await?,
    };
    let pin = sails_rs::H256::from_slice(at.as_ref());
    let admin = vft_client::VftAdmin::new(GClientRemoting::new(api.clone()));
    let minter = hash_actor(admin.minter().at_block(pin).recv(actor(token)).await?);
    let burner = hash_actor(admin.burner().at_block(pin).recv(actor(token)).await?);
    let paused = admin.is_paused().at_block(pin).recv(actor(token)).await?;
    let (manager, outstanding_redemptions) = if wrapper {
        let escrow = vft_vara_client::NativeEscrow::new(GClientRemoting::new(api.clone()));
        (
            escrow
                .manager()
                .at_block(pin)
                .recv(actor(token))
                .await?
                .map(hash_actor),
            Some(
                escrow
                    .outstanding_redemptions()
                    .at_block(pin)
                    .recv(actor(token))
                    .await?,
            ),
        )
    } else {
        (None, None)
    };
    Ok(RolePin {
        token,
        block_number: api.block_number_at(at).await?,
        block_hash: H256::from_slice(at.as_ref()),
        minter,
        burner,
        manager,
        outstanding_redemptions,
        paused,
    })
}
async fn record_role_pin(api: &GearApi, path: &Path, token: H256, wrapper: bool) -> Result<()> {
    let mut work = read_work(path)?;
    if let Some(previous) = work.role_pins.iter().find(|pin| pin.token == token) {
        ensure!(
            api.get_block_hash(previous.block_number).await?.as_ref()
                == previous.block_hash.as_bytes()
                && read_role_pin(api, token, wrapper, Some(previous.block_hash)).await?
                    == *previous,
            "HOLD: saved role evidence changed"
        );
        return Ok(());
    }
    work.role_pins
        .push(read_role_pin(api, token, wrapper, None).await?);
    save(path, &work)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Work {
    schema_version: u32,
    intent: Intent,
    blocks: Vec<CoveredBlock>,
    admissions: Vec<Admission>,
    calls: Vec<FinalizedCutoverCall>,
    role_pins: Vec<RolePin>,
    registration: Option<Registration>,
    retirement: Option<FinalizedCutoverStatus>,
    readiness: Option<FinalizedCutoverStatus>,
    new_transactions: Option<PathBuf>,
    new_discovery: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageReport {
    schema_version: u32,
    intent: Intent,
    blocks: Vec<CoveredBlock>,
    admissions: Vec<Admission>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleReport {
    schema_version: u32,
    intent: Intent,
    calls: Vec<FinalizedCutoverCall>,
    role_pins: Vec<RolePin>,
    registration: Registration,
}

fn hash_actor(id: ActorId) -> H256 {
    H256(id.into_bytes())
}
fn lock_other_signers(
    api: &GearApi,
    intent: &Intent,
    work: &Work,
    path: &Path,
) -> Result<Vec<([u8; 32], File, bool)>> {
    let current: [u8; 32] = api.account_id().clone().into();
    let mut senders = BTreeSet::from([intent.old_identity.gear_sender]);
    let lifecycle_senders = work
        .calls
        .iter()
        .map(|call| &call.original)
        .chain(
            work.admissions
                .iter()
                .flat_map(|admission| admission.signed.iter().chain(admission.attempts.iter())),
        )
        .map(|original| {
            original
                .sender
                .parse::<subxt::utils::AccountId32>()
                .map(|account| account.0)
        })
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    senders.extend(lifecycle_senders.iter().copied());
    senders.remove(&current);
    senders
        .into_iter()
        .map(|sender| {
            let durable = lifecycle_senders.contains(&sender);
            let file = if durable {
                seal::lock_cutover_signer(intent.old_identity.gear_genesis_hash, sender, path)?
            } else {
                seal::lock_signer(intent.old_identity.gear_genesis_hash, sender)?
            };
            Ok((sender, file, durable))
        })
        .collect()
}
fn release_owned_signers(
    api: &GearApi,
    path: &Path,
    current: &File,
    others: &[([u8; 32], File, bool)],
) -> Result<()> {
    let work = read_work(path)?;
    ensure!(
        work.calls.iter().all(|call| complete(&call.original)
            || terminal_upgrade(call)
            || (call.archived_no_effect && no_effect(&call.original))),
        "HOLD: unresolved lifecycle original retains signer ownership"
    );
    ensure!(
        work.admissions
            .iter()
            .all(|admission| admission.signed.is_none() || admission.settlement.is_some()),
        "HOLD: unresolved sponsored original retains signer ownership"
    );
    for (sender, file, durable) in others {
        if *durable {
            seal::release_cutover_signer(
                work.intent.old_identity.gear_genesis_hash,
                *sender,
                path,
                file,
            )?;
        }
    }
    seal::release_cutover_signer(
        work.intent.old_identity.gear_genesis_hash,
        api.account_id().clone().into(),
        path,
        current,
    )
}

fn actor(id: H256) -> ActorId {
    id.0.into()
}
fn work_path(directory: &Path) -> PathBuf {
    directory.join("cutover-work.json")
}
fn read_work(path: &Path) -> Result<Work> {
    ensure!(
        !PathBuf::from(format!("{}.new", path.display())).exists(),
        "HOLD: interrupted cutover work evidence"
    );
    let work: Work = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        work.schema_version == 1,
        "HOLD: unsupported cutover work schema"
    );
    Ok(work)
}
fn atomic_json<T: Serialize>(path: &Path, value: &T, immutable: bool) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if immutable && path.exists() {
        ensure!(
            fs::read(path)? == bytes,
            "HOLD: immutable cutover evidence changed"
        );
        return Ok(());
    }
    let temporary = PathBuf::from(format!("{}.new", path.display()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .context("HOLD: interrupted cutover write; preserve the original and pending evidence")?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    if immutable {
        fs::hard_link(&temporary, path)?;
        fs::remove_file(&temporary)?;
    } else {
        fs::rename(&temporary, path)?;
    }
    File::open(path.parent().context("Missing cutover parent")?)?.sync_all()?;
    Ok(())
}
fn save(path: &Path, work: &Work) -> Result<()> {
    atomic_json(path, work, false)
}
async fn release_draining_hold(
    api: &GearApi,
    path: &Path,
    current: &File,
    others: &[([u8; 32], File, bool)],
) -> Result<bool> {
    let work = read_work(path)?;
    if !work.calls.iter().all(|call| {
        complete(&call.original)
            || terminal_upgrade(call)
            || (call.archived_no_effect && no_effect(&call.original))
    }) || work.admissions.iter().any(|admission| {
        admission.settlement.is_none()
            && (admission.signed.is_some() || admission.external_original.is_some())
    }) {
        return Ok(false);
    }
    let (state, _) = status(api, work.intent.old_identity.vft_manager_address).await?;
    if state.phase != CutoverPhase::Draining
        || state.successor != Some(actor(work.intent.new_identity.vft_manager_address))
        || state.last_receipt_slot != Some(work.intent.boundary.slot)
        || state.config_fingerprint != Some(work.intent.config_fingerprint.0.into())
    {
        return Ok(false);
    }
    let signer: gsdk::signer::Signer = api.clone().into();
    let scanner = sender(
        ApiProviderConnection::from_api(signer.api().clone()),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    for call in &work.calls {
        verify_lifecycle_original(api, &scanner, &work.intent, call).await?;
    }
    for admission in &work.admissions {
        let mut bytes = admission.proof.as_slice();
        let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
        ensure!(
            bytes.is_empty(),
            "HOLD: owned admission proof encoding changed"
        );
        for archived in &admission.attempts {
            verify_proof_attempt(&scanner, &proof, archived).await?;
        }
        if let Some(signed) = &admission.signed {
            scanner
                .verify_saved_completion(
                    api,
                    &proof,
                    signed,
                    admission
                        .settlement
                        .as_ref()
                        .context("HOLD: owned proof settlement pending")?,
                )
                .await?;
        }
    }
    release_owned_signers(api, path, current, others)?;
    Ok(true)
}
fn lock_work(directory: &Path) -> Result<File> {
    fs::create_dir_all(directory)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("cutover.lock"))?;
    file.try_lock()
        .context("HOLD: another operator owns cutover work")?;
    Ok(file)
}
fn lock_signer(path: &Path, api: &GearApi, genesis: H256) -> Result<File> {
    seal::lock_cutover_signer(genesis, api.account_id().clone().into(), path)
}
async fn provider(connection: &GearConnectionArgs) -> Result<ApiProviderConnection> {
    let provider = ApiProvider::new(
        connection.get_endpoint()?,
        connection.max_reconnect_attempts,
    )
    .await?;
    let connection = provider.connection();
    // Connections retain the API; no background admission/discovery task is launched.
    Ok(connection)
}
fn sender(
    provider: ApiProviderConnection,
    identity: &InboundRuntimeIdentity,
    manager: Option<H160>,
) -> MessageSender {
    MessageSender::new(
        identity.vft_manager_address,
        io::SubmitReceipt::ROUTE.to_vec(),
        identity.historical_proxy_address,
        provider,
        String::new(),
        manager,
    )
}
async fn status(api: &GearApi, id: H256) -> Result<(CutoverStatus, FinalizedCutoverStatus)> {
    let at = api.last_block_hash().await?;
    let number = api.block_number_at(at).await?;
    let result = VftManager::new(GClientRemoting::new(api.clone()))
        .cutover_status()
        .at_block(sails_rs::H256::from_slice(at.as_ref()))
        .recv(actor(id))
        .await?;
    let pin = FinalizedCutoverStatus {
        block_number: number,
        block_hash: H256::from_slice(at.as_ref()),
        status: result.encode(),
    };
    Ok((result, pin))
}
async fn authenticate_environment(
    api: &GearApi,
    eth: &PollingEthApi,
    intent: &Intent,
) -> Result<()> {
    let signer: gsdk::signer::Signer = api.clone().into();
    ensure!(
        signer.api().genesis_hash().as_ref() == intent.old_identity.gear_genesis_hash.as_bytes(),
        "HOLD: Gear genesis changed"
    );
    ensure!(
        api.get_block_hash(0).await?.as_ref() == intent.old_identity.gear_genesis_hash.as_bytes(),
        "HOLD: live Gear genesis changed"
    );
    ensure!(
        eth.chain_id().await? == intent.old_identity.ethereum_chain_id,
        "HOLD: Ethereum chain changed"
    );
    let genesis = eth.get_block(0).await?;
    ensure!(
        genesis.header.hash.as_slice() == intent.old_identity.ethereum_genesis_hash.as_bytes(),
        "HOLD: Ethereum genesis changed"
    );
    let at = api.last_block_hash().await?;
    let replacement = api
        .program_at(actor(intent.new_identity.vft_manager_address), Some(at))
        .await?;
    ensure!(
        replacement.state == ProgramState::Initialized
            && replacement.code_id.into_bytes() == intent.new_manager_code_id.0,
        "HOLD: independently approved replacement code differs"
    );
    ensure!(
        eth.is_finalized_block(
            intent.boundary.execution_number,
            intent.boundary.execution_hash
        )
        .await?,
        "HOLD: boundary execution block is not canonical finalized"
    );
    Ok(())
}
async fn authenticate_boundary(
    api: &GearApi,
    beacon: &BeaconClient,
    intent: &Intent,
) -> Result<()> {
    let block = authenticate_cutover_boundary(
        beacon,
        api,
        actor(intent.old_identity.checkpoint_light_client_address),
        intent.boundary.slot,
    )
    .await?;
    let execution = &block.body.execution_payload;
    ensure!(
        block.tree_hash_root().as_bytes() == intent.boundary.beacon_root.as_bytes()
            && execution.block_number == intent.boundary.execution_number
            && execution.block_hash.0 .0 == intent.boundary.execution_hash.0
            && execution.timestamp == intent.boundary_execution_timestamp,
        "HOLD: authenticated boundary changed"
    );
    Ok(())
}

fn no_effect(original: &SignedSubmission) -> bool {
    (original.inclusion_block_number.is_none()
        && original.message_id.is_none()
        && original
            .finalized_nonce_consumed
            .is_some_and(|nonce| nonce > original.nonce))
        || (original.inclusion_block_number.is_some()
            && original.message_id.is_none()
            && original.finalized_dispatch_error.is_some())
}
fn complete(original: &SignedSubmission) -> bool {
    original.message_id.is_some()
        && original.finalized_dispatch_error.is_none()
        && original.finalized_reply_error.is_none()
        && original.finalized_reply.is_some()
}
fn exact_reply<A: ActionIo>(bytes: &[u8]) -> Result<A::Reply> {
    let mut input = bytes
        .strip_prefix(A::ROUTE)
        .context("HOLD: lifecycle reply route changed")?;
    let value = A::Reply::decode(&mut input)?;
    ensure!(input.is_empty(), "HOLD: trailing lifecycle reply bytes");
    Ok(value)
}
fn validate_call_reply(call: &FinalizedCutoverCall) -> Result<()> {
    let bytes = &call
        .original
        .finalized_reply
        .as_ref()
        .context("HOLD: original call reply missing")?
        .payload;
    match call.step {
        CutoverStep::BeginCutover => {
            exact_reply::<io::BeginCutover>(bytes)?;
        }
        CutoverStep::SourcePause => {
            exact_reply::<io::Pause>(bytes)?;
        }
        CutoverStep::PrepareCutover => {
            exact_reply::<io::PrepareCutover>(bytes)?;
        }
        CutoverStep::Upgrade(_) => {
            exact_reply::<io::Upgrade>(bytes)?;
        }
        CutoverStep::FinalizeCutover => {
            exact_reply::<io::FinalizeCutover>(bytes)?;
        }
        CutoverStep::DestinationUnpause => {
            exact_reply::<io::Unpause>(bytes)?;
        }
        CutoverStep::WrapperPause => {
            exact_reply::<vft_vara_client::vft_admin::io::Pause>(bytes)?;
        }
        CutoverStep::WrapperConfigureManager => {
            exact_reply::<vft_vara_client::native_escrow::io::ConfigureManager>(bytes)?;
        }
        CutoverStep::WrapperSetBurner => {
            exact_reply::<vft_vara_client::vft_admin::io::SetBurner>(bytes)?;
        }
        CutoverStep::WrapperResume => {
            exact_reply::<vft_vara_client::vft_admin::io::Resume>(bytes)?;
        }
        CutoverStep::TokenMinter(_) => {
            exact_reply::<vft_client::vft_admin::io::SetMinter>(bytes)?;
        }
        CutoverStep::TokenBurner(_) => {
            exact_reply::<vft_client::vft_admin::io::SetBurner>(bytes)?;
        }
        CutoverStep::SourceRefund(_, _) => {
            exact_reply::<io::HandleRequestBridgingInterruptedTransfer>(bytes)?
                .map_err(|e| anyhow!("HOLD: source refund failed: {e:?}"))?;
        }
    }
    Ok(())
}
async fn verify_lifecycle_original(
    api: &GearApi,
    scanner: &MessageSender,
    intent: &Intent,
    original: &FinalizedCutoverCall,
) -> Result<()> {
    validate_fixed_intent(intent, original)?;
    let mut observed = original.clone();
    if original.original.inclusion_block_number.is_none() {
        observed.original.scanned_finalized_number = original.original.prepared_finalized_number;
        observed.original.scanned_finalized_hash =
            original.original.prepared_finalized_hash.clone();
    }
    reconcile_call(
        api,
        &scanner_for(scanner, original.target),
        &mut observed,
        false,
    )
    .await?;
    ensure!(
        observed.original.raw_extrinsic == original.original.raw_extrinsic
            && observed.original.inclusion_block_number == original.original.inclusion_block_number
            && observed.original.inclusion_block_hash == original.original.inclusion_block_hash
            && observed.original.message_id == original.original.message_id
            && observed.original.finalized_dispatch_error
                == original.original.finalized_dispatch_error
            && observed.original.finalized_reply == original.original.finalized_reply
            && observed.original.finalized_reply_error == original.original.finalized_reply_error,
        "HOLD: original lifecycle evidence unavailable/changed"
    );
    if let Some(nonce) = original.original.finalized_nonce_consumed {
        let chain = scanner.api_provider.client();
        let pin = H256::from_str(&original.original.scanned_finalized_hash)?;
        ensure!(
            chain
                .block_number_to_hash(original.original.scanned_finalized_number)
                .await?
                == pin
                && u64::from(
                    chain
                        .api
                        .info_at(&original.original.sender, Some(pin))
                        .await?
                        .nonce
                ) == nonce
                && nonce > original.original.nonce,
            "HOLD: archived original nonce disposition changed"
        );
    }
    if original.archived_no_effect {
        ensure!(
            no_effect(&observed.original),
            "HOLD: archived original failure no longer authenticated"
        );
    } else if !terminal_upgrade(&observed) && !definite_refund_rejection(&observed) {
        ensure!(
            complete(&observed.original),
            "HOLD: original lifecycle reply unavailable"
        );
        validate_call_reply(&observed)?;
    }
    Ok(())
}
async fn verify_activation(
    api: &GearApi,
    scanner: &MessageSender,
    intent: &Intent,
    seal_path: &Path,
) -> Result<bool> {
    let path = seal_path
        .parent()
        .context("Seal parent missing")?
        .join("cutover-activation.json");
    ensure!(
        !PathBuf::from(format!("{}.new", path.display())).exists(),
        "HOLD: interrupted activation evidence"
    );
    if !path.exists() {
        return Ok(false);
    }
    let (digest, call, pin): (H256, FinalizedCutoverCall, FinalizedCutoverStatus) =
        serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        digest == seal::load_seal(seal_path)?.1
            && call.step == CutoverStep::DestinationUnpause
            && complete(&call.original),
        "HOLD: activation seal/original mismatch"
    );
    verify_lifecycle_original(api, scanner, intent, &call).await?;
    let finalized = api.block_number_at(api.last_block_hash().await?).await?;
    ensure!(
        pin.block_number <= finalized
            && api.get_block_hash(pin.block_number).await?.as_ref() == pin.block_hash.as_bytes(),
        "HOLD: activation pin no longer canonical finalized"
    );
    let mut bytes = pin.status.as_slice();
    let state = CutoverStatus::decode(&mut bytes)?;
    let historical = VftManager::new(GClientRemoting::new(api.clone()))
        .cutover_status()
        .at_block(sails_rs::H256::from_slice(pin.block_hash.as_bytes()))
        .recv(actor(intent.new_identity.vft_manager_address))
        .await?;
    ensure!(
        bytes.is_empty()
            && historical.encode() == pin.status
            && state.phase == CutoverPhase::Active
            && state.predecessor == Some(actor(intent.old_identity.vft_manager_address))
            && state.minimum_receipt_slot
                == intent
                    .boundary
                    .slot
                    .checked_add(1)
                    .context("Slot overflow")?
            && state.config_fingerprint == Some(intent.config_fingerprint.0.into())
            && call
                .original
                .finalized_reply
                .as_ref()
                .and_then(|reply| reply.observation.finalized_block_number)
                .is_some_and(|number| number <= pin.block_number),
        "HOLD: authenticated activation state/order differs"
    );
    ensure!(
        fs::read(&path)? == serde_json::to_vec(&(digest, call, pin))?,
        "HOLD: activation bytes changed during verification"
    );
    Ok(true)
}
async fn reconcile_call(
    api: &GearApi,
    scanner: &MessageSender,
    call: &mut FinalizedCutoverCall,
    broadcast: bool,
) -> Result<()> {
    let chain = scanner.api_provider.client();
    ensure!(
        call.original.chain_genesis_hash == format!("{:#x}", chain.api.genesis_hash())
            && call.original.manager == format!("{:#x}", call.target)
            && call.original.payload_hash
                == format!("{:#x}", alloy_primitives::keccak256(&call.payload))
            && call.value == 0,
        "HOLD: original lifecycle identity changed"
    );
    let decoded = subxt::ext::subxt_core::blocks::Extrinsics::<gsdk::GearConfig>::decode_from(
        vec![call.original.raw_extrinsic.clone()],
        chain.api.metadata(),
    )?;
    let raw = decoded
        .iter()
        .next()
        .context("HOLD: original signed envelope missing")?;
    let expected = chain
        .api
        .tx()
        .call_data(&gsdk::gear::tx().gear().send_message(
            actor(call.target),
            call.payload.clone(),
            call.gas,
            call.value,
            false,
        ))?;
    ensure!(
        raw.call_bytes() == expected,
        "HOLD: original lifecycle gas/value/call bytes changed"
    );
    let found = scanner
        .find_signed_extrinsic(&mut call.original, call.target, &call.payload)
        .await?;
    if found {
        if let Some(id) = &call.original.message_id {
            let id = H256::from_str(id)?;
            scanner
                .find_finalized_reply(&mut call.original, id, call.target)
                .await?;
        }
    } else {
        let latest = H256::from_str(&call.original.scanned_finalized_hash)?;
        let account = chain
            .api
            .info_at(&call.original.sender, Some(latest))
            .await?;
        if u64::from(account.nonce) > call.original.nonce {
            call.original.finalized_nonce_consumed = Some(u64::from(account.nonce));
        } else if broadcast {
            let signer: gsdk::signer::Signer = api.clone().into();
            subxt::tx::SubmittableTransaction::<gsdk::GearConfig, _>::from_bytes(
                std::ops::Deref::deref(signer.api()).clone(),
                call.original.raw_extrinsic.clone(),
            )
            .submit()
            .await
            .context("HOLD: identical original broadcast unknown; retain original and reconcile")?;
        }
    }
    Ok(())
}
fn terminal_upgrade(call: &FinalizedCutoverCall) -> bool {
    matches!(call.step, CutoverStep::Upgrade(_))
        && call.original.inclusion_block_number.is_some()
        && call.original.message_id.is_some()
        && call.original.finalized_reply.is_some()
        && call.original.finalized_reply_error.is_some()
}

fn definite_refund_rejection(call: &FinalizedCutoverCall) -> bool {
    matches!(call.step, CutoverStep::SourceRefund(_, _))
        && complete(&call.original)
        && call.original.finalized_reply.as_ref().is_some_and(|reply| {
            matches!(
                exact_reply::<io::HandleRequestBridgingInterruptedTransfer>(&reply.payload),
                Ok(Err(vft_manager_client::Error::MessageFailed))
            )
        })
}
fn validate_fixed_intent(intent: &Intent, call: &FinalizedCutoverCall) -> Result<()> {
    let old = actor(intent.old_identity.vft_manager_address);
    let new = actor(intent.new_identity.vft_manager_address);
    let (target, payload) = match call.step {
        CutoverStep::BeginCutover => (
            old,
            io::BeginCutover::encode_call(new, intent.boundary.slot),
        ),
        CutoverStep::SourcePause => (old, io::Pause::encode_call()),
        CutoverStep::PrepareCutover => (
            new,
            io::PrepareCutover::encode_call(old, intent.boundary.slot),
        ),
        CutoverStep::Upgrade(_) => (old, io::Upgrade::encode_call(new)),
        CutoverStep::FinalizeCutover => (new, io::FinalizeCutover::encode_call()),
        CutoverStep::DestinationUnpause => (new, io::Unpause::encode_call()),
        CutoverStep::SourceRefund(request, _) => (
            old,
            io::HandleRequestBridgingInterruptedTransfer::encode_call(request.0.into()),
        ),
        CutoverStep::TokenMinter(token) => {
            ensure!(
                intent.native_wrapper == Some(token)
                    || intent.mappings.iter().any(|(id, _, _)| *id == token),
                "HOLD: unbound token role"
            );
            (
                actor(token),
                vft_client::vft_admin::io::SetMinter::encode_call(new),
            )
        }
        CutoverStep::TokenBurner(token) => {
            ensure!(
                intent.mappings.iter().any(|(id, _, _)| *id == token),
                "HOLD: unbound token role"
            );
            (
                actor(token),
                vft_client::vft_admin::io::SetBurner::encode_call(new),
            )
        }
        CutoverStep::WrapperPause => (
            actor(
                intent
                    .native_wrapper
                    .context("HOLD: missing native policy")?,
            ),
            vft_vara_client::vft_admin::io::Pause::encode_call(),
        ),
        CutoverStep::WrapperConfigureManager => (
            actor(
                intent
                    .native_wrapper
                    .context("HOLD: missing native policy")?,
            ),
            vft_vara_client::native_escrow::io::ConfigureManager::encode_call(new),
        ),
        CutoverStep::WrapperSetBurner => (
            actor(
                intent
                    .native_wrapper
                    .context("HOLD: missing native policy")?,
            ),
            vft_vara_client::vft_admin::io::SetBurner::encode_call(new),
        ),
        CutoverStep::WrapperResume => (
            actor(
                intent
                    .native_wrapper
                    .context("HOLD: missing native policy")?,
            ),
            vft_vara_client::vft_admin::io::Resume::encode_call(),
        ),
    };
    ensure!(
        call.target == hash_actor(target)
            && call.payload == payload
            && call.gas > 0
            && call.value == 0,
        "HOLD: fixed lifecycle intent differs from immutable cutover"
    );
    Ok(())
}
async fn verify_proof_attempt(
    scanner: &MessageSender,
    proof: &eth_events_electra_client::EthToVaraEvent,
    signed: &SignedSubmission,
) -> Result<()> {
    use historical_proxy_client::historical_proxy::io::Redirect;
    let call = Redirect::encode_call(
        proof.proof_block.block.slot,
        proof.encode(),
        actor(scanner.receiver_address),
        scanner.receiver_route.clone(),
    );
    let mut observed = signed.clone();
    if signed.inclusion_block_number.is_none() {
        observed.scanned_finalized_number = signed.prepared_finalized_number;
        observed.scanned_finalized_hash = signed.prepared_finalized_hash.clone();
    }
    let found = scanner
        .find_signed_extrinsic(&mut observed, scanner.historical_proxy_address, &call)
        .await?;
    if let Some(nonce) = signed.finalized_nonce_consumed {
        let chain = scanner.api_provider.client();
        let hash = H256::from_str(&signed.scanned_finalized_hash)?;
        ensure!(
            !found
                && chain
                    .block_number_to_hash(signed.scanned_finalized_number)
                    .await?
                    == hash
                && u64::from(chain.api.info_at(&signed.sender, Some(hash)).await?.nonce) == nonce
                && nonce > signed.nonce,
            "HOLD: original proof absence/nonce disposition changed"
        );
    } else {
        ensure!(
            found
                && observed.inclusion_block_number == signed.inclusion_block_number
                && observed.inclusion_block_hash == signed.inclusion_block_hash
                && observed.message_id == signed.message_id
                && observed.finalized_dispatch_error == signed.finalized_dispatch_error
                && observed.message_id.is_none()
                && observed.finalized_dispatch_error.is_some(),
            "HOLD: archived original proof was not a definite pre-message failure"
        );
    }
    Ok(())
}
async fn reconcile_original(
    api: &GearApi,
    scanner: &MessageSender,
    proof: &eth_events_electra_client::EthToVaraEvent,
    signed: &mut SignedSubmission,
    pin: Option<&FinalizedReceiptObservation>,
) -> Result<(MessageStatus, Option<std::result::Result<Vec<u8>, String>>)> {
    if signed.sender == api.account_id().to_string() || pin.is_some() {
        return scanner
            .reconcile_signed_submission(api, proof, signed, pin)
            .await;
    }
    use historical_proxy_client::historical_proxy::io::Redirect;
    let payload = Redirect::encode_call(
        proof.proof_block.block.slot,
        proof.encode(),
        actor(scanner.receiver_address),
        scanner.receiver_route.clone(),
    );
    let found = scanner
        .find_signed_extrinsic(signed, scanner.historical_proxy_address, &payload)
        .await?;
    if !found {
        let chain = scanner.api_provider.client();
        let hash = H256::from_str(&signed.scanned_finalized_hash)?;
        let nonce = u64::from(chain.api.info_at(&signed.sender, Some(hash)).await?.nonce);
        if nonce > signed.nonce {
            signed.finalized_nonce_consumed = Some(nonce);
            return Ok((
                MessageStatus::RetryableNoop {
                    receipt_key: signed.receipt_key,
                    diagnostic: "HOLD: original absent and nonce consumed".into(),
                    receipt_observation: Some(FinalizedReceiptObservation {
                        finalized_block_number: Some(signed.scanned_finalized_number),
                        finalized_block_hash: signed.scanned_finalized_hash.clone(),
                    }),
                },
                None,
            ));
        }
        subxt::tx::SubmittableTransaction::<gsdk::GearConfig, _>::from_bytes(
            std::ops::Deref::deref(&chain.api).clone(),
            signed.raw_extrinsic.clone(),
        )
        .submit()
        .await?;
    } else if let Some(id) = signed
        .message_id
        .as_deref()
        .map(H256::from_str)
        .transpose()?
    {
        scanner
            .find_finalized_reply(signed, id, scanner.historical_proxy_address)
            .await?;
        let (settled, _) = scanner.settlement_status(api, proof, None).await?;
        if let MessageStatus::Success {
            receipt_observation: Some(pin),
        } = settled
        {
            return scanner
                .reconcile_signed_submission(api, proof, signed, Some(&pin))
                .await;
        }
    }
    Ok((MessageStatus::NeedsReconciliation{receipt_key:signed.receipt_key,diagnostic:"HOLD: exact original retained; native continuation requires original lane signer, resume original relayer during Draining".into(),receipt_observation:None},None))
}
async fn bind_original_storage(
    storage: &JSONStorage,
    identity: &InboundRuntimeIdentity,
    directory: &Path,
) -> Result<()> {
    let journal = seal::read_original_journal(directory)?;
    if let Some(predecessor) = journal.predecessor_cutover {
        storage
            .bind_successor_identity(identity.clone(), predecessor)
            .await?;
    } else {
        storage.bind_runtime_identity(identity.clone()).await?;
    }
    Ok(())
}

async fn settle_original_nonces(
    api: &GearApi,
    scanner: &MessageSender,
    intent: &Intent,
) -> Result<()> {
    if seal::marker_path(&intent.old_transactions).exists() {
        return Ok(());
    }
    let journal = seal::read_original_journal(&intent.old_transactions)?;
    let originals = journal
        .transactions
        .values()
        .filter(|transaction| {
            transaction
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.signed_submission.is_some())
        })
        .cloned()
        .collect::<Vec<_>>();
    if originals.is_empty() {
        return Ok(());
    }
    let storage = Arc::new(JSONStorage::new(&intent.old_transactions));
    bind_original_storage(&storage, &intent.old_identity, &intent.old_transactions).await?;
    let manager = TransactionManager::new(storage.clone());
    storage.load(&manager).await?;
    for transaction in originals {
        let proof = transaction.receipt_event()?;
        let mut signed = transaction
            .receipt
            .as_ref()
            .and_then(|receipt| receipt.signed_submission.clone())
            .context("Missing original")?;
        let result = reconcile_original(
            api,
            scanner,
            &proof,
            &mut signed,
            transaction.saved_settlement_observation.as_ref(),
        )
        .await;
        let (status, outcome) = result.unwrap_or_else(|error| {
            (
                MessageStatus::NeedsReconciliation {
                    receipt_key: signed.receipt_key,
                    diagnostic: format!("HOLD: {error:#}"),
                    receipt_observation: None,
                },
                None,
            )
        });
        let settled = matches!(
            status,
            MessageStatus::Success { .. } | MessageStatus::RetryableNoop { .. }
        );
        manager
            .finalize_transaction(Response {
                tx_uuid: transaction.uuid,
                status,
                submission: outcome.map(|reply| SubmissionEvidence {
                    observed_at_ms: relayer::message_relayer::eth_to_gear::message_sender::now_ms(),
                    manager: intent.old_identity.vft_manager_address,
                    historical_proxy: intent.old_identity.historical_proxy_address,
                    sender: signed.sender.clone(),
                    manager_reply: reply.ok(),
                    error: None,
                }),
                signed_submission: Some(signed),
            })
            .await?;
        ensure!(
            settled,
            "HOLD: drain original account nonce ownership before lifecycle signing"
        );
    }
    Ok(())
}

async fn ensure_call_authority(api: &GearApi, step: &CutoverStep, target: ActorId) -> Result<()> {
    use vft_client::traits::VftAdmin as _;
    let account: [u8; 32] = api.account_id().clone().into();
    let caller = ActorId::from(account);
    let at = api.last_block_hash().await?;
    let pin = sails_rs::H256::from_slice(at.as_ref());
    let authorized = match step {
        CutoverStep::WrapperPause | CutoverStep::WrapperResume => {
            vft_client::VftAdmin::new(GClientRemoting::new(api.clone()))
                .pauser()
                .at_block(pin)
                .recv(target)
                .await?
                == caller
        }
        CutoverStep::TokenMinter(_)
        | CutoverStep::TokenBurner(_)
        | CutoverStep::WrapperSetBurner
        | CutoverStep::WrapperConfigureManager => {
            vft_client::VftAdmin::new(GClientRemoting::new(api.clone()))
                .admin()
                .at_block(pin)
                .recv(target)
                .await?
                == caller
        }
        CutoverStep::SourcePause | CutoverStep::DestinationUnpause => {
            let manager = VftManager::new(GClientRemoting::new(api.clone()));
            manager.admin().at_block(pin).recv(target).await? == caller
                || manager.pause_admin().at_block(pin).recv(target).await? == caller
        }
        CutoverStep::SourceRefund(request, _) => {
            let manager = VftManager::new(GClientRemoting::new(api.clone()));
            if manager.admin().at_block(pin).recv(target).await? == caller {
                true
            } else {
                let mut start = 0u32;
                loop {
                    let rows = manager
                        .request_briding_msg_tracker_state(start, 100)
                        .at_block(pin)
                        .recv(target)
                        .await?;
                    if let Some((_, info)) =
                        rows.iter().find(|(id, _)| id.into_bytes() == request.0)
                    {
                        break info.details.sender == caller;
                    }
                    if rows.len() < 100 {
                        break false;
                    }
                    start = start
                        .checked_add(100)
                        .context("Authority tracker pagination overflow")?;
                }
            }
        }
        _ => {
            VftManager::new(GClientRemoting::new(api.clone()))
                .admin()
                .at_block(pin)
                .recv(target)
                .await?
                == caller
        }
    };
    ensure!(authorized,"HOLD: configured Gear signer is not the current authority for {step:?}; resume with the already-authorized account before signing");
    ensure!(
        api.get_block_hash(api.block_number_at(at).await?).await? == at,
        "HOLD: authority query finalized pin changed"
    );
    Ok(())
}
async fn run_cutover_call(
    api: &GearApi,
    work_path: &Path,
    step: CutoverStep,
    target: ActorId,
    payload: &[u8],
) -> Result<FinalizedCutoverCall> {
    let mut work = read_work(work_path)?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let mut identity = work.intent.old_identity.clone();
    identity.vft_manager_address = hash_actor(target);
    let scanner = sender(
        ApiProviderConnection::from_api(signer.api().clone()),
        &identity,
        Some(work.intent.erc20_manager),
    );
    settle_original_nonces(
        api,
        &sender(
            scanner.api_provider.clone(),
            &work.intent.old_identity,
            Some(work.intent.erc20_manager),
        ),
        &work.intent,
    )
    .await?;
    // No new nonce while any earlier lifecycle original is unresolved.
    for index in 0..work.calls.len() {
        let call = &mut work.calls[index];
        validate_fixed_intent(&work.intent, call)?;
        let result = reconcile_call(api, &scanner_for(&scanner, call.target), call, false).await;
        if no_effect(&call.original) {
            call.archived_no_effect = true;
        }
        let relevant =
            call.step == step && call.target == hash_actor(target) && call.payload == payload;
        let settled = call.archived_no_effect || complete(&call.original) || terminal_upgrade(call);
        let reply_result = if complete(&call.original) && !definite_refund_rejection(call) {
            validate_call_reply(call)
        } else {
            Ok(())
        };
        save(work_path, &work)?;
        result?;
        reply_result?;
        ensure!(
            settled || relevant,
            "HOLD: prior lifecycle original remains unresolved"
        );
    }
    save(work_path, &work)?;
    let existing = work
        .calls
        .iter()
        .rposition(|call| call.step == step && !call.archived_no_effect);
    let index = if let Some(index) = existing {
        ensure!(
            work.calls[index].target == hash_actor(target) && work.calls[index].payload == payload,
            "HOLD: fixed lifecycle intent changed"
        );
        index
    } else {
        ensure_call_authority(api, &step, target).await?;
        let original = scanner
            .prepare_call(
                api,
                (0, 0),
                hash_actor(target),
                payload.to_vec(),
                format!("{:#x}", alloy_primitives::keccak256(payload)),
            )
            .await?;
        let attempt = u32::try_from(work.calls.iter().filter(|call| call.step == step).count())?;
        work.calls.push(FinalizedCutoverCall {
            step,
            attempt,
            target: hash_actor(target),
            payload: payload.to_vec(),
            gas: api.block_gas_limit()? / 100 * 95,
            value: 0,
            original,
            archived_no_effect: false,
        });
        save(work_path, &work)?;
        work.calls.len() - 1
    };
    for _ in 0..4 {
        let call = &mut work.calls[index];
        let result = reconcile_call(api, &scanner, call, true).await;
        save(work_path, &work)?;
        result?;
        let call = &work.calls[index];
        if no_effect(&call.original) {
            bail!("HOLD: original lifecycle call definitely failed before enqueue; resume to archive and retry");
        }
        if call.original.finalized_reply_error.is_some() {
            bail!("HOLD: original lifecycle reply failed; do not infer lifecycle success");
        }
        if complete(&call.original) {
            validate_call_reply(call)?;
            return Ok(call.clone());
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
    bail!("HOLD: original lifecycle outcome pending; resume same cutover work")
}
fn scanner_for(scanner: &MessageSender, target: H256) -> MessageSender {
    MessageSender::new(
        target,
        scanner.receiver_route.clone(),
        scanner.historical_proxy_address,
        scanner.api_provider.clone(),
        String::new(),
        None,
    )
}

fn validate_covered_block(block: &CoveredBlock) -> Result<()> {
    ensure!(
        block.header.number == block.number
            && block.header.hash_slow().0 == block.hash.0
            && block.header.parent_hash.0 == block.parent_hash.0
            && block.header.receipts_root.0 == block.receipts_root.0,
        "HOLD: saved coverage header identity changed"
    );
    let transactions = block
        .raw_transactions
        .iter()
        .map(|raw| {
            let mut bytes = raw.as_slice();
            let transaction = TxEnvelope::decode_2718(&mut bytes)?;
            ensure!(
                bytes.is_empty() && transaction.encoded_2718() == *raw,
                "HOLD: noncanonical coverage transaction"
            );
            Ok(transaction)
        })
        .collect::<Result<Vec<_>>>()?;
    let hashes = block
        .raw_transactions
        .iter()
        .map(|raw| alloy_primitives::keccak256(raw))
        .collect::<Vec<_>>();
    ensure!(
        hashes == block.transactions
            && alloy::consensus::proofs::calculate_transaction_root(&transactions)
                == block.header.transactions_root,
        "HOLD: coverage transaction inventory/trie differs from authenticated header"
    );
    let mut receipts = block.receipts.clone();
    validate_receipt_set(
        block.number,
        block.hash.0.into(),
        &hashes,
        block.receipts_root.0.into(),
        &mut receipts,
    )?;
    Ok(())
}
fn is_deposit(receipt: &TransactionReceipt, manager: H160) -> Result<bool> {
    let mut found = false;
    for log in receipt.inner.logs() {
        if log.address().as_slice() == manager.as_bytes()
            && log.inner.data.topics().first()
                == Some(&IERC20Manager::BridgingRequested::SIGNATURE_HASH)
        {
            IERC20Manager::BridgingRequested::decode_log(&log.inner)
                .context("HOLD: malformed authenticated deposit event")?;
            found = true;
        }
    }
    Ok(found)
}
async fn fee_emitter(
    eth: &PollingEthApi,
    intent: &Intent,
    receipt: &TransactionReceipt,
) -> Result<Option<H160>> {
    let manager = IERC20Manager::new(
        alloy_primitives::Address::from_slice(intent.erc20_manager.as_bytes()),
        &**eth,
    );
    for log in receipt.inner.logs() {
        if log.inner.data.topics().first() != Some(&BridgingPayment::FeePaid::SIGNATURE_HASH) {
            continue;
        }
        let address = H160::from_slice(log.address().as_slice());
        let approved = if let Some(payment) = intent.old_identity.bridging_payment_address {
            address == payment
        } else {
            manager
                .isBridgingPayment(log.address())
                .block(BlockId::hash_canonical(
                    receipt
                        .block_hash
                        .context("HOLD: fee receipt block missing")?,
                ))
                .call()
                .await?
        };
        if approved {
            BridgingPayment::FeePaid::decode_log(&log.inner)
                .context("HOLD: malformed authenticated FeePaid")?;
            let payment = BridgingPayment::new(log.address(), &**eth);
            ensure!(
                payment
                    .erc20Manager()
                    .block(BlockId::hash_canonical(
                        receipt
                            .block_hash
                            .context("HOLD: fee receipt block missing")?
                    ))
                    .call()
                    .await?
                    .as_slice()
                    == intent.erc20_manager.as_bytes(),
                "HOLD: approved fee payer belongs to a different ERC20 manager"
            );
            return Ok(Some(address));
        }
    }
    Ok(None)
}
fn belongs_to_source(intent: &Intent, timestamp: u64) -> Result<bool> {
    if intent.source_receipt_floor == 0 {
        return Ok(true);
    }
    let delta = intent
        .boundary_execution_timestamp
        .checked_sub(timestamp)
        .context("HOLD: coverage timestamp exceeds boundary")?;
    let Some(slot) = intent
        .boundary
        .slot
        .checked_sub(delta / ethereum_common::SECONDS_PER_SLOT)
    else {
        return Ok(false);
    };
    if slot < intent.source_receipt_floor {
        return Ok(false);
    }
    ensure!(
        delta % ethereum_common::SECONDS_PER_SLOT == 0,
        "HOLD: execution timestamp does not map to authenticated Beacon slots"
    );
    Ok(true)
}

fn accepted_external_parent(proof: &[u8], signed: &SignedSubmission) -> bool {
    let mut bytes = proof;
    let Ok(event) = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes) else {
        return false;
    };
    signed.finalized_reply_error.is_none()
        && signed.finalized_reply.as_ref().is_some_and(|reply| {
            match exact_reply::<historical_proxy_client::historical_proxy::io::Redirect>(
                &reply.payload,
            ) {
                Ok(Ok((receipt, reply))) if receipt == event.receipt_rlp => matches!(
                    exact_reply::<io::SubmitReceipt>(&reply),
                    Ok(Ok(())) | Ok(Err(vft_manager_client::Error::NativeSettlementPending))
                ),
                _ => false,
            }
        })
}
async fn discover_external_originals(
    scanner: &MessageSender,
    genesis: H256,
) -> Result<Vec<(Vec<u8>, seal::CanonicalExternalOriginal)>> {
    use historical_proxy_client::historical_proxy::io::Redirect;
    let chain = scanner.api_provider.client();
    ensure!(
        chain.api.genesis_hash().as_ref() == genesis.as_bytes()
            && chain.block_number_to_hash(0).await? == genesis,
        "HOLD: external original discovery genesis changed"
    );
    let head = chain.latest_finalized_block().await?;
    let end = chain.block_hash_to_number(head).await?;
    let mut proofs = Vec::new();
    let mut continuations = Vec::new();
    for number in 1..=end {
        let hash = chain.block_number_to_hash(number).await?;
        let block = chain.api.blocks().at(hash).await?;
        for extrinsic in block.extrinsics().await?.iter() {
            let Some(call) =
                extrinsic.as_extrinsic::<gsdk::gear::gear::calls::types::SendMessage>()?
            else {
                continue;
            };
            let (proof, key, destination, is_continuation) = if call.destination.as_ref()
                == scanner.historical_proxy_address.as_bytes()
            {
                let Some(mut payload) = call.payload.strip_prefix(Redirect::ROUTE) else {
                    continue;
                };
                let (slot, proof, destination, route) =
                    <(u64, Vec<u8>, ActorId, Vec<u8>)>::decode(&mut payload)?;
                if destination != actor(scanner.receiver_address) || route != scanner.receiver_route
                {
                    continue;
                }
                ensure!(
                    payload.is_empty(),
                    "HOLD: archived Redirect has trailing bytes"
                );
                let mut bytes = proof.as_slice();
                let event = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
                ensure!(
                    bytes.is_empty() && slot == event.proof_block.block.slot,
                    "HOLD: archived receipt proof domain/encoding differs"
                );
                (
                    Some(proof),
                    (slot, event.transaction_index),
                    scanner.historical_proxy_address,
                    false,
                )
            } else if call.destination.as_ref() == scanner.receiver_address.as_bytes() {
                let Some(mut payload) = call.payload.strip_prefix(io::ReconcileReceipt::ROUTE)
                else {
                    continue;
                };
                let key = <(u64, u64)>::decode(&mut payload)?;
                ensure!(
                    payload.is_empty(),
                    "HOLD: archived reconciliation has trailing bytes"
                );
                (None, key, scanner.receiver_address, true)
            } else {
                continue;
            };
            ensure!(
                call.value == 0,
                "HOLD: unsupported value-bearing archived receipt call"
            );
            let address = extrinsic
                .address_bytes()
                .context("HOLD: archived receipt call is unsigned")?;
            ensure!(
                address.len() == 33 && address[0] == 0,
                "HOLD: unsupported archived receipt signer encoding"
            );
            let account = subxt::utils::AccountId32(address[1..].try_into()?);
            let nonce = extrinsic
                .transaction_extensions()
                .and_then(|extensions| extensions.nonce())
                .context("HOLD: archived receipt nonce missing")?;
            let previous = chain.block_number_to_hash(number - 1).await?;
            let payload_hash = if let Some(proof) = &proof {
                alloy_primitives::keccak256(proof)
            } else {
                alloy_primitives::keccak256(&call.payload)
            };
            let mut original = SignedSubmission {
                chain_genesis_hash: format!("{genesis:#x}"),
                manager: format!("{:#x}", scanner.receiver_address),
                historical_proxy: format!("{:#x}", scanner.historical_proxy_address),
                sender: account.to_string(),
                receipt_key: key,
                payload_hash: format!("{payload_hash:#x}"),
                nonce,
                extrinsic_hash: format!("{:#x}", extrinsic.hash()),
                raw_extrinsic: extrinsic.bytes().to_vec(),
                prepared_finalized_number: 0,
                prepared_finalized_hash: format!("{genesis:#x}"),
                scanned_finalized_number: number - 1,
                scanned_finalized_hash: format!("{previous:#x}"),
                reply_scanned_finalized_number: number - 1,
                reply_scanned_finalized_hash: format!("{previous:#x}"),
                inclusion_block_number: Some(number),
                inclusion_block_hash: Some(format!("{hash:#x}")),
                message_id: None,
                finalized_dispatch_error: None,
                finalized_reply: None,
                native_reconciliation: None,
                native_reconciliation_attempts: vec![],
                finalized_nonce_consumed: None,
                finalized_reply_error: None,
            };
            ensure!(
                scanner
                    .find_signed_extrinsic(&mut original, destination, &call.payload)
                    .await?,
                "HOLD: discovered original is unavailable at its canonical inclusion"
            );
            if let Some(id) = original
                .message_id
                .as_deref()
                .map(H256::from_str)
                .transpose()?
            {
                let reply = scanner
                    .find_finalized_reply(&mut original, id, destination)
                    .await;
                if original.finalized_reply_error.is_none() {
                    reply?;
                }
            }
            if is_continuation {
                continuations.push(original);
            } else if let Some(proof) = proof {
                if original.message_id.is_some() {
                    proofs.push((proof, original));
                }
            }
        }
        ensure!(
            chain.block_number_to_hash(number).await? == hash,
            "HOLD: external original block changed during discovery"
        );
    }
    let final_head = chain.latest_finalized_block().await?;
    let final_number = chain.block_hash_to_number(final_head).await?;
    ensure!(
        final_number >= end && chain.block_number_to_hash(end).await? == head,
        "HOLD: external original finalized history changed"
    );
    let discovered_at = FinalizedReceiptObservation {
        finalized_block_number: Some(final_number),
        finalized_block_hash: format!("{final_head:#x}"),
    };
    let mut found = Vec::with_capacity(proofs.len());
    for (proof, mut original) in proofs {
        let mut native = continuations
            .iter()
            .filter(|continuation| {
                continuation.sender == original.sender
                    && continuation.receipt_key == original.receipt_key
                    && continuation.nonce > original.nonce
            })
            .cloned()
            .collect::<Vec<_>>();
        native.sort_unstable_by_key(|continuation| continuation.nonce);
        if let Some(current) = native.pop() {
            ensure!(
                native
                    .iter()
                    .all(SignedSubmission::has_terminal_reconciliation),
                "HOLD: an earlier external native continuation remains unresolved"
            );
            original.native_reconciliation_attempts = native;
            original.native_reconciliation = Some(Box::new(current));
        }
        original.validate_native_reconciliation()?;
        found.push((
            proof,
            seal::CanonicalExternalOriginal {
                provenance: "canonical-external".into(),
                original,
                discovered_at: discovered_at.clone(),
            },
        ));
    }
    found.sort_by_key(|(proof, external)| !accepted_external_parent(proof, &external.original));
    Ok(found)
}
async fn qualify_external_original(
    api: &GearApi,
    scanner: &MessageSender,
    proof: &eth_events_electra_client::EthToVaraEvent,
    external: &mut seal::CanonicalExternalOriginal,
    saved: Option<&FinalizedReceiptObservation>,
) -> Result<FinalizedReceiptObservation> {
    ensure!(
        external.provenance == "canonical-external",
        "HOLD: unsupported external original provenance"
    );
    let chain = scanner.api_provider.client();
    let discovered = external
        .discovered_at
        .finalized_block_number
        .context("HOLD: external discovery pin lacks height")?;
    let latest_hash = chain.latest_finalized_block().await?;
    let latest = chain.block_hash_to_number(latest_hash).await?;
    ensure!(
        discovered <= latest
            && format!("{:#x}", chain.block_number_to_hash(discovered).await?)
                == external.discovered_at.finalized_block_hash,
        "HOLD: external original discovery pin changed"
    );
    if let Some(saved) = saved {
        scanner
            .verify_saved_completion(api, proof, &external.original, saved)
            .await?;
        return Ok(saved.clone());
    }
    let payload = historical_proxy_client::historical_proxy::io::Redirect::encode_call(
        proof.proof_block.block.slot,
        proof.encode(),
        actor(scanner.receiver_address),
        scanner.receiver_route.clone(),
    );
    ensure!(
        scanner
            .find_signed_extrinsic(
                &mut external.original,
                scanner.historical_proxy_address,
                &payload
            )
            .await?,
        "HOLD: external receipt original is unavailable"
    );
    let id = external
        .original
        .message_id
        .as_deref()
        .map(H256::from_str)
        .transpose()?
        .context("HOLD: external receipt original was not enqueued")?;
    scanner
        .find_finalized_reply(&mut external.original, id, scanner.historical_proxy_address)
        .await?;
    ensure!(
        accepted_external_parent(&proof.encode(), &external.original),
        "HOLD: external receipt original lacks an admitted successful/pending-native proof reply"
    );
    let mut first = external
        .original
        .finalized_reply
        .as_ref()
        .and_then(|reply| reply.observation.finalized_block_number)
        .context("HOLD: external original reply pin missing")?;
    if let Some(native) = external.original.native_reconciliation.as_mut() {
        let call = io::ReconcileReceipt::encode_call(
            external.original.receipt_key.0,
            external.original.receipt_key.1,
        );
        ensure!(
            scanner
                .find_signed_extrinsic(native, scanner.receiver_address, &call)
                .await?,
            "HOLD: external native continuation unavailable"
        );
        let id = native
            .message_id
            .as_deref()
            .map(H256::from_str)
            .transpose()?
            .context("HOLD: external native continuation not enqueued")?;
        scanner
            .find_finalized_reply(native, id, scanner.receiver_address)
            .await?;
        first = first.max(
            native
                .finalized_reply
                .as_ref()
                .and_then(|reply| reply.observation.finalized_block_number)
                .context("HOLD: external native continuation reply pending")?,
        );
    }
    for number in first..=latest {
        let pin = FinalizedReceiptObservation {
            finalized_block_number: Some(number),
            finalized_block_hash: format!("{:#x}", chain.block_number_to_hash(number).await?),
        };
        if matches!(
            scanner.settlement_status(api, proof, Some(&pin)).await?.0,
            MessageStatus::Success { .. }
        ) {
            scanner
                .verify_saved_completion(api, proof, &external.original, &pin)
                .await?;
            ensure!(
                chain.block_number_to_hash(latest).await? == latest_hash,
                "HOLD: external discovery anchor changed during settlement qualification"
            );
            external.discovered_at = FinalizedReceiptObservation {
                finalized_block_number: Some(latest),
                finalized_block_hash: format!("{latest_hash:#x}"),
            };
            return Ok(pin);
        }
    }
    bail!("HOLD: external original has no authenticated complete historical economic settlement; resume its original manual owner during Draining")
}
async fn compose_admission(
    beacon: &BeaconClient,
    api: &GearApi,
    eth: &PollingEthApi,
    hash: B256,
    identity: &InboundRuntimeIdentity,
) -> Result<eth_events_electra_client::EthToVaraEvent> {
    let proof = compose(
        beacon,
        api,
        eth,
        hash,
        actor(identity.historical_proxy_address),
    )
    .await?;
    let selected = authenticate_cutover_boundary(
        beacon,
        api,
        actor(identity.checkpoint_light_client_address),
        proof.proof_block.block.slot,
    )
    .await?;
    let authenticated = eth_events_electra_client::BlockGenericForBlockBody {
        slot: selected.slot,
        proposer_index: selected.proposer_index,
        parent_root: selected.parent_root,
        state_root: selected.state_root,
        body: selected.body.into(),
    };
    ensure!(
        authenticated.encode() == proof.proof_block.block.encode(),
        "HOLD: proof selected a block outside the independently approved checkpoint program"
    );
    Ok(proof)
}

async fn scan_coverage(
    api: &GearApi,
    eth: &PollingEthApi,
    beacon: &BeaconClient,
    path: &Path,
    sponsor: bool,
) -> Result<()> {
    let mut work = read_work(path)?;
    let originals = seal::read_original_journal(&work.intent.old_transactions)?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let readonly_sender = sender(
        ApiProviderConnection::from_api(signer.api().clone()),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    let mut external_cache: Option<Vec<(Vec<u8>, seal::CanonicalExternalOriginal)>> = None;
    // The journal floor lacks an independent constructor attestation here: scan from genesis.
    let floor = 0;
    let mut next = work
        .blocks
        .last()
        .map_or(floor, |block| block.number.saturating_add(1));
    for covered in &work.blocks {
        ensure!(
            eth.is_finalized_block(covered.number, covered.hash).await?,
            "HOLD: saved canonical coverage changed"
        );
        validate_covered_block(covered)?;
    }
    while next <= work.intent.boundary.execution_number {
        let canonical = eth.get_block(next).await?;
        let block = eth
            .get_block_by_hash(canonical.header.hash)
            .full()
            .await?
            .context("HOLD: complete canonical transaction inventory unavailable")?;
        ensure!(
            block.header.number == next
                && block.header.inner.hash_slow() == block.header.hash
                && eth
                    .is_finalized_block(next, H256(block.header.hash.0))
                    .await?,
            "HOLD: coverage header is not canonical finalized"
        );
        if let Some(previous) = work.blocks.last() {
            ensure!(
                block.header.parent_hash.0 == previous.hash.0,
                "HOLD: coverage ancestry gap"
            );
        }
        let raw_transactions = block
            .transactions
            .try_into_transactions()
            .map_err(|_| anyhow!("HOLD: full coverage transactions missing"))?
            .into_iter()
            .map(|transaction| transaction.into_inner().encoded_2718())
            .collect::<Vec<_>>();
        let hashes = raw_transactions
            .iter()
            .map(|raw| alloy_primitives::keccak256(raw))
            .collect::<Vec<_>>();
        let decoded = raw_transactions
            .iter()
            .map(|raw| TxEnvelope::decode_2718(&mut raw.as_slice()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(
            alloy::consensus::proofs::calculate_transaction_root(&decoded)
                == block.header.transactions_root,
            "HOLD: canonical transaction trie differs"
        );
        let mut receipts = if hashes.is_empty() {
            Vec::new()
        } else {
            eth.get_block_receipts(BlockId::hash_canonical(block.header.hash))
                .await?
                .context("HOLD: complete receipts unavailable")?
        };
        validate_receipt_set(
            next,
            block.header.hash,
            &hashes,
            block.header.receipts_root,
            &mut receipts,
        )?;
        for receipt in &receipts {
            if !receipt.status() {
                continue;
            }
            if !belongs_to_source(&work.intent, block.header.timestamp)?
                || !is_deposit(receipt, work.intent.erc20_manager)?
            {
                continue;
            }
            let emitter = fee_emitter(eth, &work.intent, receipt).await?;
            let paid = emitter.is_some();
            if let Some(existing) = work
                .admissions
                .iter()
                .find(|admission| admission.original_tx_hash == receipt.transaction_hash)
            {
                ensure!(
                    existing.original_block_hash.0 == block.header.hash.0
                        && existing.fee_event_observed == paid,
                    "HOLD: original receipt block/fee changed"
                );
                continue;
            }
            let original = originals
                .completed
                .values()
                .chain(originals.transactions.values())
                .find(|transaction| transaction.tx.tx_hash == receipt.transaction_hash);
            let mut external_original = None;
            let proof = if original.is_none_or(never_dispatched) {
                if external_cache.is_none() {
                    external_cache = Some(
                        discover_external_originals(
                            &readonly_sender,
                            work.intent.old_identity.gear_genesis_hash,
                        )
                        .await?,
                    );
                }
                let expected = alloy_rlp::encode(ethereum_common::utils::map_receipt_envelope(
                    receipt.as_ref(),
                ));
                let candidate = external_cache
                    .as_ref()
                    .context("External discovery cache missing")?
                    .iter()
                    .find(|(proof, _)| {
                        let Ok(event) = eth_events_electra_client::EthToVaraEvent::decode(
                            &mut proof.as_slice(),
                        ) else {
                            return false;
                        };
                        event.receipt_rlp == expected
                            && Some(event.transaction_index) == receipt.transaction_index
                            && event
                                .proof_block
                                .block
                                .body
                                .execution_payload
                                .block_hash
                                .0
                                 .0
                                == block.header.hash.0
                    });
                if let Some((proof, external)) = candidate {
                    external_original = Some(external.clone());
                    eth_events_electra_client::EthToVaraEvent::decode(&mut proof.as_slice())?
                } else if let Some(transaction) =
                    original.filter(|transaction| transaction.receipt.is_some())
                {
                    transaction.receipt_event()?
                } else {
                    compose_admission(
                        beacon,
                        api,
                        eth,
                        receipt.transaction_hash,
                        &work.intent.old_identity,
                    )
                    .await?
                }
            } else {
                original
                    .context("Original receipt identity missing")?
                    .receipt_event()?
            };
            let key = (proof.proof_block.block.slot, proof.transaction_index);
            if key.0 < work.intent.source_receipt_floor {
                continue;
            }
            ensure!(
                key.0 <= work.intent.boundary.slot,
                "HOLD: deposit exceeds source receipt boundary"
            );
            if let Some(existing) = work
                .admissions
                .iter()
                .find(|admission| admission.receipt_key == key)
            {
                ensure!(
                    existing.original_tx_hash == receipt.transaction_hash
                        && existing.original_block_hash.0 == block.header.hash.0
                        && existing.proof == proof.encode(),
                    "HOLD: original receipt admission changed"
                );
                continue;
            }
            work.admissions.push(Admission {
                receipt_key: key,
                original_tx_hash: receipt.transaction_hash,
                original_block_hash: H256(block.header.hash.0),
                admission: if external_original.is_some() {
                    "external-original-pending"
                } else if paid {
                    "paid"
                } else {
                    "cutover-sponsored"
                }
                .into(),
                fee_event_observed: paid,
                fee_emitter: emitter,
                proof: proof.encode(),
                signed: None,
                attempts: vec![],
                settlement: None,
                original_lane_uuid: None,
                undispatched_old_admission: if external_original.is_some() {
                    original
                        .filter(|transaction| never_dispatched(transaction))
                        .map(|transaction| transaction.uuid)
                } else {
                    None
                },
                external_original,
            });
            // Admission is committed before the coverage cursor can move.
            save(path, &work)?;
        }
        work.blocks.push(CoveredBlock {
            number: next,
            hash: H256(block.header.hash.0),
            parent_hash: H256(block.header.parent_hash.0),
            receipts_root: H256(block.header.receipts_root.0),
            transactions: hashes,
            receipts,
            header: block.header.inner,
            raw_transactions,
        });
        save(path, &work)?;
        next = next
            .checked_add(1)
            .context("HOLD: coverage cursor overflow")?;
    }
    ensure!(
        work.blocks.last().is_some_and(|block| block.number
            == work.intent.boundary.execution_number
            && block.hash == work.intent.boundary.execution_hash),
        "HOLD: incomplete boundary coverage"
    );
    let unpaid = work
        .admissions
        .iter()
        .filter(|admission| admission.admission == "cutover-sponsored")
        .count();
    ensure!(unpaid == 0 || sponsor, "HOLD: {unpaid} authentic unpaid pre-boundary receipts require --sponsor-unpaid; admissions retained in cutover-work.json");
    Ok(())
}

async fn drain(
    api: &GearApi,
    _connection: &GearConnectionArgs,
    eth: &PollingEthApi,
    beacon: &BeaconClient,
    path: &Path,
    sponsor: bool,
) -> Result<()> {
    let mut work = read_work(path)?;
    ensure!(
        sponsor
            || work
                .admissions
                .iter()
                .all(|admission| admission.fee_event_observed
                    || admission.external_original.is_some()),
        "HOLD: unpaid sponsorship requires explicit invocation authorization"
    );
    let signer: gsdk::signer::Signer = api.clone().into();
    let provider = ApiProviderConnection::from_api(signer.api().clone());
    let sender = sender(
        provider.clone(),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    let storage = Arc::new(JSONStorage::new(&work.intent.old_transactions));
    bind_original_storage(
        &storage,
        &work.intent.old_identity,
        &work.intent.old_transactions,
    )
    .await?;
    let manager = TransactionManager::new(storage.clone());
    storage.load(&manager).await?;
    let sender_id: [u8; 32] = api.account_id().clone().into();
    let originals: Vec<Transaction> = manager
        .transactions
        .read()
        .await
        .values()
        .cloned()
        .collect();
    for transaction in originals {
        if let Some(admission) = work
            .admissions
            .iter()
            .find(|admission| admission.undispatched_old_admission == Some(transaction.uuid))
        {
            ensure!(
                never_dispatched(&transaction)
                    && admission.original_tx_hash == transaction.tx.tx_hash
                    && admission.external_original.is_some(),
                "HOLD: external fulfillment cannot replace an original signature/handoff"
            );
            continue;
        }
        ensure!(sender_id == work.intent.old_identity.gear_sender || transaction.receipt.as_ref().is_some_and(|receipt|receipt.signed_submission.is_some()), "HOLD: distinct admin cannot sign unsigned original-lane admissions; resume original lane during Draining");
        let id = transaction.uuid;
        if transaction.receipt.is_none() {
            let proof = compose_admission(
                beacon,
                api,
                eth,
                transaction.tx.tx_hash,
                &work.intent.old_identity,
            )
            .await?;
            manager.submit_message(id, proof).await?;
        }
        let transaction = manager
            .transactions
            .read()
            .await
            .get(&id)
            .cloned()
            .context("Original admission vanished")?;
        let proof = transaction.receipt_event()?;
        let mut signed = match transaction
            .receipt
            .as_ref()
            .and_then(|receipt| receipt.signed_submission.clone())
        {
            Some(signed) => signed,
            None => {
                ensure!(
                    sender_id == work.intent.old_identity.gear_sender,
                    "HOLD: only original lane signer may create its never-signed proof"
                );
                let signed = sender.prepare_submission(api, &proof).await?;
                manager
                    .finalize_transaction(Response {
                        tx_uuid: id,
                        status: MessageStatus::Prepared,
                        submission: None,
                        signed_submission: Some(signed.clone()),
                    })
                    .await?;
                signed
            }
        };
        let result = reconcile_original(
            api,
            &sender,
            &proof,
            &mut signed,
            transaction.saved_settlement_observation.as_ref(),
        )
        .await;
        let (status, outcome) = match result {
            Ok(result) => result,
            Err(error) => (
                MessageStatus::NeedsReconciliation {
                    receipt_key: signed.receipt_key,
                    diagnostic: format!("HOLD: {error:#}"),
                    receipt_observation: None,
                },
                None,
            ),
        };
        let done = matches!(status, MessageStatus::Success { .. });
        manager
            .finalize_transaction(Response {
                tx_uuid: id,
                status,
                submission: outcome.map(|outcome| SubmissionEvidence {
                    observed_at_ms: relayer::message_relayer::eth_to_gear::message_sender::now_ms(),
                    manager: work.intent.old_identity.vft_manager_address,
                    historical_proxy: work.intent.old_identity.historical_proxy_address,
                    sender: signed.sender.clone(),
                    manager_reply: outcome.ok(),
                    error: None,
                }),
                signed_submission: Some(signed),
            })
            .await?;
        ensure!(
            done,
            "HOLD: original lane admission {id} remains unresolved; resume original bytes"
        );
    }
    ensure!(
        manager
            .failed
            .read()
            .await
            .keys()
            .all(|uuid| work.admissions.iter().any(|admission| admission
                .undispatched_old_admission
                == Some(*uuid)
                && admission.external_original.is_some())),
        "HOLD: original lane contains failed admissions"
    );
    let mut external_cache: Option<Vec<(Vec<u8>, seal::CanonicalExternalOriginal)>> = None;
    for index in 0..work.admissions.len() {
        let admission = &mut work.admissions[index];
        let mut bytes = admission.proof.as_slice();
        let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
        for archived in &admission.attempts {
            verify_proof_attempt(&sender, &proof, archived).await?;
        }
        ensure!(bytes.is_empty(), "HOLD: admission proof has trailing bytes");
        if let Some(external) = admission.external_original.as_mut() {
            ensure!(
                admission.signed.is_none()
                    && admission.attempts.is_empty()
                    && admission.original_lane_uuid.is_none(),
                "HOLD: external provenance cannot replace a retained own original"
            );
            if admission.settlement.is_none() {
                if external_cache.is_none() {
                    external_cache = Some(
                        discover_external_originals(
                            &sender,
                            work.intent.old_identity.gear_genesis_hash,
                        )
                        .await?,
                    );
                }
                let (_, fresh) = external_cache
                    .as_ref()
                    .context("External cache missing")?
                    .iter()
                    .find(|(bytes, candidate)| {
                        *bytes == admission.proof
                            && candidate.original.extrinsic_hash == external.original.extrinsic_hash
                    })
                    .context(
                        "HOLD: recorded external original unavailable in canonical discovery",
                    )?;
                ensure!(
                    fresh.original.same_request(&external.original)
                        && fresh.original.preserves_reconciliation(&external.original)
                        && fresh.original.inclusion_block_number
                            == external.original.inclusion_block_number
                        && fresh.original.inclusion_block_hash
                            == external.original.inclusion_block_hash
                        && fresh.original.message_id == external.original.message_id
                        && external
                            .original
                            .finalized_reply
                            .as_ref()
                            .is_none_or(
                                |reply| fresh.original.finalized_reply.as_ref() == Some(reply)
                            )
                        && external.original.finalized_reply_error.as_ref().is_none_or(
                            |code| fresh.original.finalized_reply_error.as_ref() == Some(code)
                        ),
                    "HOLD: canonical external original identity/history changed"
                );
                external.original = fresh.original.clone();
            }
            let result = qualify_external_original(
                api,
                &sender,
                &proof,
                external,
                admission.settlement.as_ref(),
            )
            .await;
            if let Ok(pin) = &result {
                admission.settlement = Some(pin.clone());
                admission.admission = "externally-delivered".into();
            }
            save(path, &work)?;
            result?;
            continue;
        }
        let original = manager
            .completed
            .read()
            .await
            .values()
            .find(|transaction| transaction.tx.tx_hash == admission.original_tx_hash)
            .cloned();
        if let Some(transaction) = original {
            let original_proof = transaction.receipt_event()?;
            ensure!(
                original_proof.receipt_rlp == proof.receipt_rlp
                    && original_proof.transaction_index == proof.transaction_index
                    && original_proof.proof_block.block.slot == proof.proof_block.block.slot,
                "HOLD: original paid lane receipt/key changed"
            );
            let original_signed = transaction
                .receipt
                .as_ref()
                .and_then(|receipt| receipt.signed_submission.as_ref())
                .context("HOLD: original paid signed proof missing")?;
            let original_pin = transaction
                .saved_settlement_observation
                .as_ref()
                .or(transaction.receipt_observation.as_ref())
                .context("HOLD: historical paid settlement pin missing")?;
            sender
                .verify_saved_completion(api, &original_proof, original_signed, original_pin)
                .await?;
            let (status, _) = sender
                .settlement_status(api, &original_proof, Some(original_pin))
                .await?;
            ensure!(
                matches!(status, MessageStatus::Success { .. }),
                "HOLD: original historical settlement no longer authenticates"
            );
            admission.original_lane_uuid = Some(transaction.uuid);
            admission.settlement = transaction
                .saved_settlement_observation
                .clone()
                .or(transaction.receipt_observation.clone());
            save(path, &work)?;
            continue;
        }
        let signed = match admission.signed.take() {
            Some(signed) => signed,
            None => sender.prepare_submission(api, &proof).await?,
        };
        admission.signed = Some(signed);
        save(path, &work)?;
        let admission = &mut work.admissions[index];
        let mut signed = admission
            .signed
            .take()
            .context("Missing original cutover proof")?;
        let result = reconcile_original(
            api,
            &sender,
            &proof,
            &mut signed,
            admission.settlement.as_ref(),
        )
        .await;
        admission.signed = Some(signed);
        let status = result.map(|(status, _)| status);
        if let Ok(MessageStatus::Success {
            receipt_observation,
        }) = &status
        {
            admission.settlement = receipt_observation.clone();
        }
        if matches!(&status, Ok(MessageStatus::RetryableNoop { .. })) {
            let mut archived = admission
                .signed
                .take()
                .context("Missing original no-effect proof")?;
            if archived.inclusion_block_number.is_none() {
                let hash = H256::from_str(&archived.scanned_finalized_hash)?;
                archived.finalized_nonce_consumed = Some(u64::from(
                    sender
                        .api_provider
                        .client()
                        .api
                        .info_at(&archived.sender, Some(hash))
                        .await?
                        .nonce,
                ));
            }
            admission.attempts.push(archived);
        }
        save(path, &work)?;
        ensure!(
            matches!(status?, MessageStatus::Success { .. }),
            "HOLD: cutover admission remains unresolved; resume same original"
        );
    }
    storage.save(&manager).await?;
    storage.save_blocks().await?;
    ensure!(
        manager
            .transactions
            .read()
            .await
            .values()
            .all(|transaction| never_dispatched(transaction)
                && work
                    .admissions
                    .iter()
                    .any(|admission| admission.undispatched_old_admission
                        == Some(transaction.uuid)
                        && admission.external_original.is_some()
                        && admission.settlement.is_some())),
        "HOLD: original lane remains undrained"
    );
    Ok(())
}

pub async fn prepare(
    connection: &GearConnectionArgs,
    api: &GearApi,
    args: &PrepareCutover,
) -> Result<()> {
    let _work_owner = lock_work(&args.cutover_dir)?;
    let path = work_path(&args.cutover_dir);
    let old_transactions = fs::canonicalize(&args.old_transactions)?;
    let old_discovery = fs::canonicalize(&args.old_discovery)?;
    let _journals = seal::lock_original_journals(&old_transactions, &old_discovery)?;
    let journal = seal::read_original_journal(&old_transactions)?;
    let old_identity = journal
        .runtime_identity
        .context("HOLD: original journal runtime not bound")?;
    ensure!(
        old_identity.vft_manager_address == hash_actor(args.old_vft_manager)
            && args.old_vft_manager != args.new_vft_manager
            && !args.new_vft_manager.is_zero(),
        "HOLD: source/destination identity differs"
    );
    ensure!(
        journal
            .transactions
            .values()
            .chain(journal.completed.values())
            .all(
                |transaction| transaction.tx.slot_number.0 <= args.boundary_slot
                    && transaction
                        .receipt
                        .as_ref()
                        .is_none_or(|receipt| receipt.receipt_key.0 <= args.boundary_slot)
            ),
        "HOLD: original journal contains receipts above proposed boundary"
    );
    let eth = PollingEthApi::new(&args.ethereum_rpc).await?;
    let beacon = BeaconClient::new(args.beacon_rpc.clone(), Some(Duration::from_secs(30))).await?;
    let manager = VftManager::new(GClientRemoting::new(api.clone()));
    let at = api.last_block_hash().await?;
    let pin = sails_rs::H256::from_slice(at.as_ref());
    let block = authenticate_cutover_boundary(
        &beacon,
        api,
        actor(old_identity.checkpoint_light_client_address),
        args.boundary_slot,
    )
    .await?;
    let source_status = manager
        .cutover_status()
        .at_block(pin)
        .recv(args.old_vft_manager)
        .await?;
    ensure!(
        manager
            .is_paused()
            .at_block(pin)
            .recv(args.new_vft_manager)
            .await?,
        "HOLD: replacement must remain manually paused before cutover preparation"
    );
    let mut mappings = manager
        .vara_to_eth_addresses()
        .at_block(pin)
        .recv(args.old_vft_manager)
        .await?;
    mappings.sort_unstable_by_key(|(token, _, _)| token.into_bytes());
    ensure!(
        mappings.windows(2).all(|pair| pair[0].0 != pair[1].0)
            && mappings
                .iter()
                .map(|(_, eth, _)| *eth)
                .collect::<BTreeSet<_>>()
                .len()
                == mappings.len(),
        "HOLD: duplicate token identities"
    );
    let erc20_manager = manager
        .erc_20_manager_address()
        .at_block(pin)
        .recv(args.old_vft_manager)
        .await?
        .context("HOLD: source ERC20 manager missing")?;
    let native_wrapper = manager
        .native_wrapper()
        .at_block(pin)
        .recv(args.old_vft_manager)
        .await?;
    let config = manager
        .get_config()
        .at_block(pin)
        .recv(args.old_vft_manager)
        .await?;
    let builtin = manager
        .gear_bridge_builtin()
        .at_block(pin)
        .recv(args.old_vft_manager)
        .await?;
    let proxy = manager
        .historical_proxy_address()
        .at_block(pin)
        .recv(args.old_vft_manager)
        .await?;
    ensure!(
        hash_actor(proxy) == old_identity.historical_proxy_address,
        "HOLD: original proxy differs from source"
    );
    let fingerprint = seal::digest(
        &(
            *b"vft-manager/cutover-config/v1",
            builtin,
            proxy,
            Some(erc20_manager),
            native_wrapper,
            config,
            mappings.clone(),
        )
            .encode(),
    );
    let mut new_identity = old_identity.clone();
    new_identity.vft_manager_address = hash_actor(args.new_vft_manager);
    new_identity.ethereum_start_block = block
        .body
        .execution_payload
        .block_number
        .checked_add(1)
        .context("Boundary overflow")?;
    let mut initial_role_pins = Vec::with_capacity(mappings.len());
    for (token, _, supply) in &mappings {
        let previous = if path.exists() {
            read_work(&path)?
                .intent
                .initial_role_pins
                .into_iter()
                .find(|roles| roles.token == hash_actor(*token))
        } else {
            None
        };
        let role_hash = previous
            .as_ref()
            .map_or(H256::from_slice(at.as_ref()), |roles| roles.block_hash);
        let roles = read_role_pin(
            api,
            hash_actor(*token),
            native_wrapper == Some(*token),
            Some(role_hash),
        )
        .await?;
        if let Some(previous) = previous {
            ensure!(
                roles == previous
                    && H256::from_slice(api.get_block_hash(previous.block_number).await?.as_ref())
                        == previous.block_hash,
                "HOLD: original authority pin changed"
            );
        }
        if *supply == TokenSupply::Ethereum {
            ensure!(
                roles.minter == hash_actor(args.old_vft_manager)
                    && roles.burner == hash_actor(args.old_vft_manager),
                "HOLD: Ethereum-supply token roles do not point to source"
            );
        }
        if native_wrapper == Some(*token) {
            ensure!(
                roles.manager == Some(hash_actor(args.old_vft_manager))
                    && roles.burner == hash_actor(args.old_vft_manager),
                "HOLD: source native manager/burner identities differ"
            );
        }
        initial_role_pins.push(roles);
    }
    if let Some(wrapper) = native_wrapper
        .map(hash_actor)
        .filter(|wrapper| !initial_role_pins.iter().any(|pin| pin.token == *wrapper))
    {
        let previous = if path.exists() {
            read_work(&path)?
                .intent
                .initial_role_pins
                .into_iter()
                .find(|pin| pin.token == wrapper)
        } else {
            None
        };
        let role_hash = previous
            .as_ref()
            .map_or(H256::from_slice(at.as_ref()), |pin| pin.block_hash);
        let roles = read_role_pin(api, wrapper, true, Some(role_hash)).await?;
        if let Some(previous) = previous {
            ensure!(
                roles == previous
                    && api.get_block_hash(previous.block_number).await?.as_ref()
                        == previous.block_hash.as_bytes(),
                "HOLD: native authority pin changed"
            );
        }
        ensure!(
            roles.manager == Some(hash_actor(args.old_vft_manager))
                && roles.burner == hash_actor(args.old_vft_manager),
            "HOLD: native manager/burner identities differ"
        );
        initial_role_pins.push(roles);
    }
    let intent = Intent {
        old_identity,
        new_identity,
        new_manager_code_id: H256(args.new_manager_code_id.into_bytes()),
        old_transactions,
        old_discovery,
        source_receipt_floor: source_status.minimum_receipt_slot,
        boundary: CutoverBoundary {
            slot: args.boundary_slot,
            beacon_root: H256::from_slice(block.tree_hash_root().as_bytes()),
            execution_number: block.body.execution_payload.block_number,
            execution_hash: H256(block.body.execution_payload.block_hash.0 .0),
        },
        boundary_execution_timestamp: block.body.execution_payload.timestamp,
        config_fingerprint: fingerprint,
        erc20_manager: H160(erc20_manager.0),
        native_wrapper: native_wrapper.map(hash_actor),
        mappings: mappings
            .iter()
            .map(|(token, eth, supply)| {
                (
                    hash_actor(*token),
                    H160(eth.0),
                    match supply {
                        TokenSupply::Ethereum => 0,
                        TokenSupply::Gear => 1,
                    },
                )
            })
            .collect(),
        initial_role_pins,
    };
    if path.exists() {
        ensure!(
            read_work(&path)?.intent == intent,
            "HOLD: cutover immutable intent changed"
        );
    } else {
        save(
            &path,
            &Work {
                schema_version: 1,
                intent: intent.clone(),
                blocks: vec![],
                admissions: vec![],
                calls: vec![],
                role_pins: vec![],
                registration: None,
                retirement: None,
                readiness: None,
                new_transactions: None,
                new_discovery: None,
            },
        )?;
    }
    let resumed = read_work(&path)?;
    let _signer_owner = lock_signer(&path, api, intent.old_identity.gear_genesis_hash)?;
    let _other_signers = lock_other_signers(api, &resumed.intent, &resumed, &path)?;
    let result: Result<()> = async {
        authenticate_environment(api, &eth, &intent).await?;
        run_cutover_call(
            api,
            &path,
            CutoverStep::BeginCutover,
            args.old_vft_manager,
            &io::BeginCutover::encode_call(args.new_vft_manager, args.boundary_slot),
        )
        .await?;
        let (source, _) = status(api, intent.old_identity.vft_manager_address).await?;
        ensure!(
            source.phase == CutoverPhase::Draining
                && source.successor == Some(args.new_vft_manager)
                && source.last_receipt_slot == Some(args.boundary_slot)
                && source.config_fingerprint == Some(fingerprint.0.into()),
            "HOLD: source frozen intent differs"
        );
        scan_coverage(api, &eth, &beacon, &path, args.sponsor_unpaid).await?;
        drain(api, connection, &eth, &beacon, &path, args.sponsor_unpaid).await?;
        drain_discovery(&eth, &path).await?;
        drain_source(api, &path).await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let release = release_draining_hold(api, &path, &_signer_owner, &_other_signers).await;
        return match release {
            Ok(_) => Err(error),
            Err(release) => Err(error.context(format!("Signer ownership retained: {release:#}"))),
        };
    }
    release_owned_signers(api, &path, &_signer_owner, &_other_signers)?;
    println!("Prepared cutover coverage and original admissions; source remains Draining. Run finalize-cutover with authorized EVM registration evidence.");
    Ok(())
}

async fn drain_discovery(eth: &PollingEthApi, path: &Path) -> Result<()> {
    use relayer::message_relayer::common::ethereum::block_storage::{
        BlockCursor, JSONBlockStorage,
    };
    let work = read_work(path)?;
    let predecessor =
        seal::read_original_journal(&work.intent.old_transactions)?.predecessor_cutover;
    let storage = JSONBlockStorage::new_bound(
        work.intent.old_discovery.clone(),
        work.intent.old_identity.clone(),
        predecessor,
    )
    .await?;
    let snapshot = storage.snapshot().await;
    for (number, hash) in snapshot.pending {
        ensure!(
            number <= work.intent.boundary.execution_number,
            "HOLD: original discovery crossed the cutover boundary"
        );
        ensure!(
            eth.is_finalized_block(number, hash).await?
                && work
                    .blocks
                    .iter()
                    .any(|block| block.number == number && block.hash == hash),
            "HOLD: pending handoff lacks canonical complete coverage"
        );
        // All receipts were admitted durably before acknowledging this handoff.
        storage
            .acknowledge_block(BlockCursor { number, hash })
            .await?;
    }
    Ok(())
}
async fn drain_source(api: &GearApi, path: &Path) -> Result<()> {
    let work = read_work(path)?;
    let manager = VftManager::new(GClientRemoting::new(api.clone()));
    let source = actor(work.intent.old_identity.vft_manager_address);
    let mut start = 0u32;
    loop {
        let at = api.last_block_hash().await?;
        let rows = manager
            .request_briding_msg_tracker_state(start, 100)
            .at_block(sails_rs::H256::from_slice(at.as_ref()))
            .recv(source)
            .await?;
        for (request, info) in &rows {
            use vft_manager_client::MessageStatus as SourceStatus;
            match info.status {
                SourceStatus::TokenDepositCompleted(false) | SourceStatus::BridgeResponseReceived(Some(_)) | SourceStatus::TokensReturnComplete(true) => {},
                SourceStatus::TokenDepositCompleted(true) | SourceStatus::TokensReturnComplete(false) | SourceStatus::BridgeResponseReceived(None) => {
                    let key=H256(request.into_bytes());
                    let ordinal=u32::try_from(read_work(path)?.calls.iter().filter(|call|matches!(call.step,CutoverStep::SourceRefund(previous,_) if previous==key) && definite_refund_rejection(call)).count())?;
                    run_cutover_call(api,path,CutoverStep::SourceRefund(key,ordinal),source,&io::HandleRequestBridgingInterruptedTransfer::encode_call(*request)).await?;
                }
                _ => bail!("HOLD: original outgoing request {request:?} or deposit/refund child remains unresolved"),
            }
        }
        if rows.len() < 100 {
            break;
        }
        start = start
            .checked_add(100)
            .context("Tracker pagination overflow")?;
    }
    if let Some(wrapper) = work.intent.native_wrapper {
        use vft_vara_client::traits::NativeEscrow as _;
        let escrow = vft_vara_client::NativeEscrow::new(GClientRemoting::new(api.clone()));
        let at = api.last_block_hash().await?;
        ensure!(
            escrow
                .outstanding_redemptions()
                .at_block(sails_rs::H256::from_slice(at.as_ref()))
                .recv(actor(wrapper))
                .await?
                == 0,
            "HOLD: original native payouts remain undelivered"
        );
    }
    Ok(())
}
fn validate_registration(intent: &Intent, registration: &Registration) -> Result<()> {
    ensure!(
        registration.block.header.number == registration.included_number
            && registration.block.header.hash == registration.included_hash
            && registration.block.header.inner.hash_slow() == registration.included_hash
            && registration.finalized_number >= registration.included_number,
        "HOLD: original registration inclusion changed"
    );
    let transactions = registration
        .raw_transactions
        .iter()
        .map(|raw| {
            let mut bytes = raw.as_slice();
            let transaction = TxEnvelope::decode_2718(&mut bytes)?;
            ensure!(
                bytes.is_empty() && transaction.encoded_2718() == *raw,
                "HOLD: registration signed transaction bytes changed"
            );
            Ok(transaction)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        alloy::consensus::proofs::calculate_transaction_root(&transactions)
            == registration.block.header.transactions_root,
        "HOLD: registration transaction trie differs"
    );
    let hashes = registration
        .raw_transactions
        .iter()
        .map(|raw| alloy_primitives::keccak256(raw))
        .collect::<Vec<_>>();
    let mut receipts = registration.receipts.clone();
    validate_receipt_set(
        registration.included_number,
        registration.included_hash,
        &hashes,
        registration.block.header.receipts_root,
        &mut receipts,
    )?;
    let index = hashes
        .iter()
        .position(|hash| *hash == registration.hash)
        .context("HOLD: actual registration transaction absent")?;
    let transaction = &transactions[index];
    let receipt = &receipts[index];
    ensure!(
        receipt.status()
            && transaction.chain_id() == Some(intent.old_identity.ethereum_chain_id)
            && transaction.recover_signer()? == receipt.from
            && transaction.to() == receipt.to,
        "HOLD: actual authorized registration signature/domain/result changed"
    );
    let added = receipt
        .inner
        .logs()
        .iter()
        .filter(|log| log.address().as_slice() == intent.erc20_manager.as_bytes())
        .filter_map(|log| IERC20Manager::VftManagerAdded::decode_log(&log.inner).ok())
        .any(|event| event.data.vftManager.0 == intent.new_identity.vft_manager_address.0);
    ensure!(
        added,
        "HOLD: actual operator transaction did not add approved successor to ERC20Manager"
    );
    Ok(())
}
async fn authenticate_registration(
    eth: &PollingEthApi,
    intent: &Intent,
    hash: B256,
) -> Result<Registration> {
    let receipt = eth
        .get_finalized_receipt(hash)
        .await?
        .context("HOLD: authorized EVM registration transaction not finalized")?;
    let block = eth
        .get_block_by_hash(receipt.included_block_hash)
        .full()
        .await?
        .context("HOLD: original EVM registration block unavailable")?;
    let raw_transactions = block
        .transactions
        .clone()
        .try_into_transactions()
        .map_err(|_| anyhow!("HOLD: full original EVM transactions unavailable"))?
        .into_iter()
        .map(|transaction| transaction.into_inner().encoded_2718())
        .collect();
    let receipts = eth
        .get_block_receipts(BlockId::hash_canonical(receipt.included_block_hash))
        .await?
        .context("HOLD: registration complete receipts unavailable")?;
    let registration = Registration {
        hash,
        block,
        raw_transactions,
        receipts,
        included_number: receipt.included_block_number,
        included_hash: receipt.included_block_hash,
        finalized_number: receipt.finalized_block_number,
        finalized_hash: receipt.finalized_block_hash,
    };
    validate_registration(intent, &registration)?;
    let manager = IERC20Manager::new(
        alloy_primitives::Address::from_slice(intent.erc20_manager.as_bytes()),
        &**eth,
    );
    let at = BlockId::hash_canonical(registration.finalized_hash);
    ensure!(
        manager
            .isVftManager(intent.old_identity.vft_manager_address.0.into())
            .block(at)
            .call()
            .await?
            && manager
                .isVftManager(intent.new_identity.vft_manager_address.0.into())
                .block(at)
                .call()
                .await?,
        "HOLD: ERC20Manager must retain source registration and register successor"
    );
    Ok(registration)
}
async fn verify_current_registrations(eth: &PollingEthApi, intent: &Intent) -> Result<()> {
    let finalized = eth.finalized_block().await?;
    ensure!(
        finalized.header.inner.hash_slow() == finalized.header.hash
            && eth
                .is_finalized_block(finalized.header.number, H256(finalized.header.hash.0))
                .await?,
        "HOLD: live registration anchor is not canonical finalized"
    );
    let manager = IERC20Manager::new(
        alloy_primitives::Address::from_slice(intent.erc20_manager.as_bytes()),
        &**eth,
    );
    let pin = BlockId::hash_canonical(finalized.header.hash);
    ensure!(
        manager
            .isVftManager(intent.old_identity.vft_manager_address.0.into())
            .block(pin)
            .call()
            .await?
            && manager
                .isVftManager(intent.new_identity.vft_manager_address.0.into())
                .block(pin)
                .call()
                .await?,
        "HOLD: source/successor ERC20 registration must remain enabled"
    );
    ensure!(
        eth.is_finalized_block(finalized.header.number, H256(finalized.header.hash.0))
            .await?,
        "HOLD: registration anchor changed during readback"
    );
    Ok(())
}
fn validate_lifecycle_report(work: &Work) -> Result<()> {
    let successful = |step: &CutoverStep| {
        work.calls
            .iter()
            .position(|call| &call.step == step && complete(&call.original))
    };
    let begin =
        successful(&CutoverStep::BeginCutover).context("HOLD: original BeginCutover missing")?;
    let pause =
        successful(&CutoverStep::SourcePause).context("HOLD: original source Pause missing")?;
    let prepare = successful(&CutoverStep::PrepareCutover)
        .context("HOLD: original PrepareCutover missing")?;
    let upgrade = work
        .calls
        .iter()
        .position(|call| matches!(call.step, CutoverStep::Upgrade(_)) && complete(&call.original))
        .context("HOLD: successful original Upgrade missing")?;
    let finalize = successful(&CutoverStep::FinalizeCutover)
        .context("HOLD: original FinalizeCutover missing")?;
    ensure!(
        begin < pause && pause < prepare && prepare < upgrade && upgrade < finalize,
        "HOLD: lifecycle original ordering changed"
    );
    let expected = work
        .intent
        .mappings
        .iter()
        .map(|(token, _, _)| *token)
        .chain(work.intent.native_wrapper)
        .collect::<BTreeSet<_>>();
    ensure!(
        work.role_pins.len() == expected.len()
            && work
                .role_pins
                .iter()
                .map(|pin| pin.token)
                .collect::<BTreeSet<_>>()
                == expected,
        "HOLD: incomplete/duplicate finalized role evidence"
    );
    let ready = work.readiness.as_ref().context("HOLD: Ready pin missing")?;
    for roles in &work.role_pins {
        let initial = work
            .intent
            .initial_role_pins
            .iter()
            .find(|pin| pin.token == roles.token)
            .context("HOLD: original role authority missing")?;
        let old = work.intent.old_identity.vft_manager_address;
        let new = work.intent.new_identity.vft_manager_address;
        ensure!(
            roles.block_number <= ready.block_number
                && roles.minter
                    == if initial.minter == old {
                        new
                    } else {
                        initial.minter
                    }
                && roles.burner
                    == if initial.burner == old {
                        new
                    } else {
                        initial.burner
                    },
            "HOLD: final token roles do not preserve/handoff original authorities"
        );
        if initial.minter == old {
            ensure!(
                successful(&CutoverStep::TokenMinter(roles.token)).is_some(),
                "HOLD: minter original handoff missing"
            );
        }
        if work.intent.native_wrapper == Some(roles.token) {
            ensure!(
                roles.manager == Some(new)
                    && roles.outstanding_redemptions == Some(0)
                    && !roles.paused,
                "HOLD: final native wrapper policy differs"
            );
            let mut last = upgrade;
            for step in [
                CutoverStep::WrapperPause,
                CutoverStep::WrapperConfigureManager,
                CutoverStep::WrapperSetBurner,
                CutoverStep::WrapperResume,
            ] {
                let index = successful(&step).context("HOLD: original wrapper handoff missing")?;
                ensure!(
                    last < index && index < finalize,
                    "HOLD: native wrapper original ordering changed"
                );
                last = index;
            }
        } else if initial.burner == old {
            ensure!(
                successful(&CutoverStep::TokenBurner(roles.token)).is_some(),
                "HOLD: burner original handoff missing"
            );
        }
    }
    for (index, call) in work.calls.iter().enumerate() {
        validate_fixed_intent(&work.intent, call)?;
        ensure!(
            call.attempt
                == u32::try_from(
                    work.calls[..index]
                        .iter()
                        .filter(|previous| previous.step == call.step)
                        .count()
                )?,
            "HOLD: lifecycle attempt history is not contiguous"
        );
        if let CutoverStep::Upgrade(ordinal) = call.step {
            ensure!(
                ordinal
                    == u32::try_from(
                        work.calls[..index]
                            .iter()
                            .filter(|previous| matches!(previous.step, CutoverStep::Upgrade(_))
                                && (complete(&previous.original) || terminal_upgrade(previous)))
                            .count()
                    )?,
                "HOLD: custody continuation history changed"
            );
        }
        if let CutoverStep::SourceRefund(request, ordinal) = call.step {
            ensure!(ordinal==u32::try_from(work.calls[..index].iter().filter(|previous|matches!(previous.step,CutoverStep::SourceRefund(prior,_) if prior==request) && definite_refund_rejection(previous)).count())?,"HOLD: definite refund continuation history changed");
        }
    }
    Ok(())
}
async fn handoff_roles(api: &GearApi, path: &Path) -> Result<()> {
    use vft_client::traits::VftAdmin as _;
    use vft_vara_client::traits::{NativeEscrow as _, VftAdmin as _};
    let work = read_work(path)?;
    let old = actor(work.intent.old_identity.vft_manager_address);
    let new = actor(work.intent.new_identity.vft_manager_address);
    for (token, _, _) in &work.intent.mappings {
        if work.intent.native_wrapper == Some(*token) {
            continue;
        }
        let admin = vft_client::VftAdmin::new(GClientRemoting::new(api.clone()));
        let at = api.last_block_hash().await?;
        let pin = sails_rs::H256::from_slice(at.as_ref());
        let minter = admin.minter().at_block(pin).recv(actor(*token)).await?;
        let burner = admin.burner().at_block(pin).recv(actor(*token)).await?;
        let original = work
            .intent
            .initial_role_pins
            .iter()
            .find(|pin| pin.token == *token)
            .context("HOLD: initial token roles missing")?;
        for (role, initial, step, payload) in [
            (
                minter,
                actor(original.minter),
                CutoverStep::TokenMinter(*token),
                vft_client::vft_admin::io::SetMinter::encode_call(new),
            ),
            (
                burner,
                actor(original.burner),
                CutoverStep::TokenBurner(*token),
                vft_client::vft_admin::io::SetBurner::encode_call(new),
            ),
        ] {
            if initial != old {
                ensure!(role == initial, "HOLD: preserved token role changed");
                continue;
            }
            ensure!(
                role == old
                    || (role == new
                        && read_work(path)?
                            .calls
                            .iter()
                            .any(|call| call.step == step && complete(&call.original))),
                "HOLD: token role points to unexpected identity or unjournaled handoff"
            );
            run_cutover_call(api, path, step, actor(*token), &payload).await?;
        }
        let at = api.last_block_hash().await?;
        let pin = sails_rs::H256::from_slice(at.as_ref());
        let expected_minter = if original.minter == hash_actor(old) {
            new
        } else {
            actor(original.minter)
        };
        let expected_burner = if original.burner == hash_actor(old) {
            new
        } else {
            actor(original.burner)
        };
        ensure!(
            admin.minter().at_block(pin).recv(actor(*token)).await? == expected_minter
                && admin.burner().at_block(pin).recv(actor(*token)).await? == expected_burner,
            "HOLD: finalized token role handoff differs"
        );
        record_role_pin(api, path, *token, false).await?;
    }
    if let Some(wrapper) = work.intent.native_wrapper {
        let admin = vft_vara_client::VftAdmin::new(GClientRemoting::new(api.clone()));
        let escrow = vft_vara_client::NativeEscrow::new(GClientRemoting::new(api.clone()));
        let at = api.last_block_hash().await?;
        let pin = sails_rs::H256::from_slice(at.as_ref());
        let current = escrow.manager().at_block(pin).recv(actor(wrapper)).await?;
        let burner = admin.burner().at_block(pin).recv(actor(wrapper)).await?;
        ensure!(
            current == Some(old)
                || (current == Some(new)
                    && read_work(path)?
                        .calls
                        .iter()
                        .any(|call| call.step == CutoverStep::WrapperConfigureManager
                            && complete(&call.original))),
            "HOLD: wrapper manager unexpected/unrecorded"
        );
        ensure!(
            burner == old
                || (burner == new
                    && read_work(path)?
                        .calls
                        .iter()
                        .any(|call| call.step == CutoverStep::WrapperSetBurner
                            && complete(&call.original))),
            "HOLD: wrapper burner unexpected/unrecorded"
        );
        ensure!(
            escrow
                .outstanding_redemptions()
                .at_block(pin)
                .recv(actor(wrapper))
                .await?
                == 0,
            "HOLD: outstanding original native redemption"
        );
        let initial = work
            .intent
            .initial_role_pins
            .iter()
            .find(|pin| pin.token == wrapper)
            .context("HOLD: original wrapper authorities missing")?;
        let minter = admin.minter().at_block(pin).recv(actor(wrapper)).await?;
        if initial.minter == hash_actor(old) {
            ensure!(
                minter == old
                    || (minter == new
                        && read_work(path)?
                            .calls
                            .iter()
                            .any(|call| call.step == CutoverStep::TokenMinter(wrapper)
                                && complete(&call.original))),
                "HOLD: native minter unexpected/unrecorded"
            );
            run_cutover_call(
                api,
                path,
                CutoverStep::TokenMinter(wrapper),
                actor(wrapper),
                &vft_client::vft_admin::io::SetMinter::encode_call(new),
            )
            .await?;
        } else {
            ensure!(
                minter == actor(initial.minter),
                "HOLD: preserved native minter changed"
            );
        }
        run_cutover_call(
            api,
            path,
            CutoverStep::WrapperPause,
            actor(wrapper),
            &vft_vara_client::vft_admin::io::Pause::encode_call(),
        )
        .await?;
        run_cutover_call(
            api,
            path,
            CutoverStep::WrapperConfigureManager,
            actor(wrapper),
            &vft_vara_client::native_escrow::io::ConfigureManager::encode_call(new),
        )
        .await?;
        run_cutover_call(
            api,
            path,
            CutoverStep::WrapperSetBurner,
            actor(wrapper),
            &vft_vara_client::vft_admin::io::SetBurner::encode_call(new),
        )
        .await?;
        let at = api.last_block_hash().await?;
        let pin = sails_rs::H256::from_slice(at.as_ref());
        ensure!(
            escrow.manager().at_block(pin).recv(actor(wrapper)).await? == Some(new)
                && admin.burner().at_block(pin).recv(actor(wrapper)).await? == new
                && escrow
                    .outstanding_redemptions()
                    .at_block(pin)
                    .recv(actor(wrapper))
                    .await?
                    == 0,
            "HOLD: wrapper handoff readback differs"
        );
        run_cutover_call(
            api,
            path,
            CutoverStep::WrapperResume,
            actor(wrapper),
            &vft_vara_client::vft_admin::io::Resume::encode_call(),
        )
        .await?;
        record_role_pin(api, path, wrapper, true).await?;
    }
    Ok(())
}
fn completed_coverage(work: &Work) -> Result<()> {
    let mut expected = 0;
    let mut previous = None;
    for block in &work.blocks {
        ensure!(
            block.number == expected && previous.is_none_or(|hash| hash == block.parent_hash),
            "HOLD: coverage gap/ancestry conflict"
        );
        validate_covered_block(block)?;
        if belongs_to_source(&work.intent, block.header.timestamp)? {
            for receipt in &block.receipts {
                if receipt.status() && is_deposit(receipt, work.intent.erc20_manager)? {
                    let admission = work
                        .admissions
                        .iter()
                        .find(|admission| admission.original_tx_hash == receipt.transaction_hash)
                        .context("HOLD: complete canonical receipt coverage omitted a deposit")?;
                    ensure!(
                        admission.original_block_hash == block.hash
                            && admission.receipt_key.1
                                == receipt.transaction_index.context("Missing receipt index")?,
                        "HOLD: deposit admission coordinates changed"
                    );
                    let mut bytes = admission.proof.as_slice();
                    let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
                    ensure!(
                        bytes.is_empty()
                            && proof.receipt_rlp
                                == alloy_rlp::encode(ethereum_common::utils::map_receipt_envelope(
                                    receipt.as_ref()
                                ))
                            && proof.transaction_index == admission.receipt_key.1
                            && proof.proof_block.block.slot == admission.receipt_key.0,
                        "HOLD: admission proof does not contain the original canonical receipt/key"
                    );
                }
            }
        }
        previous = Some(block.hash);
        expected = expected.checked_add(1).context("Coverage overflow")?;
    }
    ensure!(
        expected
            == work
                .intent
                .boundary
                .execution_number
                .checked_add(1)
                .context("Boundary overflow")?
            && previous == Some(work.intent.boundary.execution_hash),
        "HOLD: coverage is incomplete"
    );
    ensure!(
        work.admissions
            .iter()
            .all(|admission| admission.settlement.is_some()
                && admission.receipt_key.0 <= work.intent.boundary.slot
                && admission.receipt_key.0 >= work.intent.source_receipt_floor
                && (admission.fee_event_observed
                    || admission.admission == "cutover-sponsored"
                    || (admission.admission == "externally-delivered"
                        && admission.external_original.is_some()))),
        "HOLD: economic admissions undrained or policy changed"
    );
    let keys = work
        .admissions
        .iter()
        .map(|admission| admission.receipt_key)
        .collect::<BTreeSet<_>>();
    ensure!(
        keys.len() == work.admissions.len(),
        "HOLD: duplicate coverage admission keys"
    );
    Ok(())
}
pub async fn finalize(
    connection: &GearConnectionArgs,
    api: &GearApi,
    args: &FinalizeCutover,
) -> Result<()> {
    let _owner = lock_work(&args.cutover_dir)?;
    let path = work_path(&args.cutover_dir);
    let mut work = read_work(&path)?;
    let _journals =
        seal::lock_original_journals(&work.intent.old_transactions, &work.intent.old_discovery)?;
    let _signer = lock_signer(&path, api, work.intent.old_identity.gear_genesis_hash)?;
    let _other_signers = lock_other_signers(api, &work.intent, &work, &path)?;
    let eth = PollingEthApi::new(&args.ethereum_rpc).await?;
    let beacon = BeaconClient::new(args.beacon_rpc.clone(), Some(Duration::from_secs(30))).await?;
    authenticate_environment(api, &eth, &work.intent).await?;
    authenticate_boundary(api, &beacon, &work.intent).await?;
    save(&path, &work)?;
    let seal_path = args.cutover_dir.join("cutover.json");
    if seal_path.exists() {
        let (existing, _) = seal::verify_seal_files(&seal_path)?;
        ensure!(
            existing.old_identity == work.intent.old_identity
                && existing.new_identity == work.intent.new_identity,
            "HOLD: seal/work identity changed"
        );
        seal::install_seal_markers(&seal_path)?;
    } else {
        scan_coverage(api, &eth, &beacon, &path, args.sponsor_unpaid).await?;
        drain(api, connection, &eth, &beacon, &path, args.sponsor_unpaid).await?;
        drain_discovery(&eth, &path).await?;
        drain_source(api, &path).await?;
        work = read_work(&path)?;
        completed_coverage(&work)?;
        let old = actor(work.intent.old_identity.vft_manager_address);
        let new = actor(work.intent.new_identity.vft_manager_address);
        let (source, _) = status(api, hash_actor(old)).await?;
        if source.phase == CutoverPhase::Draining {
            run_cutover_call(
                api,
                &path,
                CutoverStep::SourcePause,
                old,
                &io::Pause::encode_call(),
            )
            .await?;
            run_cutover_call(
                api,
                &path,
                CutoverStep::PrepareCutover,
                new,
                &io::PrepareCutover::encode_call(old, work.intent.boundary.slot),
            )
            .await?;
        }
        let (source, _) = status(api, hash_actor(old)).await?;
        if source.phase != CutoverPhase::Retired {
            let ordinal = u32::try_from(
                read_work(&path)?
                    .calls
                    .iter()
                    .filter(|call| {
                        matches!(call.step, CutoverStep::Upgrade(_))
                            && (complete(&call.original) || terminal_upgrade(call))
                    })
                    .count(),
            )?;
            run_cutover_call(
                api,
                &path,
                CutoverStep::Upgrade(ordinal),
                old,
                &io::Upgrade::encode_call(new),
            )
            .await?;
        }
        let (source, retirement) = status(api, hash_actor(old)).await?;
        ensure!(
            source.phase == CutoverPhase::Retired
                && source.completed_transfer_count == source.transfer_count
                && source.in_flight_transfer_count == 0,
            "HOLD: custody transfer pending; resume Upgrade continuation, rollback unsupported"
        );
        work = read_work(&path)?;
        if work.retirement.is_none() {
            work.retirement = Some(retirement);
        }
        save(&path, &work)?;
        handoff_roles(api, &path).await?;
        work = read_work(&path)?;
        if let Some(hash) = args.evm_registration_transaction {
            ensure!(
                work.registration
                    .as_ref()
                    .is_none_or(|saved| saved.hash == hash),
                "HOLD: actual authorized EVM registration intent changed"
            );
            let registration = authenticate_registration(&eth, &work.intent, hash).await?;
            if work.registration.is_none() {
                work.registration = Some(registration);
                save(&path, &work)?;
            }
        }
        let registration=work.registration.as_ref().context("HOLD: execute authorized EVM successor registration externally, retain old registration, then supply --evm-registration-transaction; no EVM signer is used")?;
        verify_current_registrations(&eth, &work.intent).await?;
        validate_registration(&work.intent, registration)?;
        ensure!(
            eth.is_finalized_block(
                registration.included_number,
                H256(registration.included_hash.0)
            )
            .await?
                && eth
                    .is_finalized_block(
                        registration.finalized_number,
                        H256(registration.finalized_hash.0)
                    )
                    .await?,
            "HOLD: original authorized EVM registration history changed"
        );
        run_cutover_call(
            api,
            &path,
            CutoverStep::FinalizeCutover,
            new,
            &io::FinalizeCutover::encode_call(),
        )
        .await?;
        let (destination, readiness) = status(api, hash_actor(new)).await?;
        ensure!(
            destination.phase == CutoverPhase::Ready,
            "HOLD: successor not Ready"
        );
        work = read_work(&path)?;
        if work.readiness.is_none() {
            work.readiness = Some(readiness);
        }
        save(&path, &work)?;
        validate_lifecycle_report(&read_work(&path)?)?;
        let coverage_path = args.cutover_dir.join("cutover-coverage.json");
        let calls_path = args.cutover_dir.join("cutover-calls.json");
        atomic_json(
            &coverage_path,
            &CoverageReport {
                schema_version: 1,
                intent: work.intent.clone(),
                blocks: work.blocks.clone(),
                admissions: work.admissions.clone(),
            },
            true,
        )?;
        atomic_json(
            &calls_path,
            &LifecycleReport {
                schema_version: 1,
                intent: work.intent.clone(),
                calls: work.calls.clone(),
                role_pins: work.role_pins.clone(),
                registration: work.registration.clone().context("Missing registration")?,
            },
            true,
        )?;
        let sealed = CutoverSeal {
            schema_version: 1,
            old_identity: work.intent.old_identity.clone(),
            new_identity: work.intent.new_identity.clone(),
            new_manager_code_id: work.intent.new_manager_code_id,
            source_receipt_floor: work.intent.source_receipt_floor,
            boundary: work.intent.boundary.clone(),
            config_fingerprint: work.intent.config_fingerprint,
            old_state: seal::sealed_file(&work.intent.old_transactions.join("state.json"))?,
            old_blocks: seal::sealed_file(&work.intent.old_transactions.join("blocks.json"))?,
            old_discovery: seal::sealed_file(&work.intent.old_discovery)?,
            coverage: seal::sealed_file(&coverage_path)?,
            lifecycle_evidence: seal::sealed_file(&calls_path)?,
            retirement: work.retirement.clone().context("Missing retirement pin")?,
            readiness: work.readiness.clone().context("Missing readiness pin")?,
        };
        seal::seal_cutover(&seal_path, &sealed)?;
    }
    work = read_work(&path)?;
    let transactions = absolute_new_path(&args.new_transactions)?;
    let discovery = absolute_new_path(&args.new_discovery)?;
    ensure!(
        work.new_transactions
            .as_ref()
            .is_none_or(|previous| previous == &transactions)
            && work
                .new_discovery
                .as_ref()
                .is_none_or(|previous| previous == &discovery),
        "HOLD: successor lane paths changed"
    );
    work.new_transactions = Some(transactions.clone());
    work.new_discovery = Some(discovery.clone());
    save(&path, &work)?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let activation_scanner = sender(
        ApiProviderConnection::from_api(signer.api().clone()),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    if verify_activation(api, &activation_scanner, &work.intent, &seal_path).await? {
        let (sealed, _) = seal::verify_markers(&seal_path)?;
        seal::verify_originals_read_only(&seal_path, activation_scanner.api_provider.clone(), api)
            .await?;
        let new_journal = seal::read_original_journal(&transactions)?;
        ensure!(
            new_journal.runtime_identity == Some(work.intent.new_identity.clone())
                && new_journal.predecessor_cutover
                    == Some(sealed.predecessor(seal::load_seal(&seal_path)?.1)),
            "HOLD: activated successor journal binding changed"
        );
        ensure!(
            !discovery.with_extension("tmp").exists()
                && !PathBuf::from(format!("{}.new", discovery.display())).exists(),
            "HOLD: successor discovery has uncommitted evidence"
        );
        let discovery_state:relayer::message_relayer::common::ethereum::block_storage::DiscoveryState=serde_json::from_slice(&fs::read(&discovery)?)?;
        discovery_state.validate()?;
        ensure!(
            discovery_state.schema_version == 2
                && discovery_state.identity == work.intent.new_identity
                && discovery_state.predecessor_cutover == new_journal.predecessor_cutover
                && discovery_state.start_parent_hash == Some(work.intent.boundary.execution_hash),
            "HOLD: activated successor discovery binding changed"
        );
        verify_current_registrations(&eth, &work.intent).await?;
        release_owned_signers(api, &path, &_signer, &_other_signers)?;
        println!("Cutover already activated; verified original activation without resetting successor journals.");
        return Ok(());
    }
    verify_current_registrations(&eth, &work.intent).await?;
    seal::initialize_successor(&seal_path, &transactions, &discovery).await?;
    seal::authenticate_startup(
        &work.intent.new_identity,
        Some(&seal_path),
        Some(work.intent.new_manager_code_id),
        api,
        &eth,
    )
    .await?;
    run_cutover_call(
        api,
        &path,
        CutoverStep::DestinationUnpause,
        actor(work.intent.new_identity.vft_manager_address),
        &io::Unpause::encode_call(),
    )
    .await?;
    let (destination, active_pin) =
        status(api, work.intent.new_identity.vft_manager_address).await?;
    ensure!(
        destination.phase == CutoverPhase::Active
            && destination.minimum_receipt_slot
                == work
                    .intent
                    .boundary
                    .slot
                    .checked_add(1)
                    .context("Slot overflow")?,
        "HOLD: successor activation/readback differs"
    );
    let activation = read_work(&path)?
        .calls
        .into_iter()
        .find(|call| call.step == CutoverStep::DestinationUnpause && complete(&call.original))
        .context("HOLD: original activation evidence missing")?;
    atomic_json(
        &args.cutover_dir.join("cutover-activation.json"),
        &(seal::load_seal(&seal_path)?.1, activation, active_pin),
        true,
    )?;
    release_owned_signers(api, &path, &_signer, &_other_signers)?;
    println!("Successor runtime: {} --cutover-seal {} --successor-manager-code-id {} ; original journals stay sealed and read-only",serde_json::to_string(&work.intent.new_identity)?,seal_path.display(),hex::encode(work.intent.new_manager_code_id.0));
    Ok(())
}
fn absolute_new_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(fs::canonicalize(path)?);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    Ok(fs::canonicalize(parent)?.join(path.file_name().context("Missing successor path name")?))
}
pub async fn verify(connection: &GearConnectionArgs, args: &VerifyCutover) -> Result<()> {
    let (sealed, _) = seal::verify_markers(&args.seal)?;
    let provider = provider(connection).await?;
    let mut readonly = provider.clone();
    let api = readonly.gclient();
    let eth = PollingEthApi::new(&args.ethereum_rpc).await?;
    let beacon = BeaconClient::new(args.beacon_rpc.clone(), Some(Duration::from_secs(30))).await?;
    let report: CoverageReport = serde_json::from_slice(&fs::read(&sealed.coverage.path)?)?;
    let lifecycle: LifecycleReport =
        serde_json::from_slice(&fs::read(&sealed.lifecycle_evidence.path)?)?;
    ensure!(
        report.schema_version == 1
            && lifecycle.schema_version == 1
            && report.intent == lifecycle.intent
            && report.intent.old_identity == sealed.old_identity
            && report.intent.new_identity == sealed.new_identity
            && report.intent.boundary == sealed.boundary
            && report.intent.config_fingerprint == sealed.config_fingerprint
            && report.intent.new_manager_code_id == sealed.new_manager_code_id,
        "HOLD: sealed protocol evidence identities differ"
    );
    authenticate_environment(&api, &eth, &report.intent).await?;
    authenticate_boundary(&api, &beacon, &report.intent).await?;
    seal::verify_originals_read_only(&args.seal, provider.clone(), &api).await?;
    let work = Work {
        schema_version: 1,
        intent: report.intent.clone(),
        blocks: report.blocks,
        admissions: report.admissions,
        calls: lifecycle.calls,
        role_pins: lifecycle.role_pins,
        registration: Some(lifecycle.registration),
        retirement: Some(sealed.retirement.clone()),
        readiness: Some(sealed.readiness.clone()),
        new_transactions: None,
        new_discovery: None,
    };
    completed_coverage(&work)?;
    validate_lifecycle_report(&work)?;
    let finalized = api.block_number_at(api.last_block_hash().await?).await?;
    for pin in work.intent.initial_role_pins.iter().chain(&work.role_pins) {
        ensure!(
            pin.block_number <= finalized
                && H256::from_slice(api.get_block_hash(pin.block_number).await?.as_ref())
                    == pin.block_hash,
            "HOLD: finalized role state pin changed"
        );
        ensure!(
            read_role_pin(
                &api,
                pin.token,
                work.intent.native_wrapper == Some(pin.token),
                Some(pin.block_hash)
            )
            .await?
                == *pin,
            "HOLD: historical role state changed or unavailable"
        );
    }
    for block in &work.blocks {
        ensure!(
            eth.is_finalized_block(block.number, block.hash).await?,
            "HOLD: sealed canonical coverage changed"
        );
    }
    let mut scanner = sender(
        provider,
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    for original in &work.calls {
        verify_lifecycle_original(&api, &scanner, &work.intent, original).await?;
    }
    scanner.receiver_address = work.intent.old_identity.vft_manager_address;
    for admission in &work.admissions {
        let block = work
            .blocks
            .iter()
            .find(|block| block.hash == admission.original_block_hash)
            .context("HOLD: admission outside canonical coverage")?;
        let receipt = block
            .receipts
            .iter()
            .find(|receipt| receipt.transaction_hash == admission.original_tx_hash)
            .context("HOLD: original admitted receipt missing")?;
        ensure!(
            fee_emitter(&eth, &work.intent, receipt).await? == admission.fee_emitter
                && admission.fee_event_observed == admission.fee_emitter.is_some(),
            "HOLD: transaction-local fee evidence changed"
        );
        if let Some(uuid) = admission.original_lane_uuid {
            let original = seal::read_original_journal(&work.intent.old_transactions)?
                .completed
                .remove(&uuid)
                .context("HOLD: original paid lane UUID missing")?;
            ensure!(
                original.tx.tx_hash == admission.original_tx_hash
                    && original.receipt_event()?.receipt_rlp
                        == alloy_rlp::encode(ethereum_common::utils::map_receipt_envelope(
                            receipt.as_ref()
                        )),
                "HOLD: original paid receipt association changed"
            );
            continue;
        }
        let mut bytes = admission.proof.as_slice();
        let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
        ensure!(bytes.is_empty(), "HOLD: sponsored proof trailing bytes");
        for archived in &admission.attempts {
            verify_proof_attempt(&scanner, &proof, archived).await?;
        }
        if let Some(external) = &admission.external_original {
            ensure!(
                admission.signed.is_none()
                    && admission.attempts.is_empty()
                    && admission.original_lane_uuid.is_none()
                    && admission.admission == "externally-delivered",
                "HOLD: external original provenance/economic outcome conflicts"
            );
            let mut external = external.clone();
            qualify_external_original(
                &api,
                &scanner,
                &proof,
                &mut external,
                admission.settlement.as_ref(),
            )
            .await?;
            continue;
        }
        let signed = admission
            .signed
            .as_ref()
            .context("HOLD: sponsored original missing")?;
        scanner
            .verify_saved_completion(
                &api,
                &proof,
                signed,
                admission
                    .settlement
                    .as_ref()
                    .context("HOLD: saved settlement absent")?,
            )
            .await?;
    }
    let registration = work.registration.as_ref().context("Missing registration")?;
    validate_registration(&work.intent, registration)?;
    ensure!(
        eth.is_finalized_block(
            registration.included_number,
            H256(registration.included_hash.0)
        )
        .await?
            && eth
                .is_finalized_block(
                    registration.finalized_number,
                    H256(registration.finalized_hash.0)
                )
                .await?,
        "HOLD: sealed actual authorized EVM original unavailable"
    );
    verify_current_registrations(&eth, &work.intent).await?;
    verify_activation(&api, &scanner, &work.intent, &args.seal).await?;
    seal::verify_markers(&args.seal)?;
    println!("Verified original sealed journals, canonical coverage, lifecycle originals and historical economic settlement (strictly read-only).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_fulfillment_cannot_hide_a_legacy_handoff_or_completion() {
        use relayer::message_relayer::{
            common::{EthereumSlotNumber, TxHashWithSlot},
            eth_to_gear::tx_manager::{ReceiptEvidence, TxStatus},
        };
        let mut row = Transaction::new(
            TxHashWithSlot {
                tx_hash: B256::repeat_byte(3),
                slot_number: EthereumSlotNumber(32),
            },
            TxStatus::ComposeProof,
        );
        assert!(never_dispatched(&row));
        row.receipt = Some(ReceiptEvidence {
            payload: vec![],
            receipt_key: (32, 0),
            composed_at_ms: 0,
            handed_off_at_ms: Some(1),
            initial_response: None,
            signed_submission: None,
            submission_attempts: vec![],
        });
        assert!(!never_dispatched(&row));
        row.receipt.as_mut().unwrap().handed_off_at_ms = None;
        assert!(never_dispatched(&row));
        row.status = TxStatus::Completed;
        assert!(!never_dispatched(&row));
    }
    #[test]
    fn coverage_rejects_missing_receipts_and_wrong_empty_root() {
        let block = B256::repeat_byte(1);
        let empty_root = alloy_primitives::keccak256([0x80]);
        assert!(validate_receipt_set(0, block, &[], empty_root, &mut []).is_ok());
        assert!(
            validate_receipt_set(0, block, &[B256::repeat_byte(2)], empty_root, &mut []).is_err()
        );
        assert!(validate_receipt_set(0, block, &[], B256::ZERO, &mut []).is_err());
    }
    #[test]
    fn lifecycle_reply_requires_exact_route_and_no_scale_tail() {
        let route = io::Pause::ROUTE;
        assert!(exact_reply::<io::Pause>(route).is_ok());
        let mut tailed = route.to_vec();
        tailed.push(0);
        assert!(exact_reply::<io::Pause>(&tailed).is_err());
        assert!(exact_reply::<io::Pause>(io::Unpause::ROUTE).is_err());
    }
    #[test]
    fn immutable_evidence_is_not_overwritten() {
        let directory = std::env::temp_dir().join(format!("cutover-immutable-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("seal.json");
        atomic_json(&path, &1u32, true).unwrap();
        atomic_json(&path, &1u32, true).unwrap();
        assert!(atomic_json(&path, &2u32, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"1");
        fs::remove_dir_all(directory).unwrap();
    }
}
