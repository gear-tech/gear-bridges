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
use cli_utils::{EthereumFinalityArgs, GearConnectionArgs};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{abi::IERC20Manager, PollingEthApi};
use ethereum_common::tree_hash::TreeHash;
use gclient::GearApi;
use gear_common::{
    api_provider::{ApiProvider, ApiProviderConnection},
    signer_ownership,
};
use gear_core::{
    ids::{ActorId, CodeId},
    program::ProgramState,
};
use primitive_types::{H160, H256};
use relayer::message_relayer::eth_to_gear::{
    cutover::{
        self as seal, is_undispatched_old_admission as never_dispatched, validate_covered_block,
        validate_exclusion_copy, verify_excluded_original, CoveredBlock, CutoverBoundary,
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
    future::Future,
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
    #[command(flatten)]
    pub ethereum_finality: EthereumFinalityArgs,
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
    #[command(flatten)]
    pub ethereum_finality: EthereumFinalityArgs,
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
    #[command(flatten)]
    pub ethereum_finality: EthereumFinalityArgs,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exclusion: Option<FinalizedReceiptObservation>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocking_redemptions: Option<u64>,
    paused: bool,
}
async fn read_role_pin(
    api: &GearApi,
    token: H256,
    wrapper: bool,
    at: Option<H256>,
) -> Result<RolePin> {
    read_role_pin_inner(api, token, wrapper, at, false).await
}

async fn read_role_pin_inner(
    api: &GearApi,
    token: H256,
    wrapper: bool,
    at: Option<H256>,
    allow_drained_inference: bool,
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
    let (manager, outstanding_redemptions, blocking_redemptions) = if wrapper {
        let escrow = vft_vara_client::NativeEscrow::new(GClientRemoting::new(api.clone()));
        let outstanding = escrow
            .outstanding_redemptions()
            .at_block(pin)
            .recv(actor(token))
            .await?;
        let blocking = match escrow
            .blocking_redemptions()
            .at_block(pin)
            .recv(actor(token))
            .await
        {
            Ok(blocking) => blocking,
            Err(_) if allow_drained_inference && outstanding == 0 => 0,
            Err(error) => return Err(error.into()),
        };
        (
            escrow
                .manager()
                .at_block(pin)
                .recv(actor(token))
                .await?
                .map(hash_actor),
            Some(outstanding),
            Some(blocking),
        )
    } else {
        (None, None, None)
    };
    ensure!(
        blocking_redemptions
            .zip(outstanding_redemptions)
            .is_none_or(|(blocking, outstanding)| blocking <= outstanding),
        "HOLD: blocking native obligations exceed outstanding liabilities"
    );
    Ok(RolePin {
        token,
        block_number: api.block_number_at(at).await?,
        block_hash: H256::from_slice(at.as_ref()),
        minter,
        burner,
        manager,
        outstanding_redemptions,
        blocking_redemptions,
        paused,
    })
}

async fn initial_role_pin(
    api: &GearApi,
    path: &Path,
    token: H256,
    wrapper: bool,
    current: H256,
    changed: &'static str,
) -> Result<RolePin> {
    let previous = if path.exists() {
        read_work(path)?
            .intent
            .initial_role_pins
            .into_iter()
            .find(|pin| pin.token == token)
    } else {
        None
    };
    let hash = previous.as_ref().map_or(current, |pin| pin.block_hash);
    let roles = read_role_pin(api, token, wrapper, Some(hash)).await?;
    if let Some(previous) = previous {
        ensure!(
            roles == previous
                && api.get_block_hash(previous.block_number).await?.as_ref()
                    == previous.block_hash.as_bytes(),
            "{changed}"
        );
    }
    Ok(roles)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ethereum_authentication: Option<ethereum_client::FinalityCheckpoint>,
    intent: Intent,
    #[serde(default)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ethereum_authentication: Option<ethereum_client::FinalityCheckpoint>,
    intent: Intent,
    #[serde(default)]
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
                signer_ownership::lock_cutover_signer(
                    intent.old_identity.gear_genesis_hash,
                    sender,
                    path,
                )?
            } else {
                signer_ownership::lock_signer(intent.old_identity.gear_genesis_hash, sender)?
            };
            Ok((sender, file, durable))
        })
        .collect()
}
async fn release_owned_signers(
    api: &GearApi,
    eth: &PollingEthApi,
    path: &Path,
    current: &File,
    others: &[([u8; 32], File, bool)],
) -> Result<()> {
    let mut work = read_work(path)?;
    let mut view = eth.verified_finalized_view().await?;
    authenticate_work_ethereum(eth, &mut view, &mut work).await?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let scanner = sender(
        ApiProviderConnection::from_api(signer.api().clone()),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    release_signers_after_authentication(
        &work,
        path,
        api.account_id().clone().into(),
        current,
        others,
        async {
            for call in &work.calls {
                verify_lifecycle_original(api, &scanner, &work.intent, call).await?;
            }
            for admission in &work.admissions {
                if admission.exclusion.is_some() {
                    authenticate_exclusion(api, &scanner, &work, admission).await?;
                    continue;
                }
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
            view.recheck(&**eth).await?;
            Ok(())
        },
    )
    .await
}

async fn release_signers_after_authentication(
    work: &Work,
    path: &Path,
    current_sender: [u8; 32],
    current: &File,
    others: &[([u8; 32], File, bool)],
    authentication: impl Future<Output = Result<()>>,
) -> Result<()> {
    ensure!(
        work.calls.iter().all(|call| complete(&call.original)
            || terminal_upgrade(call)
            || (call.archived_no_effect && no_effect(&call.original))),
        "HOLD: unresolved lifecycle original retains signer ownership"
    );
    ensure!(
        work.admissions
            .iter()
            .all(|admission| admission.settlement.is_some() ^ admission.exclusion.is_some()),
        "HOLD: unresolved or conflicting admission retains signer ownership"
    );
    // No durable owner is removed until every original has authenticated disposition.
    authentication.await?;
    for (sender, file, durable) in others {
        if *durable {
            signer_ownership::release_cutover_signer(
                work.intent.old_identity.gear_genesis_hash,
                *sender,
                path,
                file,
            )?;
        }
    }
    signer_ownership::release_cutover_signer(
        work.intent.old_identity.gear_genesis_hash,
        current_sender,
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
fn validate_work_version(work: &Work) -> Result<()> {
    ensure!(
        matches!(work.schema_version, 1 | 2 | 3),
        "HOLD: unsupported cutover work schema"
    );
    ensure!(if work.schema_version == 3 {
        work.ethereum_authentication.as_ref().is_some_and(|checkpoint|
            checkpoint.schema_version == 1
                && checkpoint.policy == "independent-two-el-v1"
                && checkpoint.chain_id == work.intent.old_identity.ethereum_chain_id
                && checkpoint.genesis_hash.0 == work.intent.old_identity.ethereum_genesis_hash.0
                && checkpoint.common_header.number >= work.intent.boundary.execution_number
                && work.registration.as_ref().is_none_or(|registration|
                    checkpoint.common_header.number >= registration.included_number))
    } else {
        work.ethereum_authentication.is_none()
    }, "HOLD: cutover finality evidence/schema mismatch");
    if work.schema_version == 1 {
        ensure!(
            work.admissions
                .iter()
                .all(|admission| admission.exclusion.is_none()
                    && admission.admission != "externally-resolved")
                && work
                    .intent
                    .initial_role_pins
                    .iter()
                    .chain(&work.role_pins)
                    .all(|pin| pin.blocking_redemptions.is_none()),
            "HOLD: version-one evidence contains version-two exclusions/counts"
        );
    }
    Ok(())
}

fn zero_blocking(pin: &RolePin, version: u32) -> Result<bool> {
    let outstanding = pin
        .outstanding_redemptions
        .context("HOLD: native financial count missing")?;
    let blocking = match pin.blocking_redemptions {
        Some(blocking) => blocking,
        None if version == 1 && outstanding == 0 => 0,
        None => bail!("HOLD: historical blocking count missing"),
    };
    ensure!(
        blocking <= outstanding,
        "HOLD: blocking count exceeds native liabilities"
    );
    Ok(blocking == 0)
}

fn allow_historical_drained_inference(pin: &RolePin, version: u32) -> bool {
    // A journal upgrade cannot add a getter to the program at its historical block.
    // Only repeat the inference when the retained liabilities and blocking count permit it.
    pin.outstanding_redemptions == Some(0)
        && match version {
            1 => pin.blocking_redemptions.is_none(),
            2 | 3 => pin.blocking_redemptions == Some(0),
            _ => false,
        }
}

fn upgrade_role_evidence(saved: &RolePin, mut observed: RolePin) -> Result<RolePin> {
    let blocking = observed.blocking_redemptions.take();
    ensure!(
        observed == *saved,
        "HOLD: version-one historical role evidence changed"
    );
    observed.blocking_redemptions = match (blocking, observed.outstanding_redemptions) {
        (Some(blocking), Some(outstanding)) if blocking <= outstanding => Some(blocking),
        (None, Some(0)) => Some(0),
        (None, None) => None,
        _ => bail!("HOLD: historical native blocking evidence missing or contradictory"),
    };
    Ok(observed)
}

async fn authenticate_work_ethereum(
    eth: &PollingEthApi,
    view: &mut ethereum_client::FinalizedView,
    work: &mut Work,
) -> Result<()> {
    ensure!(
        eth.finality_identity()?
            == (
                work.intent.old_identity.ethereum_chain_id,
                work.intent.old_identity.ethereum_genesis_hash.0.into()
            ),
        "HOLD: cutover Ethereum approval changed"
    );
    if let Some(checkpoint) = &work.ethereum_authentication {
        view.verify_checkpoint(&**eth, checkpoint).await?;
    }
    let mut targets = std::collections::BTreeMap::from([(
        work.intent.boundary.execution_number,
        work.intent.boundary.execution_hash.0.into(),
    )]);
    if let Some(registration) = &work.registration {
        for (number, hash) in [
            (registration.included_number, registration.included_hash),
            (registration.finalized_number, registration.finalized_hash),
        ] {
            ensure!(
                targets.insert(number, hash).is_none_or(|old| old == hash),
                "HOLD: registration contradicts cutover boundary"
            );
        }
    }
    for block in &work.blocks {
        ensure!(
            targets
                .insert(block.number, block.hash.0.into())
                .is_none_or(|old| old == B256::from(block.hash.0)),
            "HOLD: saved coverage contradicts boundary"
        );
    }
    view.verify_blocks(&**eth, &targets).await?;
    let mut parent = H256::zero();
    for (index, block) in work.blocks.iter().enumerate() {
        ensure!(
            block.number == u64::try_from(index)?
                && block.parent_hash == parent
                && block.number <= work.intent.boundary.execution_number
                && (index != 0 || block.hash == work.intent.old_identity.ethereum_genesis_hash),
            "HOLD: saved coverage is partial, malformed or disconnected"
        );
        validate_covered_block(block)?;
        parent = block.hash;
    }
    // Older scanners could commit admissions before their block. Authenticate only
    // those missing blocks, without advancing the cursor past unrecorded deposits.
    // An empty legacy inventory claims no scanned prefix: its originals may span
    // multiple blocks, and scan_coverage must still visit every height from genesis.
    let mut missing = std::collections::BTreeMap::new();
    for admission in &work.admissions {
        if work.blocks.iter().any(|block| block.hash == admission.original_block_hash) {
            continue;
        }
        let mut bytes = admission.proof.as_slice();
        let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
        let number = proof.proof_block.block.body.execution_payload.block_number;
        ensure!(
            bytes.is_empty()
                && number <= work.intent.boundary.execution_number
                && (work.blocks.is_empty() || number == u64::try_from(work.blocks.len())?),
            "HOLD: saved work has a partial non-prefix inventory"
        );
        let hash = B256::from(admission.original_block_hash.0);
        ensure!(
            missing.insert(number, hash).is_none_or(|previous| previous == hash),
            "HOLD: saved admissions contradict each other's block"
        );
    }
    let mut recovered = Vec::with_capacity(missing.len());
    if !missing.is_empty() {
        view.verify_blocks(&**eth, &missing).await?;
        let started = std::time::Instant::now();
        for (number, hash) in missing {
            ensure!(started.elapsed() < Duration::from_secs(300),
                "HOLD: mutable coverage recovery deadline exhausted");
            recovered.push(seal::fetch_covered_block(eth, number, hash).await?);
        }
    }
    for admission in &work.admissions {
        let block = work
            .blocks
            .iter()
            .chain(&recovered)
            .find(|block| block.hash == admission.original_block_hash)
            .context("HOLD: saved admission has no covered block")?;
        let receipt = block
            .receipts
            .iter()
            .find(|receipt| receipt.transaction_hash == admission.original_tx_hash)
            .context("HOLD: saved admission receipt missing")?;
        let mut bytes = admission.proof.as_slice();
        let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
        let expected = ethereum_common::utils::map_receipt_envelope(receipt.as_ref());
        ensure!(
            bytes.is_empty()
                && proof.receipt_rlp == alloy_rlp::encode(&expected)
                && proof.transaction_index == admission.receipt_key.1
                && proof.proof_block.block.slot == admission.receipt_key.0
                && proof
                    .proof_block
                    .block
                    .body
                    .execution_payload
                    .block_hash
                    .0
                     .0
                    == block.hash.0
                && proof
                    .proof_block
                    .block
                    .body
                    .execution_payload
                    .receipts_root
                    .0
                     .0
                    == block.receipts_root.0
                && block
                    .transactions
                    .get(usize::try_from(admission.receipt_key.1)?)
                    == Some(&admission.original_tx_hash),
            "HOLD: saved admission original linkage changed"
        );
        ethereum_client::inclusion::verify_indexed_value(
            block.header.receipts_root,
            admission.receipt_key.1,
            &expected.encoded_2718(),
            &proof.proof,
        )?;
        ensure!(
            seal::authenticated_fee_emitter(
                eth,
                view,
                work.intent.erc20_manager,
                work.intent.old_identity.bridging_payment_address,
                receipt
            )
            .await?
                == admission.fee_emitter
                && admission.fee_event_observed == admission.fee_emitter.is_some(),
            "HOLD: saved admission fee policy changed"
        );
    }
    if let Some(registration) = &work.registration {
        validate_registration(&work.intent, registration)?;
        verify_registrations_at(
            eth,
            &work.intent,
            view,
            registration.finalized_number,
            registration.finalized_hash,
        )
        .await?;
    }
    view.recheck(&**eth).await?;
    Ok(())
}

async fn upgrade_work(api: &GearApi, eth: &PollingEthApi, path: &Path) -> Result<()> {
    let mut work = read_work(path)?;
    let mut view = eth.verified_finalized_view().await?;
    authenticate_work_ethereum(eth, &mut view, &mut work).await?;
    authenticate_source_emitter(eth, &work.intent.old_identity, work.intent.erc20_manager).await?;
    for pin in work
        .intent
        .initial_role_pins
        .iter_mut()
        .chain(&mut work.role_pins)
    {
        ensure!(
            api.get_block_hash(pin.block_number).await?.as_ref() == pin.block_hash.as_bytes(),
            "HOLD: historical role pin is noncanonical"
        );
        let observed = read_role_pin_inner(
            api,
            pin.token,
            work.intent.native_wrapper == Some(pin.token),
            Some(pin.block_hash),
            allow_historical_drained_inference(pin, work.schema_version),
        )
        .await?;
        if work.schema_version == 1 {
            *pin = upgrade_role_evidence(pin, observed)?;
        } else {
            ensure!(*pin == observed, "HOLD: historical role evidence changed");
        }
    }
    view.recheck(&**eth).await?;
    work.ethereum_authentication = Some(view.checkpoint().clone());
    work.schema_version = 3;
    save(path, &work)
}

fn read_versioned_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(matches!(value["schema_version"].as_u64(), Some(1 | 2 | 3)),
        "HOLD: unsupported cutover evidence schema");
    if value["schema_version"] != 3 {
        ensure!(value.get("ethereum_authentication").is_none(),
            "HOLD: legacy evidence contains an Ethereum authentication field");
    }
    if value["schema_version"] == 1 {
        let admissions = value["admissions"].as_array().into_iter().flatten();
        ensure!(
            admissions
                .into_iter()
                .all(|admission| admission.get("exclusion").is_none()),
            "HOLD: version-one evidence contains an exclusion field"
        );
        for pins in [&value["role_pins"], &value["intent"]["initial_role_pins"]] {
            ensure!(
                pins.as_array()
                    .into_iter()
                    .flatten()
                    .all(|pin| pin.get("blocking_redemptions").is_none()),
                "HOLD: version-one evidence contains a blocking count field"
            );
        }
    }
    Ok(serde_json::from_value(value)?)
}

fn read_work(path: &Path) -> Result<Work> {
    ensure!(
        !PathBuf::from(format!("{}.new", path.display())).exists(),
        "HOLD: interrupted cutover work evidence"
    );
    let work: Work = read_versioned_json(path)?;
    validate_work_version(&work)?;
    validate_admission_uniqueness(&work.admissions)?;
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
    eth: &PollingEthApi,
    path: &Path,
    current: &File,
    others: &[([u8; 32], File, bool)],
) -> Result<bool> {
    let work = read_work(path)?;
    let (state, _) = status(api, work.intent.old_identity.vft_manager_address).await?;
    if state.phase != CutoverPhase::Draining
        || state.successor != Some(actor(work.intent.new_identity.vft_manager_address))
        || state.last_receipt_slot != Some(work.intent.boundary.slot)
        || state.config_fingerprint != Some(work.intent.config_fingerprint.0.into())
    {
        return Ok(false);
    }
    release_owned_signers(api, eth, path, current, others).await?;
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
    signer_ownership::lock_cutover_signer(genesis, api.account_id().clone().into(), path)
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
    // Sails 0.9.2 sends unit results as an empty automatic success reply.
    let mut input = if A::is_empty_tuple::<A::Reply>() {
        bytes
    } else {
        bytes
            .strip_prefix(A::ROUTE)
            .context("HOLD: lifecycle reply route changed")?
    };
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
    reconcile_call(api, scanner, &mut observed, false).await?;
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

fn activation_recovered(call: &mut FinalizedCutoverCall) -> Result<bool> {
    if no_effect(&call.original) {
        call.archived_no_effect = true;
        return Ok(false);
    }
    ensure!(
        complete(&call.original) && !call.archived_no_effect,
        "HOLD: original activation remains unresolved; reconcile without resending"
    );
    validate_call_reply(call)?;
    Ok(true)
}

async fn reconcile_retained_activation(
    api: &GearApi,
    scanner: &MessageSender,
    path: &Path,
) -> Result<bool> {
    let mut work = read_work(path)?;
    let Some(index) = work
        .calls
        .iter()
        .rposition(|call| call.step == CutoverStep::DestinationUnpause && !call.archived_no_effect)
    else {
        return Ok(false);
    };
    let call = &mut work.calls[index];
    validate_fixed_intent(&work.intent, call)?;
    // The successor may already be live. Persist the original's read-only reconciliation
    // before inspecting its journals; never initialize or rebroadcast an ambiguous activation.
    let result = reconcile_call(api, scanner, call, false)
        .await
        .and_then(|()| activation_recovered(call));
    save(path, &work)?;
    result
}

fn validate_successor_journals(
    identity: &InboundRuntimeIdentity,
    predecessor: &seal::PredecessorCutover,
    boundary_hash: H256,
    transactions: &Path,
    discovery: &Path,
) -> Result<()> {
    let journal = seal::read_original_journal(transactions)?;
    ensure!(
        journal.schema_version == 7
            && journal.runtime_identity.as_ref() == Some(identity)
            && journal.predecessor_cutover.as_ref() == Some(predecessor),
        "HOLD: activated successor journal binding changed"
    );
    let blocks_path = transactions.join("blocks.json");
    for path in [blocks_path.as_path(), discovery] {
        for pending in [
            PathBuf::from(format!("{}.new", path.display())),
            PathBuf::from(format!("{}.tmp", path.display())),
            path.with_extension("tmp"),
        ] {
            ensure!(
                fs::symlink_metadata(&pending)
                    .is_err_and(|error| { error.kind() == std::io::ErrorKind::NotFound }),
                "HOLD: successor journal has uncommitted evidence"
            );
        }
        ensure!(
            fs::symlink_metadata(path)?.file_type().is_file(),
            "HOLD: successor snapshot is not a regular committed file"
        );
    }
    let _: std::collections::BTreeMap<
        relayer::message_relayer::common::EthereumSlotNumber,
        relayer::message_relayer::eth_to_gear::storage::Block,
    > = serde_json::from_slice(&fs::read(&blocks_path)?)?;
    let state: relayer::message_relayer::common::ethereum::block_storage::DiscoveryState =
        serde_json::from_slice(&fs::read(discovery)?)?;
    state.validate()?;
    ensure!(
        state.schema_version == 2
            && &state.identity == identity
            && state.predecessor_cutover.as_ref() == Some(predecessor)
            && state.start_parent_hash == Some(boundary_hash),
        "HOLD: activated successor discovery binding changed"
    );
    Ok(())
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
            call.original.observe_reply(&chain, id, call.target).await?;
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
fn lifecycle_request(intent: &Intent, step: &CutoverStep) -> Result<(ActorId, Vec<u8>)> {
    let old = actor(intent.old_identity.vft_manager_address);
    let new = actor(intent.new_identity.vft_manager_address);
    let wrapper = || {
        intent
            .native_wrapper
            .map(actor)
            .context("HOLD: missing native policy")
    };
    Ok(match *step {
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
        CutoverStep::TokenMinter(token) => (
            actor(token),
            vft_client::vft_admin::io::SetMinter::encode_call(new),
        ),
        CutoverStep::TokenBurner(token) => (
            actor(token),
            vft_client::vft_admin::io::SetBurner::encode_call(new),
        ),
        CutoverStep::WrapperPause => (
            wrapper()?,
            vft_vara_client::vft_admin::io::Pause::encode_call(),
        ),
        CutoverStep::WrapperConfigureManager => (
            wrapper()?,
            vft_vara_client::native_escrow::io::ConfigureManager::encode_call(new),
        ),
        CutoverStep::WrapperSetBurner => (
            wrapper()?,
            vft_vara_client::vft_admin::io::SetBurner::encode_call(new),
        ),
        CutoverStep::WrapperResume => (
            wrapper()?,
            vft_vara_client::vft_admin::io::Resume::encode_call(),
        ),
    })
}

fn validate_fixed_intent(intent: &Intent, call: &FinalizedCutoverCall) -> Result<()> {
    match call.step {
        CutoverStep::TokenMinter(token) => ensure!(
            intent.native_wrapper == Some(token)
                || intent.mappings.iter().any(|(id, _, _)| *id == token),
            "HOLD: unbound token role"
        ),
        CutoverStep::TokenBurner(token) => ensure!(
            intent.mappings.iter().any(|(id, _, _)| *id == token),
            "HOLD: unbound token role"
        ),
        _ => {}
    }
    let (target, payload) = lifecycle_request(intent, &call.step)?;
    ensure!(
        call.target == hash_actor(target)
            && call.payload == payload
            && call.gas > 0
            && call.value == 0,
        "HOLD: fixed lifecycle intent differs from immutable cutover"
    );
    Ok(())
}
fn receipt_call(
    scanner: &MessageSender,
    proof: &eth_events_electra_client::EthToVaraEvent,
) -> Vec<u8> {
    historical_proxy_client::historical_proxy::io::Redirect::encode_call(
        proof.proof_block.block.slot,
        proof.encode(),
        actor(scanner.receiver_address),
        scanner.receiver_route.clone(),
    )
}

async fn verify_proof_attempt(
    scanner: &MessageSender,
    proof: &eth_events_electra_client::EthToVaraEvent,
    signed: &SignedSubmission,
) -> Result<()> {
    let call = receipt_call(scanner, proof);
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
    let payload = receipt_call(scanner, proof);
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

async fn reconcile_original_response(
    api: &GearApi,
    scanner: &MessageSender,
    identity: &InboundRuntimeIdentity,
    transaction: &Transaction,
    proof: &eth_events_electra_client::EthToVaraEvent,
    mut signed: SignedSubmission,
) -> Response {
    let (status, outcome) = reconcile_original(
        api,
        scanner,
        proof,
        &mut signed,
        transaction.saved_settlement_observation.as_ref(),
    )
    .await
    .unwrap_or_else(|error| {
        (
            MessageStatus::NeedsReconciliation {
                receipt_key: signed.receipt_key,
                diagnostic: format!("HOLD: {error:#}"),
                receipt_observation: None,
            },
            None,
        )
    });
    Response {
        tx_uuid: transaction.uuid,
        status,
        submission: outcome.map(|reply| SubmissionEvidence {
            observed_at_ms: relayer::message_relayer::eth_to_gear::message_sender::now_ms(),
            manager: identity.vft_manager_address,
            historical_proxy: identity.historical_proxy_address,
            sender: signed.sender.clone(),
            manager_reply: reply.ok(),
            error: None,
        }),
        signed_submission: Some(signed),
    }
}

async fn settle_original_nonces(api: &GearApi, scanner: &MessageSender, work: &Work) -> Result<()> {
    let intent = &work.intent;
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
        if let Some(admission) = work.admissions.iter().find(|admission| {
            admission.original_lane_uuid == Some(transaction.uuid) && admission.exclusion.is_some()
        }) {
            authenticate_exclusion(api, scanner, work, admission).await?;
            continue;
        }
        let proof = transaction.receipt_event()?;
        let signed = transaction
            .receipt
            .as_ref()
            .and_then(|receipt| receipt.signed_submission.clone())
            .context("Missing original")?;
        if let Some(pin) = scanner.receipt_resolution_status(api, &proof, None).await? {
            ensure!(
                signed.sender.parse::<subxt::utils::AccountId32>()?.0
                    == intent.old_identity.gear_sender,
                "HOLD: excluded original signer differs from owned lane"
            );
            observe_excluded_original(scanner, &intent.old_identity, &proof, &signed, &pin).await?;
            for archived in &transaction
                .receipt
                .as_ref()
                .context("HOLD: original proof missing")?
                .submission_attempts
            {
                ensure!(
                    archived
                        .signed_submission
                        .sender
                        .parse::<subxt::utils::AccountId32>()?
                        .0
                        == intent.old_identity.gear_sender,
                    "HOLD: archived owned signer changed"
                );
                observe_excluded_original(
                    scanner,
                    &intent.old_identity,
                    &proof,
                    &archived.signed_submission,
                    &pin,
                )
                .await?;
            }
            continue;
        }
        let response = reconcile_original_response(
            api,
            scanner,
            &intent.old_identity,
            &transaction,
            &proof,
            signed,
        )
        .await;
        let settled = matches!(
            response.status,
            MessageStatus::Success { .. } | MessageStatus::RetryableNoop { .. }
        );
        manager.finalize_transaction(response).await?;
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
    eth: &PollingEthApi,
    work_path: &Path,
    step: CutoverStep,
    intent: &Intent,
) -> Result<FinalizedCutoverCall> {
    let (target, payload) = lifecycle_request(intent, &step)?;
    run_cutover_request(api, eth, work_path, step, target, &payload).await
}

async fn run_cutover_request(
    api: &GearApi,
    eth: &PollingEthApi,
    work_path: &Path,
    step: CutoverStep,
    target: ActorId,
    payload: &[u8],
) -> Result<FinalizedCutoverCall> {
    let mut work = read_work(work_path)?;
    let mut view = eth.verified_finalized_view().await?;
    authenticate_work_ethereum(eth, &mut view, &mut work).await?;
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
        &work,
    )
    .await?;
    // No new nonce while any earlier lifecycle original is unresolved.
    for index in 0..work.calls.len() {
        let call = &mut work.calls[index];
        validate_fixed_intent(&work.intent, call)?;
        let result = reconcile_call(api, &scanner, call, false).await;
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
        view.recheck(&**eth).await?;
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
        view.recheck(&**eth).await?;
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
    let mut view = eth.verified_finalized_view().await?;
    seal::authenticated_fee_emitter(eth, &mut view, intent.erc20_manager,
        intent.old_identity.bridging_payment_address, receipt).await
}

fn belongs_to_source(intent: &Intent, timestamp: u64) -> Result<bool> {
    seal::belongs_to_source(intent.source_receipt_floor, &intent.boundary,
        intent.boundary_execution_timestamp, timestamp)
}

fn external_parent_result(
    proof: &[u8],
    signed: &SignedSubmission,
) -> Option<Result<(), vft_manager_client::Error>> {
    let mut bytes = proof;
    let event = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes).ok()?;
    if !bytes.is_empty() || !complete(signed) {
        return None;
    }
    let (receipt, reply) = exact_reply::<historical_proxy_client::historical_proxy::io::Redirect>(
        &signed.finalized_reply.as_ref()?.payload,
    )
    .ok()?
    .ok()?;
    if receipt != event.receipt_rlp {
        return None;
    }
    exact_reply::<io::SubmitReceipt>(&reply).ok()
}

fn accepted_external_parent(proof: &[u8], signed: &SignedSubmission) -> bool {
    matches!(
        external_parent_result(proof, signed),
        Some(Ok(()) | Err(vft_manager_client::Error::NativeSettlementPending))
    )
}
fn refresh_pending_external_original(
    previous: &mut seal::CanonicalExternalOriginal,
    fresh: &seal::CanonicalExternalOriginal,
) -> Result<()> {
    let old = &previous.original;
    let new = &fresh.original;
    ensure!(
        previous.provenance == "canonical-external"
            && fresh.provenance == previous.provenance
            && new.same_request(old)
            && new.inclusion_block_number == old.inclusion_block_number
            && new.inclusion_block_hash == old.inclusion_block_hash
            && new.message_id == old.message_id
            && new.finalized_dispatch_error == old.finalized_dispatch_error
            && new.finalized_reply == old.finalized_reply
            && new.finalized_reply_error == old.finalized_reply_error
            && new.finalized_nonce_consumed == old.finalized_nonce_consumed,
        "HOLD: canonical external original identity/history changed"
    );
    new.validate_native_reconciliation()?;
    // These are canonical public-discovery projections, not locally owned signatures.
    // Keep the parent and its original discovery pin for historical reauthentication.
    if previous.original.native_reconciliation_attempts != new.native_reconciliation_attempts {
        previous
            .original
            .native_reconciliation_attempts
            .clone_from(&new.native_reconciliation_attempts);
    }
    if previous.original.native_reconciliation != new.native_reconciliation {
        previous
            .original
            .native_reconciliation
            .clone_from(&new.native_reconciliation);
    }
    Ok(())
}

struct ExternalReceiptCandidate {
    proof: Option<Vec<u8>>,
    receipt_key: (u64, u64),
    destination: H256,
}
fn external_receipt_candidate(
    destination: H256,
    payload: &[u8],
    proxy: H256,
    receiver: H256,
    receiver_route: &[u8],
) -> Option<ExternalReceiptCandidate> {
    use historical_proxy_client::historical_proxy::io::Redirect;
    // Public requests are candidates, not retained authenticated evidence.
    if destination == proxy {
        let mut payload = payload.strip_prefix(Redirect::ROUTE)?;
        let (slot, proof, target, route) =
            <(u64, Vec<u8>, ActorId, Vec<u8>)>::decode(&mut payload).ok()?;
        if !payload.is_empty() || target != actor(receiver) || route != receiver_route {
            return None;
        }
        let mut bytes = proof.as_slice();
        let event = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes).ok()?;
        if !bytes.is_empty() || slot != event.proof_block.block.slot {
            return None;
        }
        Some(ExternalReceiptCandidate {
            proof: Some(proof),
            receipt_key: (slot, event.transaction_index),
            destination: proxy,
        })
    } else if destination == receiver {
        let mut payload = payload.strip_prefix(io::ReconcileReceipt::ROUTE)?;
        let key = <(u64, u64)>::decode(&mut payload).ok()?;
        payload.is_empty().then_some(ExternalReceiptCandidate {
            proof: None,
            receipt_key: key,
            destination: receiver,
        })
    } else {
        None
    }
}
fn attach_external_native_continuations(
    proof: &[u8],
    original: &mut SignedSubmission,
    continuations: &[SignedSubmission],
) -> Result<()> {
    let pending_native = matches!(
        external_parent_result(proof, original),
        Some(Err(vft_manager_client::Error::NativeSettlementPending))
    );
    let mut native = continuations
        .iter()
        .filter(|continuation| {
            pending_native
                && continuation.sender == original.sender
                && continuation.receipt_key == original.receipt_key
                && continuation.nonce > original.nonce
                && original.finalized_reply.as_ref().is_some_and(|reply| {
                    reply
                        .observation
                        .finalized_block_number
                        .is_some_and(|number| continuation.prepared_finalized_number >= number)
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    native.sort_unstable_by_key(|continuation| continuation.nonce);
    // Public calls after original completion cannot acquire its reconciliation ownership.
    if let Some(completed) = native.iter().position(|continuation| {
        complete(continuation)
            && continuation.finalized_reply.as_ref().is_some_and(|reply| {
                matches!(
                    exact_reply::<io::ReconcileReceipt>(&reply.payload),
                    Ok(Ok(vft_manager_client::ReceiptStatus::Processed
                        | vft_manager_client::ReceiptStatus::Resolved))
                )
            })
    }) {
        native.truncate(completed + 1);
    }
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
    original.validate_native_reconciliation()
}
async fn discover_external_originals(
    scanner: &MessageSender,
    genesis: H256,
    until: Option<&FinalizedReceiptObservation>,
) -> Result<Vec<(Vec<u8>, seal::CanonicalExternalOriginal)>> {
    let chain = scanner.api_provider.client();
    ensure!(
        chain.api.genesis_hash().as_ref() == genesis.as_bytes()
            && chain.block_number_to_hash(0).await? == genesis,
        "HOLD: external original discovery genesis changed"
    );
    let latest = chain.latest_finalized_block().await?;
    let latest_number = chain.block_hash_to_number(latest).await?;
    let (head, end) = match until {
        Some(pin) => {
            let number = pin
                .finalized_block_number
                .context("HOLD: external discovery cutoff height missing")?;
            let hash = H256::from_str(&pin.finalized_block_hash)?;
            ensure!(
                number <= latest_number && chain.block_number_to_hash(number).await? == hash,
                "HOLD: external discovery cutoff is not canonical finalized"
            );
            (hash, number)
        }
        None => (latest, latest_number),
    };
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
            let Some(ExternalReceiptCandidate {
                proof,
                receipt_key: key,
                destination,
            }) = external_receipt_candidate(
                H256::from_slice(call.destination.as_ref()),
                &call.payload,
                scanner.historical_proxy_address,
                scanner.receiver_address,
                &scanner.receiver_route,
            )
            else {
                continue;
            };
            if call.value != 0 {
                continue;
            }
            let Some(address) = extrinsic.address_bytes() else {
                continue;
            };
            if address.len() != 33 || address[0] != 0 {
                continue;
            }
            let account = subxt::utils::AccountId32(address[1..].try_into()?);
            let Some(nonce) = extrinsic
                .transaction_extensions()
                .and_then(|extensions| extensions.nonce())
            else {
                continue;
            };
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
                prepared_finalized_number: if proof.is_none() { number - 1 } else { 0 },
                prepared_finalized_hash: if proof.is_none() {
                    format!("{previous:#x}")
                } else {
                    format!("{genesis:#x}")
                },
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
            if let Some(proof) = proof {
                if original.message_id.is_some() {
                    proofs.push((proof, original));
                }
            } else {
                continuations.push(original);
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
        finalized_block_number: Some(end),
        finalized_block_hash: format!("{head:#x}"),
    };
    let mut found = Vec::with_capacity(proofs.len());
    for (proof, mut original) in proofs {
        attach_external_native_continuations(&proof, &mut original, &continuations)?;
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
fn preserves_original(observed: &SignedSubmission, saved: &SignedSubmission) -> bool {
    validate_exclusion_copy(saved, observed).is_ok()
}

async fn observe_excluded_call(
    scanner: &MessageSender,
    signed: &mut SignedSubmission,
    destination: H256,
    call: &[u8],
    exclusion: &FinalizedReceiptObservation,
) -> Result<()> {
    let saved = signed.clone();
    let chain = scanner.api_provider.client();
    let number = exclusion
        .finalized_block_number
        .context("HOLD: exclusion height missing")?;
    let hash = H256::from_str(&exclusion.finalized_block_hash)?;
    ensure!(
        signed.prepared_finalized_number <= number
            && signed.scanned_finalized_number <= number
            && signed.reply_scanned_finalized_number <= number,
        "HOLD: original observations postdate exclusion"
    );
    // Saved dispositions are reauthenticated by the shared verifier, not replaced
    // by a newer account nonce or a later scan cursor.
    if signed.has_terminal_reconciliation() {
        return Ok(());
    }
    let found = scanner
        .find_signed_extrinsic(signed, destination, call)
        .await?;
    if signed.scanned_finalized_number < saved.scanned_finalized_number {
        signed.scanned_finalized_number = saved.scanned_finalized_number;
        signed.scanned_finalized_hash = saved.scanned_finalized_hash.clone();
    }
    if found {
        if let Some(id) = signed
            .message_id
            .as_deref()
            .map(H256::from_str)
            .transpose()?
        {
            scanner
                .find_finalized_reply(signed, id, destination)
                .await?;
        }
    } else {
        let nonce = u64::from(chain.api.info_at(&signed.sender, Some(hash)).await?.nonce);
        ensure!(
            nonce > signed.nonce,
            "HOLD: excluded original nonce remains unconsumed"
        );
        signed.scanned_finalized_number = number;
        signed.scanned_finalized_hash = exclusion.finalized_block_hash.clone();
        signed.finalized_nonce_consumed = Some(nonce);
    }
    ensure!(
        preserves_original(signed, &saved),
        "HOLD: exclusion discovery changed retained original observations"
    );
    Ok(())
}

async fn observe_excluded_original(
    scanner: &MessageSender,
    identity: &InboundRuntimeIdentity,
    proof: &eth_events_electra_client::EthToVaraEvent,
    original: &SignedSubmission,
    exclusion: &FinalizedReceiptObservation,
) -> Result<SignedSubmission> {
    let mut copy = original.clone();
    observe_excluded_call(
        scanner,
        &mut copy,
        scanner.historical_proxy_address,
        &receipt_call(scanner, proof),
        exclusion,
    )
    .await?;
    let call = io::ReconcileReceipt::encode_call(original.receipt_key.0, original.receipt_key.1);
    for continuation in copy
        .native_reconciliation_attempts
        .iter_mut()
        .chain(copy.native_reconciliation.as_deref_mut())
    {
        observe_excluded_call(
            scanner,
            continuation,
            scanner.receiver_address,
            &call,
            exclusion,
        )
        .await?;
    }
    ensure!(
        preserves_original(&copy, original),
        "HOLD: exclusion work copy replaced original evidence"
    );
    verify_excluded_original(
        scanner,
        &scanner.api_provider,
        identity,
        proof,
        &copy,
        exclusion,
    )
    .await?;
    Ok(copy)
}

fn validate_owned_exclusion(
    identity: &InboundRuntimeIdentity,
    admission: &Admission,
    retained: &Transaction,
) -> Result<()> {
    let receipt = retained
        .receipt
        .as_ref()
        .context("HOLD: excluded original proof missing")?;
    let saved = receipt
        .signed_submission
        .as_ref()
        .context("HOLD: excluded owned signature missing")?;
    let signed = admission
        .signed
        .as_ref()
        .context("HOLD: exclusion work copy missing")?;
    ensure!(
        admission.original_lane_uuid == Some(retained.uuid)
            && retained.tx.tx_hash == admission.original_tx_hash
            && retained.receipt_event()?.encode() == admission.proof
            && signed.sender.parse::<subxt::utils::AccountId32>()?.0 == identity.gear_sender
            && preserves_original(signed, saved)
            && admission.attempts.len() == receipt.submission_attempts.len()
            && admission
                .attempts
                .iter()
                .zip(&receipt.submission_attempts)
                .all(
                    |(copy, original)| preserves_original(copy, &original.signed_submission)
                        && copy
                            .sender
                            .parse::<subxt::utils::AccountId32>()
                            .is_ok_and(|sender| sender.0 == identity.gear_sender)
                ),
        "HOLD: excluded original journal linkage changed"
    );
    Ok(())
}

async fn authenticate_exclusion(
    api: &GearApi,
    scanner: &MessageSender,
    work: &Work,
    admission: &Admission,
) -> Result<()> {
    let pin = admission
        .exclusion
        .as_ref()
        .context("HOLD: exclusion pin missing")?;
    ensure!(
        matches!(work.schema_version, 2 | 3) && admission.settlement.is_none(),
        "HOLD: settlement/exclusion conflict"
    );
    let mut bytes = admission.proof.as_slice();
    let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
    ensure!(bytes.is_empty(), "HOLD: exclusion proof trailing bytes");
    ensure!(
        scanner
            .receipt_resolution_status(api, &proof, Some(pin))
            .await?
            .as_ref()
            == Some(pin),
        "HOLD: historical terminal resolution missing"
    );
    if let Some(retirement) = &work.retirement {
        ensure!(
            pin.finalized_block_number
                .is_some_and(|number| number <= retirement.block_number),
            "HOLD: exclusion postdates retirement"
        );
    }
    let journal = seal::read_original_journal(&work.intent.old_transactions)?;
    for retained in journal
        .transactions
        .values()
        .chain(journal.completed.values())
        .filter(|row| {
            row.tx.tx_hash == admission.original_tx_hash
                && row
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.signed_submission.is_some())
        })
    {
        ensure!(
            admission.original_lane_uuid == Some(retained.uuid)
                && journal.transactions.contains_key(&retained.uuid),
            "HOLD: known owned signature cannot be replaced by foreign/sponsored evidence"
        );
    }
    let original = if let Some(external) = &admission.external_original {
        ensure!(
            external.provenance == "canonical-external"
                && admission.admission == "externally-resolved"
                && admission.original_lane_uuid.is_none()
                && admission.signed.is_none()
                && admission.attempts.is_empty(),
            "HOLD: foreign exclusion provenance conflicts"
        );
        &external.original
    } else {
        ensure!(
            admission.admission
                == if admission.fee_event_observed {
                    "paid"
                } else {
                    "cutover-sponsored"
                },
            "HOLD: owned exclusion fee provenance changed"
        );
        let signed = admission
            .signed
            .as_ref()
            .context("HOLD: excluded signed original missing")?;
        if let Some(uuid) = admission.original_lane_uuid {
            let journal = seal::read_original_journal(&work.intent.old_transactions)?;
            let retained = journal
                .transactions
                .get(&uuid)
                .context("HOLD: excluded original active UUID missing")?;
            validate_owned_exclusion(&work.intent.old_identity, admission, retained)?;
        }
        signed
    };
    for archived in &admission.attempts {
        verify_excluded_original(
            scanner,
            &scanner.api_provider,
            &work.intent.old_identity,
            &proof,
            archived,
            pin,
        )
        .await?;
    }
    verify_excluded_original(
        scanner,
        &scanner.api_provider,
        &work.intent.old_identity,
        &proof,
        original,
        pin,
    )
    .await
}

async fn record_resolution(
    api: &GearApi,
    scanner: &MessageSender,
    path: &Path,
    work: &mut Work,
    index: usize,
) -> Result<bool> {
    if work.admissions[index].exclusion.is_some() {
        authenticate_exclusion(api, scanner, work, &work.admissions[index]).await?;
        return Ok(true);
    }
    if work.admissions[index].settlement.is_some() {
        return Ok(false);
    }
    let mut bytes = work.admissions[index].proof.as_slice();
    let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
    ensure!(bytes.is_empty(), "HOLD: admission proof trailing bytes");
    let retirement_pin = work
        .retirement
        .as_ref()
        .map(|retirement| FinalizedReceiptObservation {
            finalized_block_number: Some(retirement.block_number),
            finalized_block_hash: format!("{:#x}", retirement.block_hash),
        });
    let Some(pin) = scanner
        .receipt_resolution_status(api, &proof, retirement_pin.as_ref())
        .await?
    else {
        return Ok(false);
    };
    let journal = seal::read_original_journal(&work.intent.old_transactions)?;
    let owned = journal.transactions.values().find(|row| {
        row.tx.tx_hash == work.admissions[index].original_tx_hash
            && row
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.signed_submission.is_some())
    });
    let admission = &mut work.admissions[index];
    if owned.is_none() && admission.signed.is_none() && !admission.attempts.is_empty() {
        admission.signed = admission.attempts.pop();
    }
    if let Some(row) = owned {
        ensure!(
            row.receipt_event()?.encode() == admission.proof,
            "HOLD: owned exclusion proof identity changed"
        );
        let receipt = row.receipt.as_ref().context("HOLD: owned proof missing")?;
        let signed = receipt
            .signed_submission
            .as_ref()
            .context("HOLD: owned signature missing")?;
        ensure!(
            signed.sender.parse::<subxt::utils::AccountId32>()?.0
                == work.intent.old_identity.gear_sender,
            "HOLD: retained owned signer changed"
        );
        ensure!(
            admission.external_original.is_none(),
            "HOLD: foreign original cannot replace owned signature"
        );
        let previous_signed = admission.signed.as_ref();
        if let Some(previous) = previous_signed {
            validate_exclusion_copy(signed, previous)?;
        } else {
            ensure!(
                admission.attempts.is_empty(),
                "HOLD: sponsored attempts conflict with retained owned signature"
            );
        }
        let copy = observe_excluded_original(
            scanner,
            &work.intent.old_identity,
            &proof,
            previous_signed.unwrap_or(signed),
            &pin,
        )
        .await?;
        validate_exclusion_copy(signed, &copy)?;
        let mut attempts = Vec::with_capacity(receipt.submission_attempts.len());
        ensure!(
            admission.attempts.is_empty()
                || admission.attempts.len() == receipt.submission_attempts.len(),
            "HOLD: retained owned archived proof history changed"
        );
        for (index, archived) in receipt.submission_attempts.iter().enumerate() {
            ensure!(
                archived
                    .signed_submission
                    .sender
                    .parse::<subxt::utils::AccountId32>()?
                    .0
                    == work.intent.old_identity.gear_sender,
                "HOLD: archived owned signer changed"
            );
            let saved = admission
                .attempts
                .get(index)
                .unwrap_or(&archived.signed_submission);
            validate_exclusion_copy(&archived.signed_submission, saved)?;
            attempts.push(
                observe_excluded_original(scanner, &work.intent.old_identity, &proof, saved, &pin)
                    .await?,
            );
        }
        admission.signed = Some(copy);
        admission.attempts = attempts;
        admission.original_lane_uuid = Some(row.uuid);
        admission.undispatched_old_admission = None;
        admission.external_original = None;
        admission.admission = if admission.fee_event_observed {
            "paid"
        } else {
            "cutover-sponsored"
        }
        .into();
    } else if let Some(signed) = &admission.signed {
        admission.signed = Some(
            observe_excluded_original(scanner, &work.intent.old_identity, &proof, signed, &pin)
                .await?,
        );
        for archived in &mut admission.attempts {
            *archived = observe_excluded_original(
                scanner,
                &work.intent.old_identity,
                &proof,
                archived,
                &pin,
            )
            .await?;
        }
    } else {
        if admission.external_original.is_none() {
            let candidates = discover_external_originals(
                scanner,
                work.intent.old_identity.gear_genesis_hash,
                Some(&pin),
            )
            .await?;
            admission.external_original = candidates
                .into_iter()
                .find(|(bytes, _)| *bytes == admission.proof)
                .map(|(_, external)| external);
        }
        let external = admission
            .external_original
            .as_mut()
            .context("HOLD: resolved receipt lacks authenticated original provenance")?;
        external.original = observe_excluded_original(
            scanner,
            &work.intent.old_identity,
            &proof,
            &external.original,
            &pin,
        )
        .await?;
        admission.undispatched_old_admission = journal
            .transactions
            .values()
            .find(|row| row.tx.tx_hash == admission.original_tx_hash && never_dispatched(row))
            .map(|row| row.uuid);
        admission.admission = "externally-resolved".into();
    }
    admission.exclusion = Some(pin);
    authenticate_exclusion(api, scanner, work, &work.admissions[index]).await?;
    save(path, work)?;
    Ok(true)
}

enum QualifiedExternal {
    Settled(FinalizedReceiptObservation),
    Excluded(FinalizedReceiptObservation),
}

async fn qualify_external_original(
    api: &GearApi,
    scanner: &MessageSender,
    proof: &eth_events_electra_client::EthToVaraEvent,
    external: &mut seal::CanonicalExternalOriginal,
    saved: Option<&FinalizedReceiptObservation>,
    identity: &InboundRuntimeIdentity,
    exclusion: Option<&FinalizedReceiptObservation>,
) -> Result<QualifiedExternal> {
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
    if let Some(pin) = exclusion.cloned().or(if saved.is_none() {
        scanner.receipt_resolution_status(api, proof, None).await?
    } else {
        None
    }) {
        ensure!(
            saved.is_none()
                && scanner
                    .receipt_resolution_status(api, proof, Some(&pin))
                    .await?
                    .as_ref()
                    == Some(&pin),
            "HOLD: external settlement/exclusion conflicts"
        );
        external.original =
            observe_excluded_original(scanner, identity, proof, &external.original, &pin).await?;
        return Ok(QualifiedExternal::Excluded(pin));
    }
    if let Some(saved) = saved {
        scanner
            .verify_saved_completion(api, proof, &external.original, saved)
            .await?;
        return Ok(QualifiedExternal::Settled(saved.clone()));
    }
    let payload = receipt_call(scanner, proof);
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
            return Ok(QualifiedExternal::Settled(pin));
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
    let mut view = eth.verified_finalized_view().await?;
    authenticate_work_ethereum(eth, &mut view, &mut work).await?;
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
    work.ethereum_authentication = Some(view.checkpoint().clone());
    while next <= work.intent.boundary.execution_number {
        let canonical = eth.get_block(next).await?;
        let block = eth
            .get_block_by_hash(canonical.header.hash)
            .full()
            .await?
            .context("HOLD: complete canonical transaction inventory unavailable")?;
        ensure!(block.header.number == next
            && block.header.inner.hash_slow() == block.header.hash,
            "HOLD: coverage header identity changed");
        view.verify_blocks(&**eth, &std::collections::BTreeMap::from([
            (next, block.header.hash),
        ])).await?;
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
            .map(alloy_primitives::keccak256)
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
            let emitter = seal::authenticated_fee_emitter(eth, &mut view,
                work.intent.erc20_manager, work.intent.old_identity.bridging_payment_address,
                receipt).await?;
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
                            None,
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
            ensure!(
                proof.transaction_index
                    == receipt
                        .transaction_index
                        .context("HOLD: receipt index missing")?
                    && proof.receipt_rlp
                        == alloy_rlp::encode(ethereum_common::utils::map_receipt_envelope(
                            receipt.as_ref()
                        ))
                    && proof
                        .proof_block
                        .block
                        .body
                        .execution_payload
                        .block_hash
                        .0
                         .0
                        == block.header.hash.0
                    && proof
                        .proof_block
                        .block
                        .body
                        .execution_payload
                        .receipts_root
                        .0
                         .0
                        == block.header.receipts_root.0,
                "HOLD: original proof does not bind canonical receipt/header"
            );
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
                exclusion: None,
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
        view.recheck(&**eth).await?;
        // All deposits and their covered block/cursor become durable together,
        // before any resolution can persist or act on an individual admission.
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
    // Also run when the boundary block was committed before a crash: there may
    // be no new blocks, but every saved original still needs reconciliation.
    for index in 0..work.admissions.len() {
        record_resolution(api, &readonly_sender, path, &mut work, index).await?;
    }
    let unpaid = work
        .admissions
        .iter()
        .filter(|admission| {
            admission.admission == "cutover-sponsored" && admission.exclusion.is_none()
        })
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
    let mut view = eth.verified_finalized_view().await?;
    authenticate_work_ethereum(eth, &mut view, &mut work).await?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let provider = ApiProviderConnection::from_api(signer.api().clone());
    let sender = sender(
        provider.clone(),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    let mut excluded_owned = BTreeSet::new();
    for index in 0..work.admissions.len() {
        if record_resolution(api, &sender, path, &mut work, index).await? {
            if let Some(uuid) = work.admissions[index].original_lane_uuid {
                excluded_owned.insert(uuid);
            }
        }
    }
    ensure!(
        sponsor
            || work
                .admissions
                .iter()
                .all(|admission| admission.fee_event_observed
                    || admission.external_original.is_some()
                    || admission.exclusion.is_some()),
        "HOLD: unpaid sponsorship requires explicit invocation authorization"
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
        if excluded_owned.contains(&transaction.uuid) {
            continue;
        }
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
        if sender
            .receipt_resolution_status(api, &proof, None)
            .await?
            .is_some()
        {
            let index = work
                .admissions
                .iter()
                .position(|admission| admission.original_tx_hash == transaction.tx.tx_hash)
                .context("HOLD: original terminal receipt is outside canonical coverage")?;
            ensure!(
                record_resolution(api, &sender, path, &mut work, index).await?,
                "HOLD: resolution changed"
            );
            if let Some(uuid) = work.admissions[index].original_lane_uuid {
                excluded_owned.insert(uuid);
            }
            continue;
        }
        let signed = match transaction
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
                view.recheck(&**eth).await?;
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
        view.recheck(&**eth).await?;
        let response = reconcile_original_response(
            api,
            &sender,
            &work.intent.old_identity,
            &transaction,
            &proof,
            signed,
        )
        .await;
        view.recheck(&**eth).await?;
        let done = matches!(response.status, MessageStatus::Success { .. });
        manager.finalize_transaction(response).await?;
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
            .all(|uuid| excluded_owned.contains(uuid)
                || work
                    .admissions
                    .iter()
                    .any(
                        |admission| admission.undispatched_old_admission == Some(*uuid)
                            && admission.external_original.is_some()
                            && seal::read_original_journal(&work.intent.old_transactions)
                                .ok()
                                .and_then(|journal| journal.transactions.get(uuid).cloned())
                                .is_some_and(|row| never_dispatched(&row)
                                    && row.tx.tx_hash == admission.original_tx_hash)
                    )),
        "HOLD: original lane contains failed admissions"
    );
    let mut external_cache: Option<Vec<(Vec<u8>, seal::CanonicalExternalOriginal)>> = None;
    for index in 0..work.admissions.len() {
        if record_resolution(api, &sender, path, &mut work, index).await? {
            continue;
        }
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
                            None,
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
                refresh_pending_external_original(external, fresh)?;
            }
            let result = qualify_external_original(
                api,
                &sender,
                &proof,
                external,
                admission.settlement.as_ref(),
                &work.intent.old_identity,
                admission.exclusion.as_ref(),
            )
            .await;
            match &result {
                Ok(QualifiedExternal::Settled(pin)) => {
                    admission.settlement = Some(pin.clone());
                    admission.admission = "externally-delivered".into();
                }
                Ok(QualifiedExternal::Excluded(pin)) => {
                    admission.exclusion = Some(pin.clone());
                    admission.admission = "externally-resolved".into();
                }
                Err(_) => {}
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
            admission.settlement = Some(original_pin.clone());
            save(path, &work)?;
            continue;
        }
        view.recheck(&**eth).await?;
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
        view.recheck(&**eth).await?;
        let result = reconcile_original(
            api,
            &sender,
            &proof,
            &mut signed,
            admission.settlement.as_ref(),
        )
        .await;
        view.recheck(&**eth).await?;
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
            .all(|transaction| excluded_owned.contains(&transaction.uuid)
                || (never_dispatched(transaction)
                    && work.admissions.iter().any(|admission| admission
                        .undispatched_old_admission
                        == Some(transaction.uuid)
                        && admission.external_original.is_some()
                        && (admission.settlement.is_some() ^ admission.exclusion.is_some())))),
        "HOLD: original lane remains undrained"
    );
    Ok(())
}

fn validate_source_emitter(
    identity: &InboundRuntimeIdentity,
    payment_manager: Option<H160>,
    source_manager: H160,
) -> Result<()> {
    let bound_manager = match (
        identity.erc20_manager_address,
        identity.bridging_payment_address,
        payment_manager,
    ) {
        (Some(manager), None, None) => manager,
        (None, Some(payment), Some(manager)) if !payment.is_zero() => manager,
        _ => bail!("HOLD: original journal has an invalid emitter routing identity"),
    };
    ensure!(
        !bound_manager.is_zero() && bound_manager == source_manager,
        "HOLD: original journal ERC20 emitter differs from source manager configuration"
    );
    Ok(())
}

async fn authenticate_source_emitter(
    eth: &PollingEthApi,
    identity: &InboundRuntimeIdentity,
    source_manager: H160,
) -> Result<()> {
    let view = eth.verified_finalized_view().await?;
    let payment_manager = if let Some(payment) = identity.bridging_payment_address {
        Some(seal::authenticated_payment_manager(eth, &view, payment,
            view.block_number(), view.block_hash()).await?)
    } else {
        None
    };
    validate_source_emitter(identity, payment_manager, source_manager)?;
    view.recheck(&**eth).await?;
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
    let eth = PollingEthApi::new(&args.ethereum_rpc).await?
        .with_finality_witness(&args.ethereum_finality.ethereum_witness_rpc,
            old_identity.ethereum_chain_id, old_identity.ethereum_genesis_hash.0.into()).await?;
    eth.enable_finality_archive(&args.cutover_dir.join("ethereum-finality"),
        old_identity.ethereum_genesis_hash.0.into()).await?;
    let mut view = eth.verified_finalized_view().await?;
    if let Some(predecessor) = seal::predecessor_seal_path(&old_transactions)? {
        let signer: gsdk::signer::Signer = api.clone().into();
        seal::verify_originals_read_only(&predecessor,
            ApiProviderConnection::from_api(signer.api().clone()), api, &eth, &mut view).await?;
    }
    let beacon = BeaconClient::new(args.beacon_rpc.clone(), Some(Duration::from_secs(30))).await?;
    let manager = VftManager::new(GClientRemoting::new(api.clone()));
    let at = api.last_block_hash().await?;
    let pin = sails_rs::H256::from_slice(at.as_ref());
    // Beacon is only a locator until the execution boundary and checkpoint proof agree.
    let selected = beacon.get_block::<ethereum_common::beacon::electra::Block>(
        args.boundary_slot).await?;
    ensure!(selected.slot == args.boundary_slot, "HOLD: selected boundary slot changed");
    view.verify_blocks(&*eth, &std::collections::BTreeMap::from([(
        selected.body.execution_payload.block_number,
        B256::from(selected.body.execution_payload.block_hash.0.0),
    )])).await?;
    let block = authenticate_cutover_boundary(
        &beacon,
        api,
        actor(old_identity.checkpoint_light_client_address),
        args.boundary_slot,
    )
    .await?;
    ensure!(block.tree_hash_root() == selected.tree_hash_root(),
        "HOLD: authenticated Beacon boundary changed after execution qualification");
    view.recheck(&*eth).await?;
    if path.exists() {
        upgrade_work(api, &eth, &path).await?;
    }
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
    authenticate_source_emitter(&eth, &old_identity, H160(erc20_manager.0)).await?;
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
        let roles = initial_role_pin(
            api,
            &path,
            hash_actor(*token),
            native_wrapper == Some(*token),
            H256::from_slice(at.as_ref()),
            "HOLD: original authority pin changed",
        )
        .await?;
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
        let roles = initial_role_pin(
            api,
            &path,
            wrapper,
            true,
            H256::from_slice(at.as_ref()),
            "HOLD: native authority pin changed",
        )
        .await?;
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
                schema_version: 3,
                ethereum_authentication: Some(view.checkpoint().clone()),
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
        run_cutover_call(api, &eth, &path, CutoverStep::BeginCutover, &intent).await?;
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
        drain_source(api, &eth, &path).await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let release = release_draining_hold(api, &eth, &path, &_signer_owner, &_other_signers).await;
        return match release {
            Ok(_) => Err(error),
            Err(release) => Err(error.context(format!("Signer ownership retained: {release:#}"))),
        };
    }
    release_owned_signers(api, &eth, &path, &_signer_owner, &_other_signers).await?;
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
async fn drain_source(api: &GearApi, eth: &PollingEthApi, path: &Path) -> Result<()> {
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
                    run_cutover_call(api, eth, path, CutoverStep::SourceRefund(key, ordinal), &work.intent).await?;
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
        let at = api.last_block_hash().await?;
        let roles = read_role_pin(api, wrapper, true, Some(H256::from_slice(at.as_ref()))).await?;
        ensure!(
            zero_blocking(&roles, work.schema_version)?,
            "HOLD: original native payouts remain blocking"
        );
        ensure!(
            !roles.paused,
            "HOLD: resume wrapper after retaining holds and before source custody transfer"
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
        .map(alloy_primitives::keccak256)
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
    let mut view = eth.verified_finalized_view().await?;
    view.verify_checkpoint(&**eth, &receipt.checkpoint).await?;
    view.verify_blocks(&**eth, &std::collections::BTreeMap::from([
        (registration.included_number, registration.included_hash),
        (registration.finalized_number, registration.finalized_hash),
    ])).await?;
    receipt.inclusion.verify_inclusion(&view, hash)?;
    verify_registrations_at(eth, intent, &view,
        registration.finalized_number, registration.finalized_hash).await?;
    Ok(registration)
}
async fn verify_registrations_at(
    eth: &PollingEthApi,
    intent: &Intent,
    view: &ethereum_client::FinalizedView,
    number: u64,
    hash: B256,
) -> Result<()> {
    seal::authenticate_manager_registrations(eth, view, intent.erc20_manager,
        [intent.old_identity.vft_manager_address, intent.new_identity.vft_manager_address],
        number, hash).await
}

async fn verify_current_registrations(eth: &PollingEthApi, intent: &Intent) -> Result<()> {
    let view = eth.verified_finalized_view().await?;
    verify_registrations_at(eth, intent, &view, view.block_number(), view.block_hash()).await
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
                    && zero_blocking(roles, work.schema_version)?
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
async fn reconcile_retained_role_calls(api: &GearApi, path: &Path) -> Result<()> {
    let mut work = read_work(path)?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let scanner = sender(
        ApiProviderConnection::from_api(signer.api().clone()),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    for index in 0..work.calls.len() {
        if !matches!(
            work.calls[index].step,
            CutoverStep::TokenMinter(_)
                | CutoverStep::TokenBurner(_)
                | CutoverStep::WrapperPause
                | CutoverStep::WrapperConfigureManager
                | CutoverStep::WrapperSetBurner
                | CutoverStep::WrapperResume
        ) {
            continue;
        }
        let call = &mut work.calls[index];
        validate_fixed_intent(&work.intent, call)?;
        let result = reconcile_call(api, &scanner, call, false).await;
        let reply = if complete(&call.original) {
            validate_call_reply(call)
        } else {
            Ok(())
        };
        save(path, &work)?;
        result?;
        reply?;
    }
    Ok(())
}

async fn handoff_after_reconciliation<R, F, H>(reconciliation: R, handoff: F) -> Result<()>
where
    R: std::future::Future<Output = Result<()>>,
    F: FnOnce() -> H,
    H: std::future::Future<Output = Result<()>>,
{
    reconciliation.await?;
    handoff().await
}

async fn handoff_roles(api: &GearApi, eth: &PollingEthApi, path: &Path) -> Result<()> {
    // Delay all role readback/guards until retained originals are persisted.
    handoff_after_reconciliation(reconcile_retained_role_calls(api, path), || {
        handoff_roles_reconciled(api, eth, path)
    })
    .await
}

async fn handoff_roles_reconciled(api: &GearApi, eth: &PollingEthApi, path: &Path) -> Result<()> {
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
        for (role, initial, step) in [
            (
                minter,
                actor(original.minter),
                CutoverStep::TokenMinter(*token),
            ),
            (
                burner,
                actor(original.burner),
                CutoverStep::TokenBurner(*token),
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
            run_cutover_call(api, eth, path, step, &work.intent).await?;
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
            zero_blocking(
                &read_role_pin(api, wrapper, true, Some(H256::from_slice(at.as_ref()))).await?,
                work.schema_version
            )?,
            "HOLD: blocking original native redemption"
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
            run_cutover_call(api, eth, path, CutoverStep::TokenMinter(wrapper), &work.intent).await?;
        } else {
            ensure!(
                minter == actor(initial.minter),
                "HOLD: preserved native minter changed"
            );
        }
        for step in [
            CutoverStep::WrapperPause,
            CutoverStep::WrapperConfigureManager,
            CutoverStep::WrapperSetBurner,
        ] {
            run_cutover_call(api, eth, path, step, &work.intent).await?;
        }
        let at = api.last_block_hash().await?;
        let pin = sails_rs::H256::from_slice(at.as_ref());
        ensure!(
            escrow.manager().at_block(pin).recv(actor(wrapper)).await? == Some(new)
                && admin.burner().at_block(pin).recv(actor(wrapper)).await? == new
                && zero_blocking(
                    &read_role_pin(api, wrapper, true, Some(H256::from_slice(at.as_ref()))).await?,
                    work.schema_version
                )?,
            "HOLD: wrapper handoff readback differs"
        );
        run_cutover_call(api, eth, path, CutoverStep::WrapperResume, &work.intent).await?;
        record_role_pin(api, path, wrapper, true).await?;
    }
    Ok(())
}
fn validate_admission_uniqueness(admissions: &[Admission]) -> Result<()> {
    ensure!(
        admissions
            .iter()
            .map(|admission| admission.receipt_key)
            .collect::<BTreeSet<_>>()
            .len()
            == admissions.len(),
        "HOLD: duplicate coverage admission keys"
    );
    ensure!(
        admissions
            .iter()
            .map(|admission| admission.original_tx_hash)
            .collect::<BTreeSet<_>>()
            .len()
            == admissions.len(),
        "HOLD: duplicate admitted transaction identity"
    );
    let owned = admissions
        .iter()
        .filter_map(|admission| {
            admission
                .original_lane_uuid
                .or(admission.undispatched_old_admission)
        })
        .collect::<Vec<_>>();
    ensure!(
        owned.iter().collect::<BTreeSet<_>>().len() == owned.len(),
        "HOLD: duplicate ownership UUID"
    );
    ensure!(
        admissions
            .iter()
            .all(|admission| !(admission.original_lane_uuid.is_some()
                && admission.undispatched_old_admission.is_some())),
        "HOLD: conflicting owned/undispatched UUID"
    );
    Ok(())
}

fn completed_coverage(work: &Work) -> Result<()> {
    validate_work_version(work)?;
    validate_admission_uniqueness(&work.admissions)?;
    let mut expected = 0;
    let mut previous = None;
    let mut accounted = BTreeSet::new();
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
                            && proof.proof_block.block.slot == admission.receipt_key.0
                            && proof
                                .proof_block
                                .block
                                .body
                                .execution_payload
                                .block_hash
                                .0
                                 .0
                                == block.hash.0
                            && proof
                                .proof_block
                                .block
                                .body
                                .execution_payload
                                .receipts_root
                                .0
                                 .0
                                == block.receipts_root.0
                            && block
                                .transactions
                                .get(usize::try_from(proof.transaction_index)?)
                                == Some(&admission.original_tx_hash),
                        "HOLD: admission proof does not contain the original canonical receipt/key"
                    );
                    accounted.insert(admission.original_tx_hash);
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
        accounted.len() == work.admissions.len(),
        "HOLD: admission lies outside canonical covered deposits"
    );
    ensure!(
        work.admissions
            .iter()
            .all(
                |admission| (admission.settlement.is_some() ^ admission.exclusion.is_some())
                    && admission.receipt_key.0 <= work.intent.boundary.slot
                    && admission.receipt_key.0 >= work.intent.source_receipt_floor
                    && (admission.fee_event_observed
                        || admission.admission == "cutover-sponsored"
                        || (matches!(
                            admission.admission.as_str(),
                            "externally-delivered" | "externally-resolved"
                        ) && admission.external_original.is_some()))
            ),
        "HOLD: economic admissions undrained or policy changed"
    );
    for admission in &work.admissions {
        ensure!(
            !(admission.original_lane_uuid.is_some()
                && admission.undispatched_old_admission.is_some()),
            "HOLD: conflicting owned/undispatched UUID"
        );
        if admission.exclusion.is_some() {
            ensure!(
                matches!(work.schema_version, 2 | 3)
                    && admission.settlement.is_none()
                    && ((admission.external_original.is_some()
                        && admission.admission == "externally-resolved"
                        && admission.signed.is_none()
                        && admission.original_lane_uuid.is_none()
                        && admission.attempts.is_empty())
                        || (admission.external_original.is_none()
                            && admission.signed.is_some()
                            && admission.undispatched_old_admission.is_none()
                            && admission.admission
                                == if admission.fee_event_observed {
                                    "paid"
                                } else {
                                    "cutover-sponsored"
                                })),
                "HOLD: exclusion provenance conflicts"
            );
        }
    }
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
    let eth = PollingEthApi::new(&args.ethereum_rpc).await?
        .with_finality_witness(&args.ethereum_finality.ethereum_witness_rpc,
            work.intent.old_identity.ethereum_chain_id,
            work.intent.old_identity.ethereum_genesis_hash.0.into()).await?;
    eth.enable_finality_archive(&args.cutover_dir.join("ethereum-finality"),
        work.intent.old_identity.ethereum_genesis_hash.0.into()).await?;
    let mut view = eth.verified_finalized_view().await?;
    let beacon = BeaconClient::new(args.beacon_rpc.clone(), Some(Duration::from_secs(30))).await?;
    authenticate_environment(api, &eth, &work.intent).await?;
    authenticate_boundary(api, &beacon, &work.intent).await?;
    let signer: gsdk::signer::Signer = api.clone().into();
    let provider = ApiProviderConnection::from_api(signer.api().clone());
    if let Some(predecessor) = seal::predecessor_seal_path(&work.intent.old_transactions)? {
        seal::verify_originals_read_only(&predecessor, provider.clone(), api, &eth, &mut view).await?;
    }
    let seal_path = args.cutover_dir.join("cutover.json");
    if seal_path.exists() {
        let (existing, _) = seal::verify_seal_files(&seal_path)?;
        ensure!(
            existing.old_identity == work.intent.old_identity
                && existing.new_identity == work.intent.new_identity,
            "HOLD: seal/work identity changed"
        );
        seal::install_seal_markers(&seal_path)?;
        seal::verify_originals_read_only(&seal_path, provider.clone(), api, &eth, &mut view).await?;
        upgrade_work(api, &eth, &path).await?;
    } else {
        upgrade_work(api, &eth, &path).await?;
        scan_coverage(api, &eth, &beacon, &path, args.sponsor_unpaid).await?;
        drain(api, connection, &eth, &beacon, &path, args.sponsor_unpaid).await?;
        drain_discovery(&eth, &path).await?;
        drain_source(api, &eth, &path).await?;
        work = read_work(&path)?;
        completed_coverage(&work)?;
        let old = actor(work.intent.old_identity.vft_manager_address);
        let new = actor(work.intent.new_identity.vft_manager_address);
        let (source, _) = status(api, hash_actor(old)).await?;
        if source.phase == CutoverPhase::Draining {
            run_cutover_call(api, &eth, &path, CutoverStep::SourcePause, &work.intent).await?;
            run_cutover_call(api, &eth, &path, CutoverStep::PrepareCutover, &work.intent).await?;
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
            run_cutover_call(api, &eth, &path, CutoverStep::Upgrade(ordinal), &work.intent).await?;
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
        handoff_roles(api, &eth, &path).await?;
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
                let mut registration_view = eth.verified_finalized_view().await?;
                authenticate_work_ethereum(&eth, &mut registration_view, &mut work).await?;
                work.ethereum_authentication = Some(registration_view.checkpoint().clone());
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
        let mut registration_view = eth.verified_finalized_view().await?;
        authenticate_work_ethereum(&eth, &mut registration_view, &mut work).await?;
        work.ethereum_authentication = Some(registration_view.checkpoint().clone());
        save(&path, &work)?;
        run_cutover_request(
            api,
            &eth,
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
                schema_version: 3,
                ethereum_authentication: work.ethereum_authentication.clone(),
                intent: work.intent.clone(),
                blocks: work.blocks.clone(),
                admissions: work.admissions.clone(),
            },
            true,
        )?;
        atomic_json(
            &calls_path,
            &LifecycleReport {
                schema_version: 3,
                intent: work.intent.clone(),
                calls: work.calls.clone(),
                role_pins: work.role_pins.clone(),
                registration: work.registration.clone().context("Missing registration")?,
            },
            true,
        )?;
        let sealed = CutoverSeal {
            schema_version: 3,
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
    let mut view = eth.verified_finalized_view().await?;
    seal::verify_originals_read_only(&seal_path, provider, api, &eth, &mut view).await?;
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
        seal::verify_originals_read_only(&seal_path, activation_scanner.api_provider.clone(),
            api, &eth, &mut view)
            .await?;
        validate_successor_journals(
            &work.intent.new_identity,
            &sealed.predecessor(seal::load_seal(&seal_path)?.1),
            work.intent.boundary.execution_hash,
            &transactions,
            &discovery,
        )?;
        verify_current_registrations(&eth, &work.intent).await?;
        release_owned_signers(api, &eth, &path, &_signer, &_other_signers).await?;
        println!("Cutover already activated; verified original activation without resetting successor journals.");
        return Ok(());
    }
    let recovered_activation =
        reconcile_retained_activation(api, &activation_scanner, &path).await?;
    verify_current_registrations(&eth, &work.intent).await?;
    if recovered_activation {
        let (sealed, digest) = seal::verify_markers(&seal_path)?;
        validate_successor_journals(
            &work.intent.new_identity,
            &sealed.predecessor(digest),
            work.intent.boundary.execution_hash,
            &transactions,
            &discovery,
        )?;
    } else {
        seal::initialize_successor(&seal_path, &transactions, &discovery).await?;
    }
    seal::authenticate_startup(
        &work.intent.new_identity,
        Some(&seal_path),
        Some(work.intent.new_manager_code_id),
        api,
        &eth,
    )
    .await?;
    if !recovered_activation {
        run_cutover_call(api, &eth, &path, CutoverStep::DestinationUnpause, &work.intent).await?;
    }
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
    release_owned_signers(api, &eth, &path, &_signer, &_other_signers).await?;
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
    let eth = PollingEthApi::new(&args.ethereum_rpc).await?
        .with_finality_witness(&args.ethereum_finality.ethereum_witness_rpc,
            sealed.old_identity.ethereum_chain_id,
            sealed.old_identity.ethereum_genesis_hash.0.into()).await?;
    // Deliberately no archive attachment: verification never owns or advances durable history.
    let mut view = eth.verified_finalized_view().await?;
    let beacon = BeaconClient::new(args.beacon_rpc.clone(), Some(Duration::from_secs(30))).await?;
    let report: CoverageReport = read_versioned_json(&sealed.coverage.path)?;
    let lifecycle: LifecycleReport = read_versioned_json(&sealed.lifecycle_evidence.path)?;
    ensure!(
        report.schema_version == sealed.schema_version
            && lifecycle.schema_version == sealed.schema_version
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
    seal::verify_originals_read_only(&args.seal, provider.clone(), &api, &eth, &mut view).await?;
    let mut work = Work {
        schema_version: sealed.schema_version,
        ethereum_authentication: report.ethereum_authentication,
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
    if work.blocks.is_empty() && matches!(work.schema_version, 1 | 2) {
        work.blocks = seal::reconstruct_coverage(&eth, &mut view, &work.intent.boundary,
            work.intent.old_identity.ethereum_genesis_hash).await?;
    }
    authenticate_work_ethereum(&eth, &mut view, &mut work).await?;
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
        let mut observed = read_role_pin_inner(
            &api,
            pin.token,
            work.intent.native_wrapper == Some(pin.token),
            Some(pin.block_hash),
            allow_historical_drained_inference(pin, work.schema_version),
        )
        .await?;
        if work.schema_version == 1 {
            observed.blocking_redemptions = None;
        }
        ensure!(
            observed == *pin,
            "HOLD: historical role state changed or unavailable"
        );
    }
    for block in &work.blocks {
        ensure!(
            eth.is_finalized_block(block.number, block.hash).await?,
            "HOLD: sealed canonical coverage changed"
        );
    }
    let scanner = sender(
        provider,
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
    );
    for original in &work.calls {
        verify_lifecycle_original(&api, &scanner, &work.intent, original).await?;
    }
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
        if admission.exclusion.is_some() {
            authenticate_exclusion(&api, &scanner, &work, admission).await?;
            continue;
        }
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
                &work.intent.old_identity,
                admission.exclusion.as_ref(),
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

    fn cutover_identity() -> InboundRuntimeIdentity {
        InboundRuntimeIdentity {
            ethereum_chain_id: 1,
            ethereum_genesis_hash: H256::repeat_byte(1),
            ethereum_start_block: 101,
            erc20_manager_address: Some(H160::repeat_byte(2)),
            bridging_payment_address: None,
            gear_genesis_hash: H256::repeat_byte(3),
            vft_manager_address: H256::repeat_byte(4),
            checkpoint_light_client_address: H256::repeat_byte(5),
            historical_proxy_address: H256::repeat_byte(6),
            gear_sender: [7; 32],
        }
    }

    #[test]
    fn preparation_rejects_rotated_emitter_for_direct_and_paid_lanes() {
        let mut identity = cutover_identity();
        let bound = identity.erc20_manager_address.unwrap();
        let rotated = H160::repeat_byte(9);
        assert!(validate_source_emitter(&identity, None, bound).is_ok());
        assert!(validate_source_emitter(&identity, None, rotated).is_err());
        identity.erc20_manager_address = None;
        identity.bridging_payment_address = Some(H160::repeat_byte(8));
        // Paid discovery binds the payment contract, not the actual receipt emitter.
        assert!(validate_source_emitter(&identity, Some(bound), bound).is_ok());
        assert!(validate_source_emitter(&identity, Some(bound), rotated).is_err());
        assert!(validate_source_emitter(&identity, Some(rotated), bound).is_err());
        assert!(validate_source_emitter(&identity, None, bound).is_err());
        identity.erc20_manager_address = Some(bound);
        assert!(validate_source_emitter(&identity, Some(bound), bound).is_err());
    }

    #[test]
    fn ambiguous_activation_holds_until_original_reply_is_reconciled() {
        let mut call = FinalizedCutoverCall {
            step: CutoverStep::DestinationUnpause,
            attempt: 0,
            target: H256::repeat_byte(4),
            payload: io::Unpause::encode_call(),
            gas: 1,
            value: 0,
            original: original_with_reply(None),
            archived_no_effect: false,
        };
        let original = serde_json::to_vec(&call).unwrap();
        assert!(activation_recovered(&mut call).is_err());
        assert_eq!(serde_json::to_vec(&call).unwrap(), original);
        call.original.finalized_reply = original_with_reply(Some(vec![])).finalized_reply;
        assert!(activation_recovered(&mut call).unwrap());
        let saved = call.original.clone();
        let mut recovered: FinalizedCutoverCall =
            serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        assert!(activation_recovered(&mut recovered).unwrap());
        assert!(recovered.original.same_request(&saved));
        recovered.original.finalized_reply_error = Some("failed".into());
        assert!(activation_recovered(&mut recovered).is_err());
        call.original.finalized_reply = None;
        call.original.message_id = None;
        call.original.finalized_dispatch_error = Some("definite pre-enqueue rejection".into());
        assert!(!activation_recovered(&mut call).unwrap());
        assert!(call.archived_no_effect);
    }

    #[test]
    fn missing_activation_marker_preserves_populated_bound_successor_journals() {
        use relayer::message_relayer::{
            common::{
                ethereum::block_storage::{BlockCursor, DiscoveryState},
                EthereumBlockNumber, EthereumSlotNumber, TxHashWithSlot,
            },
            eth_to_gear::{storage::Block, tx_manager::TxStatus},
        };
        let directory =
            std::env::temp_dir().join(format!("cutover-activation-recovery-{}", Uuid::now_v7()));
        let transactions = directory.join("transactions");
        fs::create_dir_all(&transactions).unwrap();
        let discovery = directory.join("discovery.json");
        let identity = cutover_identity();
        let mut old_identity = identity.clone();
        old_identity.vft_manager_address = H256::repeat_byte(10);
        let predecessor = seal::PredecessorCutover {
            manifest_digest: H256::repeat_byte(11),
            predecessor_identity: old_identity,
        };
        let boundary_hash = H256::repeat_byte(12);
        let tx = Transaction::new(
            TxHashWithSlot {
                tx_hash: B256::repeat_byte(13),
                slot_number: EthereumSlotNumber(33),
            },
            TxStatus::ComposeProof,
        );
        let journal = seal::OriginalJournal {
            schema_version: 7,
            runtime_identity: Some(identity.clone()),
            predecessor_cutover: Some(predecessor.clone()),
            transactions: [(tx.uuid, tx)].into(),
            completed: Default::default(),
            failed: Default::default(),
        };
        atomic_json(&transactions.join("state.json"), &journal, false).unwrap();
        let blocks: std::collections::BTreeMap<_, _> = [(
            EthereumSlotNumber(33),
            Block {
                number: EthereumBlockNumber(101),
                hash: Some(H256::repeat_byte(14)),
                transactions: [B256::repeat_byte(13)].into(),
            },
        )]
        .into();
        atomic_json(&transactions.join("blocks.json"), &blocks, false).unwrap();
        let state = DiscoveryState {
            schema_version: 2,
            identity: identity.clone(),
            predecessor_cutover: Some(predecessor.clone()),
            start_parent_hash: Some(boundary_hash),
            discovered: Some(BlockCursor {
                number: 102,
                hash: H256::repeat_byte(15),
            }),
            extracted: Some(BlockCursor {
                number: 101,
                hash: H256::repeat_byte(14),
            }),
            pending: [(102, H256::repeat_byte(15))].into(),
        };
        atomic_json(&discovery, &state, false).unwrap();
        let paths = [
            transactions.join("state.json"),
            transactions.join("blocks.json"),
            discovery.clone(),
        ];
        let before = paths.each_ref().map(|path| fs::read(path).unwrap());
        assert!(!directory.join("cutover-activation.json").exists());
        validate_successor_journals(
            &identity,
            &predecessor,
            boundary_hash,
            &transactions,
            &discovery,
        )
        .unwrap();
        assert_eq!(paths.each_ref().map(|path| fs::read(path).unwrap()), before);
        let mut changed = predecessor.clone();
        changed.manifest_digest = H256::repeat_byte(16);
        assert!(validate_successor_journals(
            &identity,
            &changed,
            boundary_hash,
            &transactions,
            &discovery,
        )
        .is_err());
        assert!(validate_successor_journals(
            &identity,
            &predecessor,
            H256::repeat_byte(17),
            &transactions,
            &discovery,
        )
        .is_err());
        fs::remove_dir_all(directory).unwrap();
    }

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
    fn lifecycle_unit_reply_is_exactly_empty_and_nonunit_is_routed() {
        assert!(exact_reply::<io::Pause>(&[]).is_ok());
        assert!(exact_reply::<io::BeginCutover>(&[]).is_ok());
        assert!(exact_reply::<vft_client::vft_admin::io::SetMinter>(&[]).is_ok());
        assert!(exact_reply::<io::Pause>(io::Pause::ROUTE).is_err());
        assert!(exact_reply::<io::Pause>(&[0]).is_err());
        assert!(exact_reply::<io::SubmitReceipt>(&[]).is_err());
        let mut reply = io::SubmitReceipt::ROUTE.to_vec();
        reply.extend(Result::<(), vft_manager_client::Error>::Ok(()).encode());
        assert!(matches!(
            exact_reply::<io::SubmitReceipt>(&reply),
            Ok(Ok(()))
        ));
        assert!(exact_reply::<io::ReconcileReceipt>(&reply).is_err());
        reply.push(0);
        assert!(exact_reply::<io::SubmitReceipt>(&reply).is_err());
    }

    fn receipt_frame() -> eth_events_electra_client::EthToVaraEvent {
        // A structural SCALE frame, not proof-verifier or economic evidence.
        let mut block = eth_events_electra_client::BlockGenericForBlockBody::decode(
            &mut [0u8; 2048].as_slice(),
        )
        .unwrap();
        block.slot = 32;
        eth_events_electra_client::EthToVaraEvent {
            proof_block: eth_events_electra_client::BlockInclusionProof {
                block,
                headers: vec![],
            },
            proof: vec![],
            transaction_index: 7,
            receipt_rlp: vec![1, 2, 3],
        }
    }

    fn original_with_reply(payload: Option<Vec<u8>>) -> SignedSubmission {
        use relayer::message_relayer::eth_to_gear::message_sender::FinalizedReplyEvidence;
        SignedSubmission {
            chain_genesis_hash: format!("{:#x}", H256::repeat_byte(1)),
            manager: format!("{:#x}", H256::repeat_byte(2)),
            historical_proxy: format!("{:#x}", H256::repeat_byte(3)),
            sender: "original-signer".into(),
            receipt_key: (32, 7),
            payload_hash: "original-payload".into(),
            nonce: 12,
            extrinsic_hash: "original-hash".into(),
            raw_extrinsic: vec![1, 2, 3, 4],
            prepared_finalized_number: 40,
            prepared_finalized_hash: format!("{:#x}", H256::repeat_byte(4)),
            scanned_finalized_number: 50,
            scanned_finalized_hash: format!("{:#x}", H256::repeat_byte(5)),
            reply_scanned_finalized_number: 52,
            reply_scanned_finalized_hash: format!("{:#x}", H256::repeat_byte(6)),
            inclusion_block_number: Some(50),
            inclusion_block_hash: Some(format!("{:#x}", H256::repeat_byte(5))),
            message_id: Some(format!("{:#x}", H256::repeat_byte(7))),
            finalized_dispatch_error: None,
            finalized_reply: payload.map(|payload| FinalizedReplyEvidence {
                payload,
                observation: FinalizedReceiptObservation {
                    finalized_block_number: Some(52),
                    finalized_block_hash: format!("{:#x}", H256::repeat_byte(6)),
                },
            }),
            native_reconciliation: None,
            native_reconciliation_attempts: vec![],
            finalized_nonce_consumed: None,
            finalized_reply_error: None,
        }
    }

    #[test]
    fn malformed_public_frames_do_not_hide_a_later_candidate() {
        use historical_proxy_client::historical_proxy::io::Redirect;
        let proxy = H256::repeat_byte(1);
        let receiver = H256::repeat_byte(2);
        let route = io::SubmitReceipt::ROUTE.to_vec();
        let event = receipt_frame();
        let valid = Redirect::encode_call(32, event.encode(), actor(receiver), route.clone());
        let empty_proof = Redirect::encode_call(32, vec![], actor(receiver), route.clone());
        let malformed_proof = Redirect::encode_call(32, vec![0], actor(receiver), route.clone());
        let wrong_slot = Redirect::encode_call(33, event.encode(), actor(receiver), route.clone());
        let wrong_receiver = Redirect::encode_call(32, event.encode(), actor(proxy), route.clone());
        let wrong_route = Redirect::encode_call(32, event.encode(), actor(receiver), vec![0]);
        let mut trailing = valid.clone();
        trailing.push(0);
        let mut proof_tail = event.encode();
        proof_tail.push(0);
        let trailing_proof = Redirect::encode_call(32, proof_tail, actor(receiver), route.clone());
        let malformed = [
            Redirect::ROUTE.to_vec(),
            empty_proof,
            malformed_proof,
            wrong_slot,
            trailing,
            trailing_proof,
            wrong_receiver,
            wrong_route,
        ];
        for payload in &malformed {
            assert!(external_receipt_candidate(proxy, payload, proxy, receiver, &route).is_none());
        }
        let candidates = malformed
            .iter()
            .chain(std::iter::once(&valid))
            .filter_map(|payload| {
                external_receipt_candidate(proxy, payload, proxy, receiver, &route)
            })
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].receipt_key, (32, 7));
        let reconcile = io::ReconcileReceipt::encode_call(32, 7);
        assert!(
            external_receipt_candidate(receiver, &reconcile, proxy, receiver, &route).is_some()
        );
        let mut tailed = reconcile.clone();
        tailed.push(0);
        for payload in [io::ReconcileReceipt::ROUTE.to_vec(), tailed] {
            assert!(
                external_receipt_candidate(receiver, &payload, proxy, receiver, &route).is_none()
            );
        }
        assert!(external_receipt_candidate(proxy, &reconcile, proxy, receiver, &route).is_none());
    }

    #[test]
    fn rejected_self_relay_never_owns_external_admission() {
        use historical_proxy_client::historical_proxy::io::Redirect;
        let proof = receipt_frame();
        for (result, accepted) in [
            (Ok(()), true),
            (
                Err(vft_manager_client::Error::NativeSettlementPending),
                true,
            ),
            (Err(vft_manager_client::Error::Paused), false),
        ] {
            let mut manager_reply = io::SubmitReceipt::ROUTE.to_vec();
            manager_reply.extend(result.encode());
            let nested: <Redirect as ActionIo>::Reply =
                Ok((proof.receipt_rlp.clone(), manager_reply));
            let mut payload = Redirect::ROUTE.to_vec();
            payload.extend(nested.encode());
            let original = original_with_reply(Some(payload));
            assert_eq!(
                accepted_external_parent(&proof.encode(), &original),
                accepted
            );
            let mut failed = original.clone();
            failed.finalized_reply_error = Some("Error".into());
            assert!(!accepted_external_parent(&proof.encode(), &failed));
            let mut undispatched = original.clone();
            undispatched.message_id = None;
            assert!(!accepted_external_parent(&proof.encode(), &undispatched));
            undispatched.message_id = original.message_id.clone();
            undispatched.finalized_dispatch_error = Some("DispatchError".into());
            assert!(!accepted_external_parent(&proof.encode(), &undispatched));
            let mut tailed = proof.encode();
            tailed.push(0);
            assert!(!accepted_external_parent(&tailed, &original));
        }
        assert!(!accepted_external_parent(
            &proof.encode(),
            &original_with_reply(None)
        ));
    }

    fn external_parent_fixture(pending_native: bool) -> (Vec<u8>, SignedSubmission) {
        use historical_proxy_client::historical_proxy::io::Redirect;
        let proof = receipt_frame();
        let mut receiver = io::SubmitReceipt::ROUTE.to_vec();
        receiver.extend(
            if pending_native {
                Result::<(), vft_manager_client::Error>::Err(
                    vft_manager_client::Error::NativeSettlementPending,
                )
            } else {
                Ok(())
            }
            .encode(),
        );
        let mut reply = Redirect::ROUTE.to_vec();
        let nested: <Redirect as ActionIo>::Reply = Ok((proof.receipt_rlp.clone(), receiver));
        reply.extend(nested.encode());
        (proof.encode(), original_with_reply(Some(reply)))
    }

    fn external_continuation_fixture(parent: &SignedSubmission, ordinal: u8) -> SignedSubmission {
        use subxt::config::{substrate::BlakeTwo256, Hasher};
        // Structural linkage fixtures only: no proof, runtime or economic evidence is fabricated.
        let mut continuation = parent.clone();
        continuation.nonce += u64::from(ordinal);
        continuation.prepared_finalized_number = 52;
        continuation.inclusion_block_number = Some(52 + u32::from(ordinal));
        let payload = io::ReconcileReceipt::encode_call(parent.receipt_key.0, parent.receipt_key.1);
        continuation.payload_hash = format!("{:#x}", alloy_primitives::keccak256(payload));
        continuation.raw_extrinsic = vec![ordinal];
        continuation.extrinsic_hash = format!(
            "{:#x}",
            H256::from(BlakeTwo256.hash(&continuation.raw_extrinsic).0)
        );
        let mut reply = io::ReconcileReceipt::ROUTE.to_vec();
        reply.extend(
            Result::<vft_manager_client::ReceiptStatus, vft_manager_client::Error>::Ok(
                vft_manager_client::ReceiptStatus::Processed,
            )
            .encode(),
        );
        let evidence = continuation.finalized_reply.as_mut().unwrap();
        evidence.payload = reply;
        evidence.observation.finalized_block_number = Some(54 + u32::from(ordinal));
        continuation
    }

    #[test]
    fn cutover_public_continuation_token_completion_ignores_redundant_call() {
        let (proof, mut parent) = external_parent_fixture(false);
        let before = parent.clone();
        let redundant = external_continuation_fixture(&parent, 1);
        attach_external_native_continuations(&proof, &mut parent, &[redundant]).unwrap();
        assert!(parent.same_request(&before));
        assert_eq!(parent.finalized_reply, before.finalized_reply);
        assert!(
            parent.native_reconciliation.is_none(),
            "token completion must not own a native continuation"
        );
        assert!(parent.native_reconciliation_attempts.is_empty());
    }

    #[test]
    fn cutover_public_continuation_native_completion_ignores_later_calls() {
        let (proof, parent) = external_parent_fixture(true);
        let delivered = external_continuation_fixture(&parent, 1);
        for failed in [false, true] {
            let mut redundant = external_continuation_fixture(&parent, 2);
            if failed {
                redundant.finalized_reply_error = Some("Error".into());
                redundant.finalized_reply.as_mut().unwrap().payload.clear();
            }
            let mut selected = parent.clone();
            attach_external_native_continuations(
                &proof,
                &mut selected,
                &[delivered.clone(), redundant],
            )
            .unwrap();
            assert!(selected.same_request(&parent));
            assert_eq!(selected.native_reconciliation.as_deref(), Some(&delivered),
                "later redundant/failed public calls must not replace the successful native original");
            assert!(selected.native_reconciliation_attempts.is_empty());
            let mut restored: SignedSubmission =
                serde_json::from_slice(&serde_json::to_vec(&selected).unwrap()).unwrap();
            let redundant = external_continuation_fixture(&parent, 3);
            attach_external_native_continuations(
                &proof,
                &mut restored,
                &[delivered.clone(), redundant],
            )
            .unwrap();
            assert_eq!(
                restored.native_reconciliation,
                selected.native_reconciliation
            );
            assert!(restored.preserves_reconciliation(&selected));
        }
    }

    #[test]
    fn cutover_public_continuation_preserves_earlier_attempts_and_unknown_fences() {
        let (proof, parent) = external_parent_fixture(true);
        let mut failed = external_continuation_fixture(&parent, 1);
        failed.finalized_reply_error = Some("Error".into());
        failed.finalized_reply.as_mut().unwrap().payload.clear();
        let delivered = external_continuation_fixture(&parent, 2);
        let mut selected = parent.clone();
        attach_external_native_continuations(
            &proof,
            &mut selected,
            &[failed.clone(), delivered.clone()],
        )
        .unwrap();
        assert_eq!(
            selected.native_reconciliation_attempts,
            vec![failed.clone()]
        );
        assert_eq!(selected.native_reconciliation.as_deref(), Some(&delivered));
        failed.finalized_reply = None;
        failed.finalized_reply_error = None;
        let mut held = parent;
        assert!(
            attach_external_native_continuations(&proof, &mut held, &[failed, delivered],).is_err()
        );
    }

    #[test]
    fn cutover_public_continuation_refreshes_only_pending_external_projection() {
        for pending_native in [false, true] {
            let (proof, parent) = external_parent_fixture(pending_native);
            let delivered = external_continuation_fixture(&parent, 1);
            let mut redundant = external_continuation_fixture(&parent, 2);
            redundant.finalized_reply_error = Some("Error".into());
            redundant.finalized_reply.as_mut().unwrap().payload.clear();
            let mut old = seal::CanonicalExternalOriginal {
                provenance: "canonical-external".into(),
                original: parent.clone(),
                discovered_at: parent.finalized_reply.as_ref().unwrap().observation.clone(),
            };
            if pending_native {
                old.original
                    .native_reconciliation_attempts
                    .push(delivered.clone());
            }
            old.original.native_reconciliation = Some(Box::new(redundant.clone()));
            let owned = old.original.clone();
            let mut fresh = seal::CanonicalExternalOriginal {
                provenance: old.provenance.clone(),
                original: parent,
                discovered_at: old.discovered_at.clone(),
            };
            attach_external_native_continuations(
                &proof,
                &mut fresh.original,
                &[delivered, redundant],
            )
            .unwrap();
            assert!(
                !fresh.original.preserves_reconciliation(&owned),
                "owned signatures must retain their existing strict preservation fence"
            );
            let mut restored: seal::CanonicalExternalOriginal =
                serde_json::from_slice(&serde_json::to_vec(&old).unwrap()).unwrap();
            refresh_pending_external_original(&mut restored, &fresh).unwrap();
            assert!(restored.original.same_request(&owned));
            assert_eq!(restored.original.finalized_reply, owned.finalized_reply);
            assert_eq!(restored.discovered_at, old.discovered_at);
            assert_eq!(
                restored.original.native_reconciliation,
                fresh.original.native_reconciliation
            );
            assert_eq!(
                restored.original.native_reconciliation_attempts,
                fresh.original.native_reconciliation_attempts
            );
            for field in [
                "raw",
                "nonce",
                "inclusion",
                "message",
                "dispatch",
                "reply",
                "code",
                "disposition",
                "provenance",
            ] {
                let mut changed = fresh.clone();
                match field {
                    "raw" => changed.original.raw_extrinsic.push(0),
                    "nonce" => changed.original.nonce += 1,
                    "inclusion" => changed.original.inclusion_block_hash = None,
                    "message" => changed.original.message_id = None,
                    "dispatch" => {
                        changed.original.finalized_dispatch_error = Some("changed".into())
                    }
                    "reply" => changed
                        .original
                        .finalized_reply
                        .as_mut()
                        .unwrap()
                        .payload
                        .push(0),
                    "code" => changed.original.finalized_reply_error = Some("changed".into()),
                    "disposition" => changed.original.finalized_nonce_consumed = Some(100),
                    "provenance" => changed.provenance = "own".into(),
                    _ => unreachable!(),
                }
                assert!(
                    refresh_pending_external_original(&mut restored, &changed).is_err(),
                    "{field}"
                );
            }
        }
    }

    #[tokio::test]
    async fn first_role_guard_waits_for_durable_original_reconciliation() {
        let directory = std::env::temp_dir().join(format!("cutover-role-order-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("call.json");
        let token = H256::repeat_byte(8);
        let new = actor(H256::repeat_byte(9));
        let call = FinalizedCutoverCall {
            step: CutoverStep::TokenMinter(token),
            attempt: 0,
            target: token,
            payload: vft_client::vft_admin::io::SetMinter::encode_call(new),
            gas: 1,
            value: 0,
            original: original_with_reply(None),
            archived_no_effect: false,
        };
        atomic_json(&path, &call, false).unwrap();
        let before = call.original.clone();
        let readback_started = std::cell::Cell::new(false);
        handoff_after_reconciliation(
            async {
                // Yield so polling the role callback too early deterministically fails its guard.
                tokio::task::yield_now().await;
                let mut saved: FinalizedCutoverCall = serde_json::from_slice(&fs::read(&path)?)?;
                ensure!(
                    !readback_started.get(),
                    "role readback preceded original recovery"
                );
                saved.original.finalized_reply = original_with_reply(Some(vec![])).finalized_reply;
                validate_call_reply(&saved)?;
                atomic_json(&path, &saved, false)
            },
            || async {
                readback_started.set(true);
                // The actual role is already the successor, but completion needs the saved original.
                let live_role = new;
                let saved: FinalizedCutoverCall = serde_json::from_slice(&fs::read(&path)?)?;
                ensure!(
                    live_role == new && complete(&saved.original),
                    "unjournaled role handoff"
                );
                ensure!(
                    saved.original.same_request(&before),
                    "original identity replaced"
                );
                validate_call_reply(&saved)
            },
        )
        .await
        .unwrap();
        assert!(readback_started.get());
        readback_started.set(false);
        let held =
            handoff_after_reconciliation(async { bail!("HOLD: original unresolved") }, || async {
                readback_started.set(true);
                Ok(())
            })
            .await;
        assert!(held.is_err());
        assert!(
            !readback_started.get(),
            "failed reconciliation must not query roles or sign"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    #[ignore = "read-only owned native stack and two loopback archives required"]
    async fn owned_native_first_minter_original_replays_before_role_guard() -> Result<()> {
        use vft_client::traits::VftAdmin as _;
        let stack_path = std::env::var("CUTOVER_NATIVE_STACK")?;
        let source = std::env::var("CUTOVER_NATIVE_SOURCE")?;
        let witness = std::env::var("CUTOVER_NATIVE_WITNESS")?;
        for rpc in [&source, &witness] {
            ensure!(
                rpc.starts_with("ws://127.0.0.1:") && rpc[15..].bytes().all(|b| b.is_ascii_digit()),
                "only owned loopback archives may run this no-signature replay"
            );
        }
        let stack: serde_json::Value = serde_json::from_slice(&fs::read(stack_path)?)?;
        let message = stack["configuration"]["actions"]
            .as_object()
            .context("actions missing")?
            .iter()
            .filter(|(key, _)| key.ends_with(".minter"))
            .map(|(_, action)| &action["messages"][0])
            .min_by_key(|message| message["target"].as_str().unwrap_or_default())
            .context("retained first token SetMinter missing")?;
        let submission = &message["submission"];
        let string = |value: &serde_json::Value| -> Result<String> {
            Ok(value
                .as_str()
                .context("missing retained string")?
                .to_owned())
        };
        let target = H256::from_str(&string(&message["target"])?)?;
        let payload = hex::decode(string(&message["payload"])?.trim_start_matches("0x"))?;
        let mut params = payload
            .strip_prefix(vft_client::vft_admin::io::SetMinter::ROUTE)
            .context("not the original SetMinter route")?;
        let successor = ActorId::decode(&mut params)?;
        ensure!(params.is_empty(), "SetMinter trailing bytes");
        let prepared = u32::try_from(
            submission["scanFromNumber"]
                .as_u64()
                .context("scan pin missing")?,
        )?;
        let pin = string(&submission["scanFromHash"])?;
        // Setup journals encode public account bytes as hex; SignedSubmission uses SS58.
        let sender = subxt::utils::AccountId32(H256::from_str(&string(&submission["sender"])?)?.0);
        let mut call = FinalizedCutoverCall {
            step: CutoverStep::TokenMinter(target),
            attempt: 0,
            target,
            payload: payload.clone(),
            gas: message["gasLimit"].as_u64().context("gas missing")?,
            value: 0,
            archived_no_effect: false,
            original: SignedSubmission {
                chain_genesis_hash: string(&stack["sourceGenesis"])?,
                manager: format!("{target:#x}"),
                historical_proxy: format!("{target:#x}"),
                sender: sender.to_string(),
                receipt_key: (0, 0),
                payload_hash: format!("{:#x}", alloy_primitives::keccak256(&payload)),
                nonce: submission["nonce"].as_u64().context("nonce missing")?,
                extrinsic_hash: string(&submission["extrinsicHash"])?,
                raw_extrinsic: hex::decode(
                    string(&submission["rawExtrinsic"])?.trim_start_matches("0x"),
                )?,
                prepared_finalized_number: prepared,
                prepared_finalized_hash: pin.clone(),
                scanned_finalized_number: prepared,
                scanned_finalized_hash: pin.clone(),
                reply_scanned_finalized_number: prepared,
                reply_scanned_finalized_hash: pin,
                inclusion_block_number: Some(u32::try_from(
                    message["inclusionEvidence"]["blockNumber"]
                        .as_u64()
                        .context("inclusion missing")?,
                )?),
                inclusion_block_hash: Some(string(&message["inclusionEvidence"]["blockHash"])?),
                message_id: None,
                finalized_dispatch_error: None,
                finalized_reply: None,
                native_reconciliation: None,
                native_reconciliation_attempts: vec![],
                finalized_nonce_consumed: None,
                finalized_reply_error: None,
            },
        };
        let before = call.original.clone();
        let directory =
            std::env::temp_dir().join(format!("cutover-native-readonly-{}", Uuid::now_v7()));
        fs::create_dir(&directory)?;
        let path = directory.join("first-role.json");
        atomic_json(&path, &call, false)?;
        // Both archives reauthenticate the same original; no call preparation, signing or broadcast.
        for rpc in [source, witness] {
            call.original = before.clone();
            let provider = ApiProvider::new(rpc, 0).await?;
            let connection = provider.connection();
            let mut readonly = connection.clone();
            let api = readonly.gclient();
            let scanner =
                MessageSender::new(target, vec![], target, connection, String::new(), None);
            handoff_after_reconciliation(
                async {
                    let result = reconcile_call(&api, &scanner, &mut call, false).await;
                    atomic_json(&path, &call, false)?;
                    result?;
                    validate_call_reply(&call)
                },
                || async {
                    let recovered: FinalizedCutoverCall =
                        serde_json::from_slice(&fs::read(&path)?)?;
                    ensure!(
                        recovered.original.same_request(&before) && complete(&recovered.original),
                        "retained original not recovered before role guard"
                    );
                    let reply = recovered
                        .original
                        .finalized_reply
                        .as_ref()
                        .context("reply absent")?;
                    ensure!(
                        reply.payload.is_empty(),
                        "native unit reply is not exactly empty"
                    );
                    ensure!(
                        reply.observation.finalized_block_hash
                            == string(&message["replyEvidence"]["blockHash"])?
                            && recovered.original.message_id.as_deref()
                                == message["messageId"].as_str(),
                        "original native reply linkage/pin changed"
                    );
                    let hash = H256::from_str(&reply.observation.finalized_block_hash)?;
                    let minter = vft_client::VftAdmin::new(GClientRemoting::new(api.clone()))
                        .minter()
                        .at_block(sails_rs::H256::from_slice(hash.as_bytes()))
                        .recv(actor(target))
                        .await?;
                    ensure!(minter == successor, "authenticated successor role differs");
                    println!(
                        "Read-only original {} nonce {} recovered before first-role guard at {}",
                        recovered.original.extrinsic_hash,
                        recovered.original.nonce,
                        reply.observation.finalized_block_hash
                    );
                    Ok(())
                },
            )
            .await?;
        }
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn original_empty_minter_reply_survives_restart_without_replacement() {
        let token = H256::repeat_byte(8);
        let new = actor(H256::repeat_byte(9));
        let mut call = FinalizedCutoverCall {
            step: CutoverStep::TokenMinter(token),
            attempt: 0,
            target: token,
            payload: vft_client::vft_admin::io::SetMinter::encode_call(new),
            gas: 1,
            value: 0,
            original: original_with_reply(None),
            archived_no_effect: false,
        };
        let saved = call.original.clone();
        assert!(!complete(&call.original));
        assert!(validate_call_reply(&call).is_err());
        // Reconciliation supplies the original automatic success, not a role readback.
        call.original.finalized_reply = original_with_reply(Some(vec![])).finalized_reply;
        let recovered: FinalizedCutoverCall =
            serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        assert!(recovered.original.same_request(&saved));
        assert!(complete(&recovered.original));
        validate_call_reply(&recovered).unwrap();
        call.original.finalized_reply_error = Some("Error".into());
        assert!(!complete(&call.original));
    }
    async fn terminal_coverage_fixture() -> Work {
        use alloy::{
            consensus::{
                proofs::{calculate_receipt_root, calculate_transaction_root},
                Receipt, ReceiptEnvelope, SignableTransaction, TxLegacy,
            },
            network::TxSigner,
            primitives::{Address, Log, TxKind, U256},
            signers::local::PrivateKeySigner,
        };
        let identity = cutover_identity();
        let emitter = identity.erc20_manager_address.unwrap();
        let event = IERC20Manager::BridgingRequested {
            token: Address::from([9; 20]),
            from: Address::from([10; 20]),
            to: B256::ZERO,
            amount: U256::from(1),
        };
        let log = Log {
            address: Address::from(emitter.0),
            data: event.encode_log_data(),
        };
        let envelope = ReceiptEnvelope::Legacy(
            Receipt {
                status: true.into(),
                cumulative_gas_used: 21_000,
                logs: vec![log.clone()],
            }
            .with_bloom(),
        );
        let signer = PrivateKeySigner::from_bytes(&B256::from([17; 32])).unwrap();
        let mut transaction = TxLegacy {
            chain_id: Some(1),
            gas_price: 1,
            gas_limit: 21_000,
            to: TxKind::Call(log.address),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut transaction).await.unwrap();
        let transaction = TxEnvelope::Legacy(transaction.into_signed(signature));
        let raw = transaction.encoded_2718();
        let tx_hash = alloy_primitives::keccak256(&raw);
        let header = alloy::consensus::Header {
            transactions_root: calculate_transaction_root(&[transaction]),
            receipts_root: calculate_receipt_root(std::slice::from_ref(&envelope)),
            gas_used: 21_000,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let block_hash = header.hash_slow();
        let mut receipt = serde_json::to_value(&envelope).unwrap();
        let fields = serde_json::json!({
            "transactionHash": tx_hash, "transactionIndex": "0x0", "blockHash": block_hash,
            "blockNumber": "0x0", "gasUsed": "0x5208", "effectiveGasPrice": "0x1",
            "from": signer.address(), "to": log.address, "contractAddress": null,
            "logs": [{ "address": log.address, "topics": log.data.topics(), "data": log.data.data,
                "transactionHash": tx_hash, "transactionIndex": "0x0", "blockHash": block_hash,
                "blockNumber": "0x0", "logIndex": "0x0", "removed": false }]
        });
        receipt
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let receipt: TransactionReceipt = serde_json::from_value(receipt).unwrap();
        let covered = CoveredBlock {
            number: 0,
            hash: H256(block_hash.0),
            parent_hash: H256(header.parent_hash.0),
            receipts_root: H256(header.receipts_root.0),
            transactions: vec![tx_hash],
            receipts: vec![receipt.clone()],
            header,
            raw_transactions: vec![raw],
        };
        // Real canonical Ethereum encoding; the zero SCALE beacon frame is only
        // a coordinate fixture, never passed off as authenticated checkpoint evidence.
        let mut proof = receipt_frame();
        proof.transaction_index = 0;
        proof.receipt_rlp = alloy_rlp::encode(ethereum_common::utils::map_receipt_envelope(
            receipt.as_ref(),
        ));
        proof
            .proof_block
            .block
            .body
            .execution_payload
            .block_hash
            .0
             .0 = block_hash.0;
        proof
            .proof_block
            .block
            .body
            .execution_payload
            .receipts_root
            .0
             .0 = covered.receipts_root.0;
        let pin = FinalizedReceiptObservation {
            finalized_block_number: Some(52),
            finalized_block_hash: format!("{:#x}", H256::repeat_byte(6)),
        };
        let admission = Admission {
            receipt_key: (32, 0),
            original_tx_hash: tx_hash,
            original_block_hash: covered.hash,
            fee_emitter: None,
            admission: "cutover-sponsored".into(),
            fee_event_observed: false,
            proof: proof.encode(),
            signed: Some(original_with_reply(None)),
            attempts: vec![],
            settlement: None,
            exclusion: Some(pin),
            original_lane_uuid: None,
            external_original: None,
            undispatched_old_admission: None,
        };
        Work {
            schema_version: 2,
            ethereum_authentication: None,
            intent: Intent {
                old_identity: identity.clone(),
                new_identity: identity.clone(),
                new_manager_code_id: H256::repeat_byte(11),
                old_transactions: PathBuf::from("unused-originals"),
                old_discovery: PathBuf::from("unused-discovery"),
                source_receipt_floor: 0,
                boundary: CutoverBoundary {
                    slot: 32,
                    beacon_root: H256::zero(),
                    execution_number: 0,
                    execution_hash: covered.hash,
                },
                boundary_execution_timestamp: 0,
                config_fingerprint: H256::zero(),
                erc20_manager: emitter,
                native_wrapper: None,
                initial_role_pins: vec![],
                mappings: vec![],
            },
            blocks: vec![covered],
            admissions: vec![admission],
            calls: vec![],
            role_pins: vec![],
            registration: None,
            retirement: None,
            readiness: None,
            new_transactions: None,
            new_discovery: None,
        }
    }

    // Reproduce a committed admission whose block was not yet appended by scan_coverage.
    // Only transport responses are mocked: headers, signed transactions and receipt proofs
    // are checked by authenticate_work_ethereum exactly as they are on restart.
    async fn interrupted_coverage_fixture() -> (Work, Vec<CoveredBlock>) {
        let mut work = terminal_coverage_fixture().await;
        let mut deposit = work.blocks.pop().unwrap();
        use alloy::{
            consensus::{SignableTransaction, TxLegacy},
            network::TxSigner,
            primitives::TxKind,
            signers::local::PrivateKeySigner,
        };
        // The crash happened after the first of two deposits. Recovery must not
        // mark this whole block scanned and silently skip its second receipt.
        let signer = PrivateKeySigner::from_bytes(&B256::from([17; 32])).unwrap();
        let mut transaction = TxLegacy {
            chain_id: Some(1), nonce: 1, gas_price: 1, gas_limit: 21_000,
            to: TxKind::Call(work.intent.erc20_manager.0.into()),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut transaction).await.unwrap();
        let transaction = TxEnvelope::Legacy(transaction.into_signed(signature));
        let raw = transaction.encoded_2718();
        let hash = alloy_primitives::keccak256(&raw);
        let mut receipt = serde_json::to_value(&deposit.receipts[0]).unwrap();
        receipt["transactionHash"] = serde_json::json!(hash);
        receipt["transactionIndex"] = serde_json::json!("0x1");
        receipt["cumulativeGasUsed"] = serde_json::json!("0xa410");
        for log in receipt["logs"].as_array_mut().unwrap() {
            log["transactionHash"] = serde_json::json!(hash);
            log["transactionIndex"] = serde_json::json!("0x1");
            log["logIndex"] = serde_json::json!("0x1");
        }
        deposit.transactions.push(hash);
        deposit.raw_transactions.push(raw);
        deposit.receipts.push(serde_json::from_value(receipt).unwrap());
        let transactions = deposit.raw_transactions.iter()
            .map(|raw| TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap()).collect::<Vec<_>>();
        let envelopes = deposit.receipts.iter()
            .map(|receipt| ethereum_common::utils::map_receipt_envelope(receipt.as_ref()))
            .collect::<Vec<_>>();
        deposit.header.transactions_root = alloy::consensus::proofs::calculate_transaction_root(&transactions);
        deposit.header.receipts_root = alloy::consensus::proofs::calculate_receipt_root(&envelopes);
        deposit.header.gas_used = 42_000;
        deposit.receipts_root = H256(deposit.header.receipts_root.0);
        let mut chain = Vec::new();
        let mut parent = B256::ZERO;
        for number in 0..=8 {
            let mut header = if number == 2 {
                deposit.header.clone()
            } else {
                alloy::consensus::Header {
                    transactions_root: alloy_primitives::keccak256([0x80]),
                    receipts_root: alloy_primitives::keccak256([0x80]),
                    ..Default::default()
                }
            };
            header.number = number;
            header.parent_hash = parent;
            header.timestamp = number * 12;
            if number == 0 {
                header.extra_data = Uuid::now_v7().as_bytes().to_vec().into();
            }
            let hash = header.hash_slow();
            let block = if number == 2 {
                deposit.number = number;
                deposit.hash = H256(hash.0);
                deposit.parent_hash = H256(parent.0);
                deposit.header = header;
                for saved in &mut deposit.receipts {
                    let mut receipt = serde_json::to_value(&*saved).unwrap();
                    receipt["blockNumber"] = serde_json::json!("0x2");
                    receipt["blockHash"] = serde_json::json!(hash);
                    for log in receipt["logs"].as_array_mut().unwrap() {
                        log["blockNumber"] = serde_json::json!("0x2");
                        log["blockHash"] = serde_json::json!(hash);
                    }
                    *saved = serde_json::from_value(receipt).unwrap();
                }
                deposit.clone()
            } else {
                CoveredBlock {
                    number, hash: H256(hash.0), parent_hash: H256(parent.0),
                    receipts_root: H256(header.receipts_root.0), header,
                    transactions: vec![], raw_transactions: vec![], receipts: vec![],
                }
            };
            validate_covered_block(&block).unwrap();
            chain.push(block);
            parent = hash;
        }
        let admission = &mut work.admissions[0];
        admission.original_block_hash = deposit.hash;
        admission.exclusion = None;
        let mut proof = eth_events_electra_client::EthToVaraEvent::decode(
            &mut admission.proof.as_slice(),
        ).unwrap();
        let payload = &mut proof.proof_block.block.body.execution_payload;
        payload.block_number = 2;
        payload.block_hash.0.0 = deposit.hash.0;
        payload.receipts_root.0.0 = deposit.receipts_root.0;
        payload.timestamp = 24;
        proof.proof = ethereum_common::utils::generate_merkle_proof(
            0, &envelopes.into_iter().enumerate().map(|(index, receipt)| (index as u64, receipt)).collect::<Vec<_>>(),
        ).unwrap().proof;
        admission.proof = proof.encode();
        work.intent.old_identity.ethereum_genesis_hash = chain[0].hash;
        work.intent.boundary.execution_number = 8;
        work.intent.boundary.execution_hash = chain[8].hash;
        work.intent.boundary.slot = 38;
        work.intent.boundary_execution_timestamp = 96;
        work.blocks = chain[..2].to_vec();
        (work, chain)
    }

    struct WorkEvidenceRpc {
        url: String,
        // Pruning bodies must not remove the canonical headers used by finality.
        bodies: Arc<std::sync::Mutex<BTreeSet<u64>>>,
        requests: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl Drop for WorkEvidenceRpc {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    impl WorkEvidenceRpc {
        async fn new(chain: Vec<CoveredBlock>) -> Self {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            use serde_json::{json, Value};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let bodies = Arc::new(std::sync::Mutex::new(chain.iter().map(|b| b.number).collect::<BTreeSet<_>>()));
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let available = bodies.clone();
            let observed = requests.clone();
            let chain = Arc::new(chain);
            let server = tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else { break };
                    let chain = chain.clone();
                    let available = available.clone();
                    let observed = observed.clone();
                    tokio::spawn(async move {
                        let mut buffer = Vec::new();
                        let request = loop {
                            let mut chunk = [0; 4096];
                            let count = socket.read(&mut chunk).await.unwrap();
                            if count == 0 { return; }
                            buffer.extend_from_slice(&chunk[..count]);
                            let Some(end) = buffer.windows(4).position(|b| b == b"\r\n\r\n") else { continue };
                            let headers = std::str::from_utf8(&buffer[..end]).unwrap();
                            let length: usize = headers.lines().find_map(|line| line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())).unwrap();
                            if buffer.len() >= end + 4 + length {
                                break serde_json::from_slice::<Value>(&buffer[end + 4..end + 4 + length]).unwrap();
                            }
                        };
                        observed.lock().unwrap().push(request.clone());
                        let params = &request["params"];
                        let result = match request["method"].as_str().unwrap() {
                            "eth_chainId" => json!("0x1"),
                            "eth_getBlockByNumber" | "eth_getBlockByHash" => {
                                let block = if request["method"] == "eth_getBlockByNumber" {
                                    let number = if params[0] == "finalized" { 8 }
                                    else { u64::from_str_radix(params[0].as_str().unwrap().trim_start_matches("0x"), 16).unwrap() };
                                    chain.iter().find(|b| b.number == number)
                                } else {
                                    chain.iter().find(|b| json!(b.hash) == params[0])
                                };
                                block.map_or(Value::Null, |block| {
                                    if params[1] == true && !available.lock().unwrap().contains(&block.number) {
                                        return Value::Null;
                                    }
                                    let mut header: alloy::rpc::types::Header = Default::default();
                                    header.inner = block.header.clone();
                                    header.hash = block.hash.0.into();
                                    let mut value = serde_json::to_value(header).unwrap();
                                    value["transactions"] = if params[1] == true {
                                        json!(block.raw_transactions.iter().enumerate().map(|(index, raw)| {
                                            let tx = TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap();
                                            let mut tx = serde_json::to_value(tx).unwrap();
                                            tx["blockHash"] = json!(block.hash);
                                            tx["blockNumber"] = json!(format!("0x{:x}", block.number));
                                            tx["transactionIndex"] = json!(format!("0x{index:x}"));
                                            tx["from"] = json!(block.receipts[index].from);
                                            tx
                                        }).collect::<Vec<_>>())
                                    } else { json!(block.transactions) };
                                    value["uncles"] = json!([]);
                                    value
                                })
                            }
                            "eth_getBlockReceipts" => {
                                let hash = params[0].get("blockHash").unwrap_or(&params[0]);
                                chain.iter().find(|b| json!(b.hash) == *hash)
                                    .filter(|b| available.lock().unwrap().contains(&b.number))
                                    .map_or(Value::Null, |b| json!(b.receipts))
                            }
                            method => panic!("unexpected work recovery RPC {method}"),
                        };
                        let response = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
                        let _ = socket.write_all(format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                            response.len(),
                        ).as_bytes()).await;
                    });
                }
            });
            Self { url, bodies, requests, server }
        }
    }

    async fn assert_interrupted_coverage_recovers_without_unneeded_bodies(version: u32, empty_inventory: bool) {
        let (mut work, chain) = interrupted_coverage_fixture().await;
        if empty_inventory {
            work.blocks.clear();
        }
        let primary = WorkEvidenceRpc::new(chain.clone()).await;
        let witness = WorkEvidenceRpc::new(chain.clone()).await;
        let eth = PollingEthApi::new(&primary.url).await.unwrap()
            .with_finality_witness(&witness.url, 1, chain[0].hash.0.into()).await.unwrap();
        let mut view = eth.verified_finalized_view().await.unwrap();
        work.schema_version = version;
        if version == 3 {
            work.ethereum_authentication = Some(view.checkpoint().clone());
        }
        let directory = std::env::temp_dir().join(format!("cutover-mid-block-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        let path = work_path(&directory);
        // This is exactly the durable state between the admission save and block save.
        save(&path, &work).unwrap();
        let original = fs::read(&path).unwrap();
        let mut restarted = read_work(&path).unwrap();
        authenticate_work_ethereum(&eth, &mut view, &mut restarted).await.unwrap();
        assert_eq!(serde_json::to_vec(&restarted).unwrap(), original);
        let mut falsely_advanced = restarted.clone();
        falsely_advanced.blocks = chain.clone();
        assert!(completed_coverage(&falsely_advanced).unwrap_err().to_string().contains("omitted a deposit"),
            "recovery must not certify the other deposit in a partially scanned block");

        let mut changed_original = restarted.clone();
        changed_original.admissions[0].original_tx_hash = B256::repeat_byte(99);
        assert!(authenticate_work_ethereum(&eth, &mut view, &mut changed_original).await.is_err(),
            "recovery must still bind the saved original to the authenticated receipt");

        // A missing original body still HOLDs; retained signatures/admissions are not replaced.
        primary.bodies.lock().unwrap().clear();
        assert!(authenticate_work_ethereum(&eth, &mut view, &mut restarted).await.is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(serde_json::to_vec(&restarted).unwrap(), original);
        primary.bodies.lock().unwrap().insert(2);
        primary.requests.lock().unwrap().clear();
        let result = authenticate_work_ethereum(&eth, &mut view, &mut restarted).await;
        let requests = primary.requests.lock().unwrap().clone();
        let body_hashes = requests.iter().filter_map(|request| {
            (request["method"] == "eth_getBlockByHash" && request["params"][1] == true)
                .then(|| request["params"][0].clone())
        }).collect::<Vec<_>>();
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(serde_json::to_vec(&restarted).unwrap(), original);
        fs::remove_dir_all(directory).unwrap();
        assert!(result.is_ok(),
            "a saved prefix plus its one in-progress admitted block must recover without old/future bodies: {result:?}; requests={body_hashes:?}");
        assert_eq!(body_hashes, vec![serde_json::json!(chain[2].hash)]);
        assert_eq!(restarted.blocks.len(), if empty_inventory { 0 } else { 2 },
            "authentication must not advance the scan past unrecorded sibling deposits");
    }

    #[tokio::test]
    async fn interrupted_v3_coverage_recovers_without_refetching_saved_prefix() {
        assert_interrupted_coverage_recovers_without_unneeded_bodies(3, false).await;
    }

    #[tokio::test]
    async fn interrupted_v2_coverage_recovers_without_fetching_future_boundary() {
        assert_interrupted_coverage_recovers_without_unneeded_bodies(2, false).await;
    }

    #[tokio::test]
    async fn interrupted_legacy_empty_coverage_recovers_without_certifying_inventory() {
        assert_interrupted_coverage_recovers_without_unneeded_bodies(2, true).await;
        // upgrade_work retains the empty scan cursor when adding its v3 checkpoint.
        assert_interrupted_coverage_recovers_without_unneeded_bodies(3, true).await;
    }

    #[tokio::test]
    async fn committed_coverage_block_retains_all_pending_originals_on_restart() {
        let (mut work, chain) = interrupted_coverage_fixture().await;
        let block = &chain[2];
        let envelopes = block.receipts.iter().enumerate().map(|(index, receipt)| {
            (index as u64, ethereum_common::utils::map_receipt_envelope(receipt.as_ref()))
        }).collect::<Vec<_>>();
        let mut second = work.admissions[0].clone();
        second.receipt_key.1 = 1;
        second.original_tx_hash = block.transactions[1];
        second.signed = None;
        let mut proof = eth_events_electra_client::EthToVaraEvent::decode(
            &mut second.proof.as_slice(),
        ).unwrap();
        proof.transaction_index = 1;
        proof.receipt_rlp = alloy_rlp::encode(&envelopes[1].1);
        proof.proof = ethereum_common::utils::generate_merkle_proof(1, &envelopes).unwrap().proof;
        second.proof = proof.encode();
        work.admissions.push(second);
        work.blocks = chain[..3].to_vec();
        work.intent.boundary.execution_number = 2;
        work.intent.boundary.execution_hash = block.hash;
        work.intent.boundary.slot = 32;
        work.intent.boundary_execution_timestamp = 24;
        let primary = WorkEvidenceRpc::new(chain.clone()).await;
        let witness = WorkEvidenceRpc::new(chain.clone()).await;
        let eth = PollingEthApi::new(&primary.url).await.unwrap()
            .with_finality_witness(&witness.url, 1, chain[0].hash.0.into()).await.unwrap();
        let mut view = eth.verified_finalized_view().await.unwrap();
        work.schema_version = 3;
        work.ethereum_authentication = Some(view.checkpoint().clone());
        let directory = std::env::temp_dir().join(format!("cutover-block-commit-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        let path = work_path(&directory);
        // One durable save contains the cursor and BOTH pending deposits, including
        // the retained signed original. Restart needs no old transaction bodies.
        save(&path, &work).unwrap();
        let original = fs::read(&path).unwrap();
        let mut restarted = read_work(&path).unwrap();
        primary.bodies.lock().unwrap().clear();
        witness.bodies.lock().unwrap().clear();
        authenticate_work_ethereum(&eth, &mut view, &mut restarted).await.unwrap();
        assert_eq!(restarted.blocks.last().unwrap().number, restarted.intent.boundary.execution_number);
        assert_eq!(restarted.admissions.len(), 2);
        assert!(restarted.admissions.iter().all(|row| row.settlement.is_none() && row.exclusion.is_none()));
        assert!(restarted.admissions[0].signed.is_some());
        assert!(restarted.admissions[1].signed.is_none());
        assert!(completed_coverage(&restarted).unwrap_err().to_string().contains("undrained"),
            "a committed scan cursor is not an economic settlement");
        assert_eq!(serde_json::to_vec(&restarted).unwrap(), original);
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::write(PathBuf::from(format!("{}.new", path.display())), b"interrupted").unwrap();
        assert!(read_work(&path).is_err(), "an incomplete atomic write must still HOLD");
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn signer_release_requires_authenticated_terminal_evidence_and_preserves_originals() {
        let fixture = terminal_coverage_fixture().await;
        for provenance in ["owned", "sponsored", "external"] {
            for disposition in [
                "excluded", "settled", "pending", "conflict", "invalid", "lifecycle",
            ] {
                let mut work = fixture.clone();
                let admission = &mut work.admissions[0];
                match provenance {
                    "owned" => admission.original_lane_uuid = Some(Uuid::now_v7()),
                    "external" => {
                        admission.external_original = Some(seal::CanonicalExternalOriginal {
                            provenance: "canonical-external".into(),
                            original: admission.signed.take().unwrap(),
                            discovered_at: admission.exclusion.clone().unwrap(),
                        });
                        admission.admission = "externally-resolved".into();
                    }
                    _ => {}
                }
                if provenance != "external" {
                    admission.attempts.push(original_with_reply(None));
                }
                match disposition {
                    "settled" => {
                        admission.settlement = admission.exclusion.take();
                        if provenance == "external" {
                            admission.admission = "externally-delivered".into();
                        }
                    }
                    "pending" => admission.exclusion = None,
                    "conflict" => admission.settlement = admission.exclusion.clone(),
                    "lifecycle" => work.calls.push(FinalizedCutoverCall {
                        step: CutoverStep::DestinationUnpause,
                        attempt: 0,
                        target: work.intent.new_identity.vft_manager_address,
                        payload: io::Unpause::encode_call(),
                        gas: 1,
                        value: 0,
                        original: original_with_reply(None),
                        archived_no_effect: false,
                    }),
                    _ => {}
                }
                let directory = std::env::temp_dir()
                    .join(format!("cutover-signer-release-{}", Uuid::now_v7()));
                fs::create_dir(&directory).unwrap();
                let path = work_path(&directory);
                let genesis = seal::digest(Uuid::now_v7().as_bytes());
                work.intent.old_identity.gear_genesis_hash = genesis;
                save(&path, &work).unwrap();
                let before = fs::read(&path).unwrap();
                let current_sender = [41; 32];
                let other_sender = [42; 32];
                let current =
                    signer_ownership::lock_cutover_signer(genesis, current_sender, &path).unwrap();
                let others = vec![(
                    other_sender,
                    signer_ownership::lock_cutover_signer(genesis, other_sender, &path).unwrap(),
                    true,
                )];
                let authenticated = std::cell::Cell::new(false);
                // Exercise the real release gate and durable ownership; the RPC
                // authentication boundary is controlled here, not chain evidence.
                let result = release_signers_after_authentication(
                    &work,
                    &path,
                    current_sender,
                    &current,
                    &others,
                    async {
                        authenticated.set(true);
                        ensure!(disposition != "invalid", "HOLD: invalid exclusion evidence");
                        Ok(())
                    },
                )
                .await;
                let released = matches!(disposition, "excluded" | "settled");
                assert_eq!(result.is_ok(), released, "{provenance}/{disposition}");
                assert_eq!(
                    authenticated.get(),
                    matches!(disposition, "excluded" | "settled" | "invalid"),
                    "{provenance}/{disposition}"
                );
                assert_eq!(fs::read(&path).unwrap(), before);
                assert_eq!(serde_json::to_vec(&work).unwrap(), before);
                drop(current);
                drop(others);
                for account in [current_sender, other_sender] {
                    let normal_owner = signer_ownership::lock_signer(genesis, account);
                    assert_eq!(
                        normal_owner.is_ok(),
                        released,
                        "durable owner must survive an unsuccessful release: {provenance}/{disposition}"
                    );
                    drop(normal_owner);
                    if !released {
                        let retained =
                            signer_ownership::lock_cutover_signer(genesis, account, &path).unwrap();
                        signer_ownership::release_cutover_signer(genesis, account, &path, &retained)
                            .unwrap();
                    }
                }
                fs::remove_dir_all(directory).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn canonical_terminal_coverage_preserves_each_original_provenance_and_xor() {
        let fixture = terminal_coverage_fixture().await;
        for provenance in ["owned", "sponsored", "external"] {
            let mut work = fixture.clone();
            let admission = &mut work.admissions[0];
            match provenance {
                "owned" => admission.original_lane_uuid = Some(Uuid::now_v7()),
                "external" => {
                    admission.external_original = Some(seal::CanonicalExternalOriginal {
                        provenance: "canonical-external".into(),
                        original: admission.signed.take().unwrap(),
                        discovered_at: admission.exclusion.clone().unwrap(),
                    });
                    admission.admission = "externally-resolved".into();
                }
                _ => {}
            }
            let before = serde_json::to_vec(&work).unwrap();
            completed_coverage(&work).unwrap();
            assert_eq!(
                serde_json::to_vec(&work).unwrap(),
                before,
                "read-only coverage must not rewrite original work"
            );
            work.admissions[0].settlement = work.admissions[0].exclusion.clone();
            assert!(
                completed_coverage(&work).is_err(),
                "settlement and exclusion are mutually exclusive"
            );
            work.admissions[0].settlement = None;
            work.admissions[0].exclusion = None;
            assert!(
                completed_coverage(&work).is_err(),
                "neither terminal pin cannot qualify"
            );
        }
    }

    #[tokio::test]
    async fn canonical_terminal_coverage_rejects_hash_index_header_and_uuid_substitution() {
        let fixture = terminal_coverage_fixture().await;
        for field in ["tx", "index", "block", "receipt", "root"] {
            let mut work = fixture.clone();
            match field {
                "tx" => work.admissions[0].original_tx_hash = B256::repeat_byte(44),
                "index" => work.admissions[0].receipt_key.1 = 1,
                "block" => work.admissions[0].original_block_hash = H256::repeat_byte(44),
                "receipt" => work.blocks[0].receipts[0].transaction_hash = B256::repeat_byte(44),
                "root" => work.blocks[0].header.transactions_root = B256::repeat_byte(44),
                _ => unreachable!(),
            }
            assert!(completed_coverage(&work).is_err(), "{field}");
        }
        let mut admissions = fixture.admissions.clone();
        admissions[0].original_lane_uuid = Some(Uuid::now_v7());
        let mut duplicate = admissions[0].clone();
        duplicate.receipt_key.1 += 1;
        duplicate.original_tx_hash = B256::repeat_byte(45);
        admissions.push(duplicate);
        assert!(validate_admission_uniqueness(&admissions)
            .unwrap_err()
            .to_string()
            .contains("UUID"));
        admissions[1].original_lane_uuid = None;
        admissions[1].undispatched_old_admission = admissions[0].original_lane_uuid;
        assert!(
            validate_admission_uniqueness(&admissions).is_err(),
            "UUID cannot change ownership kind"
        );
    }

    fn historical_wrapper_pin(outstanding: u64, blocking: Option<u64>) -> RolePin {
        RolePin {
            token: H256::repeat_byte(1),
            block_number: 10,
            block_hash: H256::repeat_byte(2),
            minter: H256::repeat_byte(3),
            burner: H256::repeat_byte(3),
            manager: Some(H256::repeat_byte(3)),
            outstanding_redemptions: Some(outstanding),
            blocking_redemptions: blocking,
            paused: true,
        }
    }

    #[tokio::test]
    async fn version_one_cannot_smuggle_exclusions_or_blocking_counts() {
        let mut work = terminal_coverage_fixture().await;
        work.schema_version = 1;
        assert!(validate_work_version(&work).is_err());
        work.admissions[0].settlement = work.admissions[0].exclusion.take();
        completed_coverage(&work).unwrap();
        work.intent
            .initial_role_pins
            .push(historical_wrapper_pin(0, None));
        validate_work_version(&work).unwrap();
        work.intent.initial_role_pins[0].blocking_redemptions = Some(0);
        assert!(validate_work_version(&work).is_err());
    }

    #[tokio::test]
    async fn version_three_preserves_exclusion_rules_and_requires_checkpoint() {
        let mut work = terminal_coverage_fixture().await;
        let original = serde_json::to_vec(&work.admissions).unwrap();
        work.schema_version = 3;
        assert!(completed_coverage(&work).is_err());
        let header = work.blocks[0].header.clone();
        work.intent.old_identity.ethereum_genesis_hash = H256(header.hash_slow().0);
        work.ethereum_authentication = Some(ethereum_client::FinalityCheckpoint {
            schema_version: 1,
            policy: "independent-two-el-v1".into(),
            chain_id: work.intent.old_identity.ethereum_chain_id,
            genesis_hash: header.hash_slow(),
            common_header: header.clone(),
            primary_finalized_header: header.clone(),
            witness_finalized_header: header,
        });
        completed_coverage(&work).unwrap();
        assert_eq!(serde_json::to_vec(&work.admissions).unwrap(), original);
        work.admissions[0].settlement = work.admissions[0].exclusion.clone();
        assert!(completed_coverage(&work).is_err());
        work.admissions[0].settlement = None;
        work.ethereum_authentication.as_mut().unwrap().policy = "single-provider".into();
        assert!(completed_coverage(&work).is_err());
    }

    #[tokio::test]
    async fn legacy_work_rejects_present_null_checkpoint_without_rewriting() {
        let mut work = terminal_coverage_fixture().await;
        let directory = std::env::temp_dir().join(format!("cutover-schema-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        let path = work_path(&directory);
        for version in [1, 2] {
            work.schema_version = version;
            work.admissions[0].settlement = work.admissions[0].exclusion.take()
                .or(work.admissions[0].settlement.clone());
            let mut value = serde_json::to_value(&work).unwrap();
            value["ethereum_authentication"] = serde_json::Value::Null;
            let bytes = serde_json::to_vec(&value).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(read_work(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn historical_role_upgrade_requires_unchanged_pins_and_separates_liability() {
        let saved = historical_wrapper_pin(1, None);
        let upgraded = upgrade_role_evidence(&saved, historical_wrapper_pin(1, Some(0))).unwrap();
        assert_eq!(upgraded.outstanding_redemptions, Some(1));
        assert!(zero_blocking(&upgraded, 2).unwrap());
        assert!(zero_blocking(&saved, 1).is_err());
        assert!(upgrade_role_evidence(&saved, saved.clone()).is_err());
        let drained = historical_wrapper_pin(0, None);
        assert_eq!(
            upgrade_role_evidence(&drained, drained.clone())
                .unwrap()
                .blocking_redemptions,
            Some(0)
        );
        for field in [
            "hash",
            "height",
            "manager",
            "paused",
            "outstanding",
            "blocking",
        ] {
            let mut changed = historical_wrapper_pin(1, Some(0));
            match field {
                "hash" => changed.block_hash = H256::repeat_byte(99),
                "height" => changed.block_number += 1,
                "manager" => changed.manager = Some(H256::repeat_byte(99)),
                "paused" => changed.paused = false,
                "outstanding" => changed.outstanding_redemptions = Some(2),
                "blocking" => changed.blocking_redemptions = Some(2),
                _ => unreachable!(),
            }
            assert!(upgrade_role_evidence(&saved, changed).is_err(), "{field}");
        }
    }

    #[test]
    fn historical_drained_inference_survives_upgrade_and_revalidation() {
        let legacy = historical_wrapper_pin(0, None);
        assert!(allow_historical_drained_inference(&legacy, 1));
        let upgraded = upgrade_role_evidence(&legacy, legacy.clone()).unwrap();
        assert_eq!(upgraded, historical_wrapper_pin(0, Some(0)));
        for versions in [[1, 3, 3], [1, 2, 3]] {
            let mut saved = legacy.clone();
            for version in versions {
                assert!(allow_historical_drained_inference(&saved, version));
                if version == 1 {
                    saved = upgrade_role_evidence(&saved, saved.clone()).unwrap();
                }
                assert_eq!(saved, upgraded);
                saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
            }
        }
    }

    #[test]
    fn historical_drained_inference_rejects_nonzero_unknown_and_contradictory_counts() {
        for version in [1, 2, 3] {
            for pin in [
                historical_wrapper_pin(1, None),
                historical_wrapper_pin(1, Some(0)),
                historical_wrapper_pin(0, Some(1)),
                RolePin {
                    outstanding_redemptions: None,
                    ..historical_wrapper_pin(0, Some(0))
                },
            ] {
                assert!(!allow_historical_drained_inference(&pin, version));
            }
        }
        for version in [2, 3] {
            assert!(!allow_historical_drained_inference(
                &historical_wrapper_pin(0, None),
                version,
            ));
        }
    }

    #[test]
    fn exclusion_copy_retains_original_parent_and_continuation_observations() {
        let (proof, mut original) = external_parent_fixture(true);
        let continuation = external_continuation_fixture(&original, 1);
        original.native_reconciliation = Some(Box::new(continuation));
        let encoded = serde_json::to_vec(&original).unwrap();
        let copy = original.clone();
        assert!(preserves_original(&copy, &original));
        for field in ["raw", "reply", "nonce", "child", "attempt"] {
            let mut changed = copy.clone();
            match field {
                "raw" => changed.raw_extrinsic.push(0),
                "reply" => changed.finalized_reply.as_mut().unwrap().payload.push(0),
                "nonce" => changed.finalized_nonce_consumed = Some(100),
                "child" => changed.native_reconciliation.as_mut().unwrap().message_id = None,
                "attempt" => changed
                    .native_reconciliation_attempts
                    .push(external_continuation_fixture(&original, 2)),
                _ => unreachable!(),
            }
            if field == "nonce" {
                let mut saved = original.clone();
                saved.finalized_nonce_consumed = Some(99);
                assert!(!preserves_original(&changed, &saved));
            } else {
                assert!(!preserves_original(&changed, &original), "{field}");
            }
        }
        assert_eq!(serde_json::to_vec(&original).unwrap(), encoded);
        assert!(!proof.is_empty());
    }

    #[tokio::test]
    async fn owned_exclusion_uses_work_copy_without_completing_or_rewriting_original_row() {
        use relayer::message_relayer::{
            common::{EthereumSlotNumber, TxHashWithSlot},
            eth_to_gear::tx_manager::{ReceiptEvidence, TxStatus},
        };
        let mut work = terminal_coverage_fixture().await;
        let admission = &mut work.admissions[0];
        let mut original = admission.signed.clone().unwrap();
        original.sender =
            subxt::utils::AccountId32(work.intent.old_identity.gear_sender).to_string();
        original.receipt_key = admission.receipt_key;
        let mut row = Transaction::new(
            TxHashWithSlot {
                tx_hash: admission.original_tx_hash,
                slot_number: EthereumSlotNumber(32),
            },
            TxStatus::NeedsReconciliation {
                diagnostic: "retained held liability".into(),
            },
        );
        row.receipt = Some(ReceiptEvidence {
            payload: admission.proof.clone(),
            receipt_key: admission.receipt_key,
            composed_at_ms: 0,
            handed_off_at_ms: Some(1),
            initial_response: None,
            signed_submission: Some(original.clone()),
            submission_attempts: vec![],
        });
        admission.original_lane_uuid = Some(row.uuid);
        admission.signed = Some(original.clone());
        admission.signed.as_mut().unwrap().finalized_reply =
            original_with_reply(Some(vec![1, 2])).finalized_reply;
        let directory =
            std::env::temp_dir().join(format!("cutover-owned-exclusion-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        let journal = seal::OriginalJournal {
            schema_version: 7,
            runtime_identity: Some(work.intent.old_identity.clone()),
            predecessor_cutover: None,
            transactions: [(row.uuid, row.clone())].into(),
            completed: Default::default(),
            failed: Default::default(),
        };
        atomic_json(&directory.join("state.json"), &journal, false).unwrap();
        let before = fs::read(directory.join("state.json")).unwrap();
        let retained = seal::read_original_journal(&directory).unwrap();
        validate_owned_exclusion(
            &work.intent.old_identity,
            admission,
            retained.transactions.get(&row.uuid).unwrap(),
        )
        .unwrap();
        assert!(retained.completed.is_empty());
        assert!(matches!(
            retained.transactions[&row.uuid].status,
            TxStatus::NeedsReconciliation { .. }
        ));
        assert_eq!(
            retained.transactions[&row.uuid]
                .receipt
                .as_ref()
                .unwrap()
                .signed_submission
                .as_ref(),
            Some(&original)
        );
        assert_eq!(fs::read(directory.join("state.json")).unwrap(), before);
        admission.signed.as_mut().unwrap().raw_extrinsic.push(0);
        assert!(validate_owned_exclusion(&work.intent.old_identity, admission, &row).is_err());
        admission.signed = Some(original);
        admission.original_lane_uuid = Some(Uuid::now_v7());
        assert!(validate_owned_exclusion(&work.intent.old_identity, admission, &row).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn version_one_reader_rejects_even_null_new_fields_without_rewriting_work() {
        let directory = std::env::temp_dir().join(format!("cutover-v1-fields-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("cutover-work.json");
        let mut work = terminal_coverage_fixture().await;
        work.schema_version = 1;
        work.admissions[0].settlement = work.admissions[0].exclusion.take();
        work.intent
            .initial_role_pins
            .push(historical_wrapper_pin(0, None));
        atomic_json(&path, &work, false).unwrap();
        assert_eq!(read_work(&path).unwrap().schema_version, 1);
        for field in ["exclusion", "blocking_redemptions"] {
            let mut value = serde_json::to_value(&work).unwrap();
            if field == "exclusion" {
                value["admissions"][0][field] = serde_json::Value::Null;
            } else {
                value["intent"]["initial_role_pins"][0][field] = serde_json::Value::Null;
            }
            atomic_json(&path, &value, false).unwrap();
            let before = fs::read(&path).unwrap();
            assert!(read_work(&path).is_err(), "{field}");
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn external_resolved_continuation_is_terminal_but_not_financial_success() {
        let (proof, mut parent) = external_parent_fixture(true);
        let mut resolved = external_continuation_fixture(&parent, 1);
        let mut payload = io::ReconcileReceipt::ROUTE.to_vec();
        payload.extend(
            Result::<vft_manager_client::ReceiptStatus, vft_manager_client::Error>::Ok(
                vft_manager_client::ReceiptStatus::Resolved,
            )
            .encode(),
        );
        resolved.finalized_reply.as_mut().unwrap().payload = payload;
        let redundant = external_continuation_fixture(&parent, 2);
        attach_external_native_continuations(&proof, &mut parent, &[resolved.clone(), redundant])
            .unwrap();
        assert_eq!(parent.native_reconciliation.as_deref(), Some(&resolved));
        assert!(parent.native_reconciliation_attempts.is_empty());
        let mut reply = io::SubmitReceipt::ROUTE.to_vec();
        reply.extend(
            Result::<(), vft_manager_client::Error>::Err(
                vft_manager_client::Error::ReceiptResolved,
            )
            .encode(),
        );
        use historical_proxy_client::historical_proxy::io::Redirect;
        let event =
            eth_events_electra_client::EthToVaraEvent::decode(&mut proof.as_slice()).unwrap();
        let nested: <Redirect as ActionIo>::Reply = Ok((event.receipt_rlp, reply));
        let mut encoded = Redirect::ROUTE.to_vec();
        encoded.extend(nested.encode());
        let exception = original_with_reply(Some(encoded));
        assert!(
            !accepted_external_parent(&proof, &exception),
            "exception admission must use independent exclusion authentication"
        );
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
