use super::*;

#[derive(Args)]
pub struct AbortCutover {
    #[arg(long)]
    pub cutover_dir: PathBuf,
    #[arg(long)]
    pub expected_generation: u64,
    #[arg(long)]
    pub ethereum_rpc: String,
    #[command(flatten)]
    pub ethereum_finality: EthereumFinalityArgs,
    #[command(flatten)]
    pub native_capability: NativeCapabilityArgs,
}

pub async fn abort(api: &GearApi, args: &AbortCutover) -> Result<()> {
    let _work = lock_work(&args.cutover_dir)?;
    let path = work_path(&args.cutover_dir);
    let approved = args.native_capability.load()?;
    bind_invocation_approval(api, &path, approved.as_ref()).await?;
    let work = read_work(&path)?;
    ensure_unfrozen(&path, &work)?;
    let intent = &work.intent;
    ensure!(
        intent.cutover_generation == Some(args.expected_generation),
        "HOLD: abort generation differs from independently retained preparation"
    );
    let _journals = seal::lock_original_journals(&intent.old_transactions, &intent.old_discovery)?;
    let owner = lock_signer(&path, api, intent.old_identity.gear_genesis_hash)?;
    let others = lock_other_signers(api, intent, &work, &path)?;
    let eth = ethereum_api(
        &args.ethereum_rpc,
        &args.ethereum_finality,
        &intent.old_identity,
    )
    .await?;
    eth.enable_finality_archive(
        &args.cutover_dir.join("ethereum-finality"),
        intent.old_identity.ethereum_genesis_hash.0.into(),
    )
    .await?;
    authenticate_environment(api, &eth, intent).await?;
    let service = VftManager::new(GClientRemoting::new(api.clone()));
    let source = actor(intent.old_identity.vft_manager_address);
    let destination = actor(intent.new_identity.vft_manager_address);
    let at = api.last_block_hash().await?;
    let pin = sails_rs::H256::from_slice(at.as_ref());
    let aborted = service
        .cutover_abort(args.expected_generation)
        .at_block(pin)
        .recv(source)
        .await?;
    if aborted.is_none() {
        let (current, _) = status(api, intent.old_identity.vft_manager_address).await?;
        ensure!(current.generation == args.expected_generation && current.phase == CutoverPhase::Draining
            && !current.custody_started && current.in_flight_transfer_count == 0
            && current.successor == Some(destination) && current.last_receipt_slot == Some(intent.boundary.slot),
            "HOLD: abort is only permitted for exact pre-custody Draining generation; post-child freezes remain gated");
        if !service.is_paused().at_block(pin).recv(source).await? {
            run_cutover_call(api, &eth, &path, CutoverStep::SourcePause, intent).await?;
        }
        run_cutover_call(
            api,
            &eth,
            &path,
            CutoverStep::AbortCutover(args.expected_generation),
            intent,
        )
        .await?;
    }
    let at = api.last_block_hash().await?;
    let pin = sails_rs::H256::from_slice(at.as_ref());
    let proof = service
        .cutover_abort(args.expected_generation)
        .at_block(pin)
        .recv(source)
        .await?
        .context("HOLD: finalized generation abort evidence unavailable")?;
    ensure!(
        proof.generation == args.expected_generation
            && proof.successor == destination
            && proof.last_receipt_slot == intent.boundary.slot
            && proof.config_fingerprint.0 == intent.config_fingerprint.0,
        "HOLD: abort evidence does not match this exact preparation"
    );
    let (successor, _) = status(api, intent.new_identity.vft_manager_address).await?;
    if successor.phase == CutoverPhase::AwaitingSource {
        ensure!(
            successor.predecessor == Some(source)
                && successor.predecessor_generation == Some(args.expected_generation),
            "HOLD: destination belongs to another source generation"
        );
        run_cutover_call(
            api,
            &eth,
            &path,
            CutoverStep::RestoreAbortedCutover(successor.generation),
            intent,
        )
        .await?;
    }
    let (source_status, _) = status(api, intent.old_identity.vft_manager_address).await?;
    let (destination_status, _) = status(api, intent.new_identity.vft_manager_address).await?;
    let at = api.last_block_hash().await?;
    let pin = sails_rs::H256::from_slice(at.as_ref());
    ensure!(
        source_status.phase == CutoverPhase::Active
            && source_status.generation
                == args
                    .expected_generation
                    .checked_add(1)
                    .context("Generation overflow")?
            && source_status.minimum_receipt_slot == intent.source_receipt_floor
            && !source_status.custody_started
            && destination_status.phase == CutoverPhase::Active
            && destination_status.predecessor.is_none()
            && destination_status.minimum_receipt_slot
                == intent
                    .boundary
                    .slot
                    .checked_add(1)
                    .context("Boundary overflow")?
            && service.is_paused().at_block(pin).recv(source).await?
            && service.is_paused().at_block(pin).recv(destination).await?,
        "HOLD: abort/restoration incomplete; both managers must remain paused"
    );
    immutable_abort(&path, args.expected_generation, &proof.encode())?;
    release_owned_signers(api, &eth, &path, &owner, &others).await?;
    println!("Aborted exact pre-custody generation; originals and liabilities retained, both managers paused.");
    Ok(())
}
fn immutable_abort(path: &Path, generation: u64, evidence: &[u8]) -> Result<()> {
    atomic_json(
        &path.with_file_name(format!("cutover-abort-{generation}.json")),
        &serde_json::json!({"generation": generation, "evidence": hex::encode(evidence)}),
        true,
    )
}
