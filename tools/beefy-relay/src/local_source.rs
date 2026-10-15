use super::*;
use anyhow::bail;
use gsdk::{
    ext::sp_core::{sr25519, Pair as _},
    PairSigner,
};
use std::fs::OpenOptions;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RotationEvidence {
    authority: String,
    stash: Hash32,
    beefy_key: Vec<u8>,
    extrinsic_hash: Hash32,
    block: u32,
    block_hash: Hash32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedRotation {
    authority: String,
    stash: Hash32,
    session_keys: Vec<u8>,
    ownership_proof: Vec<u8>,
    source_genesis: Hash32,
    nonce: u64,
    from_height: u32,
    from_hash: Hash32,
    extrinsic_hash: Hash32,
    signed_extrinsic: Vec<u8>,
    call_data: Vec<u8>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct ObservedMessage {
    pub block: u32,
    pub block_hash: Hash32,
    pub message: dto::Message,
    pub message_hash: Hash32,
    pub inclusion: dto::MerkleProof,
    pub snapshot: QueueSnapshot,
    pub retained_at_block: u32,
}

fn validate_native_keys(keys: &[u8], proof: &[u8]) -> Result<()> {
    ensure!(
        keys.len() == 161 && proof.len() == 321,
        "native registration requires all five session keys and ownership proofs; HOLD"
    );
    ensure!(
        matches!(keys[128], 2 | 3),
        "native BEEFY key is not compressed SEC1; HOLD"
    );
    Ok(())
}

async fn rotation_call(
    api: &GearApi,
    block_hash: H256,
    stash: Hash32,
    keys: &[u8],
    proof: &[u8],
) -> Result<gsdk::ext::subxt::tx::DynamicPayload> {
    validate_native_keys(keys, proof)?;
    let address = subxt::dynamic::storage("Session", "NextKeys", vec![Value::from_bytes(stash)]);
    let stored = api
        .api
        .storage()
        .at(block_hash)
        .fetch(&address)
        .await?
        .context("Session.NextKeys has no entry for active stash")?;
    let mut key_value = stored.to_value()?.remove_context();
    let gsdk::ext::scale_value::ValueDef::Composite(gsdk::ext::scale_value::Composite::Named(
        fields,
    )) = &mut key_value.value
    else {
        bail!("Session.NextKeys is not a named key bundle in live metadata");
    };
    let names = [
        "babe",
        "grandpa",
        "im_online",
        "authority_discovery",
        "beefy",
    ];
    ensure!(
        fields.len() == names.len(),
        "runtime does not expose the native five-key bundle"
    );
    for (index, ((name, value), expected)) in fields.iter_mut().zip(names).enumerate() {
        ensure!(
            name == expected,
            "native session key ordering differs from live metadata; HOLD"
        );
        let end = if index == 4 { 161 } else { (index + 1) * 32 };
        *value = Value::from_bytes(&keys[index * 32..end]);
    }
    Ok(gsdk::ext::subxt::tx::dynamic(
        "Session",
        "set_keys",
        vec![key_value, Value::from_bytes(proof)],
    ))
}

async fn validate_rotation(api: &GearApi, prepared: &PreparedRotation) -> Result<()> {
    ensure!(
        api.block_number_to_hash(0).await?.0 == prepared.source_genesis
            && api.block_number_to_hash(prepared.from_height).await?.0 == prepared.from_hash,
        "prepared rotation belongs to another source or original checkpoint; HOLD"
    );
    ensure!(
        sp_core::blake2_256(&prepared.signed_extrinsic) == prepared.extrinsic_hash,
        "prepared rotation signed bytes/hash changed; HOLD"
    );
    let decoded = subxt::ext::subxt_core::blocks::Extrinsics::<gsdk::GearConfig>::decode_from(
        vec![prepared.signed_extrinsic.clone()],
        api.api.metadata(),
    )?;
    let extrinsic = decoded
        .iter()
        .next()
        .context("prepared rotation has no extrinsic")?;
    let mut address = [0; 33];
    address[1..].copy_from_slice(&prepared.stash);
    ensure!(
        extrinsic.address_bytes() == Some(address.as_slice())
            && extrinsic
                .transaction_extensions()
                .and_then(|extensions| extensions.nonce())
                == Some(prepared.nonce)
            && extrinsic.pallet_name()? == "Session"
            && extrinsic.variant_name()? == "set_keys"
            && extrinsic.call_bytes() == prepared.call_data,
        "prepared rotation signer, nonce or call changed; HOLD"
    );
    let call = rotation_call(
        api,
        H256(prepared.from_hash),
        prepared.stash,
        &prepared.session_keys,
        &prepared.ownership_proof,
    )
    .await?;
    ensure!(
        api.api.tx().call_data(&call)? == prepared.call_data,
        "prepared native key bundle or ownership proof changed; HOLD"
    );
    Ok(())
}

async fn rotation_evidence(
    api: &GearApi,
    prepared: &PreparedRotation,
    hash: H256,
) -> Result<RotationEvidence> {
    let keys = storage_encoded(
        api,
        "Session",
        "NextKeys",
        hash,
        vec![Value::from_bytes(prepared.stash)],
    )
    .await?
    .context("finalized session key registration is missing; HOLD")?;
    ensure!(
        keys == prepared.session_keys,
        "finalized Session.NextKeys differs from the original native bundle; HOLD"
    );
    Ok(RotationEvidence {
        authority: prepared.authority.clone(),
        stash: prepared.stash,
        beefy_key: prepared.session_keys[128..].to_vec(),
        extrinsic_hash: prepared.extrinsic_hash,
        block: api.block_hash_to_number(hash).await?,
        block_hash: hash.0,
    })
}

async fn submit_prepared_rotation(
    api: &GearApi,
    prepared: &PreparedRotation,
) -> Result<RotationEvidence> {
    validate_rotation(api, prepared).await?;
    let transaction = subxt::tx::SubmittableTransaction::<gsdk::GearConfig, _>::from_bytes(
        (*api.api).clone(),
        prepared.signed_extrinsic.clone(),
    );
    ensure!(
        transaction.hash().0 == prepared.extrinsic_hash,
        "prepared rotation SDK hash changed; HOLD"
    );
    let progress = transaction.submit_and_watch().await?;
    let finalized = timeout(MAX_WAIT, progress.wait_for_finalized())
        .await
        .context("original rotation finality pending; HOLD")??;
    finalized
        .wait_for_success()
        .await
        .context("original rotation failed dispatch; HOLD")?;
    rotation_evidence(api, prepared, finalized.block_hash()).await
}

/// Read-only recovery of the original extrinsic; absence never permits re-signing.
async fn reconcile_rotation(
    api: &GearApi,
    prepared: &PreparedRotation,
) -> Result<Option<RotationEvidence>> {
    timeout(MAX_WAIT, async {
        validate_rotation(api, prepared).await?;
        let finalized = api
            .block_hash_to_number(api.latest_finalized_block().await?)
            .await?;
        ensure!(
            finalized >= prepared.from_height,
            "source finality regressed behind rotation anchor; HOLD"
        );
        for height in prepared.from_height..=finalized {
            let hash = api.block_number_to_hash(height).await?;
            let block = api.get_block_at(hash).await?;
            let extrinsics = block.extrinsics().await?;
            for extrinsic in extrinsics.iter() {
                if extrinsic.hash().0 != prepared.extrinsic_hash {
                    continue;
                }
                ensure!(
                    extrinsic.bytes() == prepared.signed_extrinsic,
                    "original rotation inclusion changed bytes; HOLD"
                );
                let events = extrinsic.events().await?;
                let mut success = false;
                for event in events.iter() {
                    let event = event?;
                    ensure!(
                        event.pallet_name() != "System"
                            || event.variant_name() != "ExtrinsicFailed",
                        "original rotation failed dispatch; HOLD"
                    );
                    success |= event.pallet_name() == "System"
                        && event.variant_name() == "ExtrinsicSuccess";
                }
                ensure!(
                    success,
                    "original rotation has no finalized successful dispatch; HOLD"
                );
                return Ok(Some(rotation_evidence(api, prepared, hash).await?));
            }
        }
        Ok(None)
    })
    .await
    .context("original rotation history lookup pending; HOLD")?
}

async fn prepare_rotation(
    api: &GearApi,
    authority: &str,
    stash_pair: sr25519::Pair,
    source_genesis: Hash32,
) -> Result<PreparedRotation> {
    let stash = stash_pair.public().0;
    let block_hash = api.latest_finalized_block().await?;
    let bonded: [u8; 32] = storage_decode(
        api,
        "Staking",
        "Bonded",
        block_hash,
        vec![Value::from_bytes(stash)],
    )
    .await?
    .context("validator stash is not bonded")?;
    ensure!(
        bonded == stash,
        "Staking.Bonded does not map the signer to its validator stash"
    );
    let ledger = storage_encoded(
        api,
        "Staking",
        "Ledger",
        block_hash,
        vec![Value::from_bytes(stash)],
    )
    .await?
    .context("validator stash has no staking ledger")?;
    ensure!(
        ledger.get(..32) == Some(stash.as_slice()),
        "Staking.Ledger belongs to another stash"
    );
    let validators = storage_encoded(api, "Session", "Validators", block_hash, vec![])
        .await?
        .context("Session validators storage is unavailable")?;
    ensure!(
        decode_session_validators(&validators)?.contains(&stash),
        "{authority} stash is not an active validator"
    );
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct GeneratedSessionKeys {
        keys: String,
        proof: String,
    }
    let generated: GeneratedSessionKeys = api
        .api
        .rpc()
        .request(
            "author_rotateKeysWithOwner",
            rpc_params![format!("0x{}", hex::encode(stash))],
        )
        .await
        .context("generate native owner-bound session keys")?;
    let session_keys = decode_hex_bounded(&generated.keys, 161)?;
    let ownership_proof = decode_hex_bounded(&generated.proof, 321)?;
    validate_native_keys(&session_keys, &ownership_proof)?;
    let present: bool = api
        .api
        .rpc()
        .request("author_hasSessionKeys", rpc_params![&generated.keys])
        .await?;
    ensure!(
        present,
        "native session keys are absent from the generating node keystore; HOLD"
    );
    let call = rotation_call(api, block_hash, stash, &session_keys, &ownership_proof).await?;
    let signer = PairSigner::<gsdk::GearConfig, sr25519::Pair>::new(stash_pair);
    let nonce = api.api.tx().account_nonce(signer.account_id()).await?;
    let params = subxt::config::polkadot::PolkadotExtrinsicParamsBuilder::<gsdk::GearConfig>::new()
        .nonce(nonce)
        .immortal()
        .build();
    let transaction = api.api.tx().create_signed(&call, &signer, params).await?;
    Ok(PreparedRotation {
        authority: authority.to_owned(),
        stash,
        session_keys,
        ownership_proof,
        source_genesis,
        nonce,
        from_height: api.block_hash_to_number(block_hash).await?,
        from_hash: block_hash.0,
        extrinsic_hash: transaction.hash().0,
        signed_extrinsic: transaction.encoded().to_vec(),
        call_data: api.api.tx().call_data(&call)?,
    })
}

#[cfg(test)]
pub async fn send_message(
    api: &GearApi,
    receiver: [u8; 20],
    payload: &[u8],
    gear_suri: &str,
) -> Result<ObservedMessage> {
    let pair = sr25519::Pair::from_string(gear_suri, None).context("derive message signer")?;
    let sender = pair.public().0;
    let call = gsdk::ext::subxt::tx::dynamic(
        "GearEthBridge",
        "send_eth_message",
        vec![Value::from_bytes(receiver), Value::from_bytes(payload)],
    );
    let (progress, block_hash) = submit_in_block(api, call, pair).await?;
    ensure!(
        has_event(api, block_hash, "GearEthBridge", "MessageQueued").await?,
        "included message transaction has no dynamic MessageQueued event"
    );
    let messages = api.message_queued_events(block_hash).await?;
    let message = messages
        .into_iter()
        .find(|message| {
            message.source == sender
                && message.destination == receiver
                && message.payload == payload
        })
        .ok_or_else(|| {
            anyhow!("MessageQueued event does not contain requested source/destination/payload")
        })?;
    let block = api.block_hash_to_number(block_hash).await?;
    let bridge_domain: Hash32 =
        storage_decode(api, "GearEthBridge", "BridgeDomain", block_hash, vec![])
            .await?
            .context("source bridge domain is missing")?;
    let source_timestamp_ms = api.fetch_timestamp(block_hash).await?;
    let (queue_id, queue_root) = api.fetch_queue_merkle_root(block_hash).await?;
    let initialized = bridge_initialized(api, block_hash).await?;
    ensure!(
        initialized,
        "message was queued before bridge initialization"
    );
    let snapshot = QueueSnapshot::new(
        bridge_domain,
        source_timestamp_ms,
        true,
        queue_id,
        queue_root.0,
    )?;
    let message_hash = crate::message_hash(&message);
    let inclusion = api
        .fetch_message_inclusion_merkle_proof(block_hash, H256(message_hash))
        .await
        .context("retain message inclusion proof before queue clear")?;
    ensure!(
        inclusion.root == snapshot.queue_root,
        "message inclusion proof root differs from source queue snapshot"
    );
    ensure!(
        inclusion.num_leaves > inclusion.leaf_index,
        "message inclusion index is outside queue"
    );
    let best: H256 = api
        .api
        .rpc()
        .request("chain_getBlockHash", rpc_params![])
        .await?;
    let retained_at_block = api.block_hash_to_number(best).await?;
    ensure!(
        api.fetch_queue_merkle_root(best).await?.0 == snapshot.queue_id,
        "queue cleared before message proof was retained"
    );
    let finalized = timeout(MAX_WAIT, progress.wait_for_finalized())
        .await
        .context("timed out waiting for message finality")??;
    ensure!(
        finalized.block_hash() == block_hash,
        "message reorged after its proof was retained"
    );
    Ok(ObservedMessage {
        block,
        block_hash: block_hash.0,
        message,
        message_hash,
        inclusion,
        snapshot,
        retained_at_block,
    })
}

pub async fn initialize_bridge(api: &GearApi) -> Result<()> {
    let deadline = Instant::now() + MAX_WAIT;
    let initial_hash = api.latest_finalized_block().await?;
    if !bridge_initialized(api, initial_hash).await? {
        let mut cursor = api
            .block_hash_to_number(initial_hash)
            .await?
            .saturating_sub(32);
        let mut observed = false;
        loop {
            let head_hash = api.latest_finalized_block().await?;
            let head = api.block_hash_to_number(head_hash).await?;
            while cursor <= head {
                let hash = api.block_number_to_hash(cursor).await?;
                if has_event(api, hash, "GearEthBridge", "BridgeInitialized").await? {
                    observed = true;
                    cursor = head.saturating_add(1);
                    break;
                }
                cursor = cursor.saturating_add(1);
            }
            if observed && bridge_initialized(api, head_hash).await? {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "timed out waiting for BridgeInitialized"
            );
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    if bridge_paused(api, api.latest_finalized_block().await?).await? {
        let alice =
            sr25519::Pair::from_string("//Alice", None).context("derive Alice sudo signer")?;
        let inner = gsdk::ext::subxt::tx::dynamic("GearEthBridge", "unpause", Vec::<Value>::new())
            .into_value();
        let call = gsdk::ext::subxt::tx::dynamic("Sudo", "sudo", vec![inner]);
        let (_, block_hash) = submit_in_block(api, call, alice).await?;
        ensure!(
            has_event(api, block_hash, "GearEthBridge", "BridgeUnpaused").await?,
            "unpause included without BridgeUnpaused"
        );
        ensure!(
            !bridge_paused(api, block_hash).await?,
            "bridge remains paused after sudo unpause"
        );
    }
    Ok(())
}
async fn submit_watched(
    api: &GearApi,
    call: gsdk::ext::subxt::tx::DynamicPayload,
    pair: sr25519::Pair,
) -> Result<subxt::tx::TxProgress<gsdk::GearConfig, subxt::OnlineClient<gsdk::GearConfig>>> {
    let signer = PairSigner::<gsdk::GearConfig, sr25519::Pair>::new(pair);
    api.api
        .tx()
        .sign_and_submit_then_watch_default(&call, &signer)
        .await
        .map_err(|error| anyhow!("submit dynamic extrinsic: {error:?}"))
}

async fn submit_in_block(
    api: &GearApi,
    call: gsdk::ext::subxt::tx::DynamicPayload,
    pair: sr25519::Pair,
) -> Result<(
    subxt::tx::TxProgress<gsdk::GearConfig, subxt::OnlineClient<gsdk::GearConfig>>,
    H256,
)> {
    use subxt::tx::TxStatus;
    let mut progress = submit_watched(api, call, pair).await?;
    let block_hash = timeout(MAX_WAIT, async {
        loop {
            match progress
                .next()
                .await
                .context("transaction status subscription closed")??
            {
                TxStatus::InBestBlock(block) => {
                    block
                        .wait_for_success()
                        .await
                        .context("included extrinsic failed")?;
                    break Ok::<_, anyhow::Error>(block.block_hash());
                }
                TxStatus::Error { message }
                | TxStatus::Invalid { message }
                | TxStatus::Dropped { message } => bail!("transaction rejected: {message}"),
                TxStatus::InFinalizedBlock(_) => {
                    bail!("transaction finalized before inclusion could be captured")
                }
                TxStatus::NoLongerInBestBlock => {
                    bail!("transaction reorged before inclusion capture")
                }
                _ => {}
            }
        }
    })
    .await
    .context("timed out waiting for transaction inclusion")??;
    Ok((progress, block_hash))
}

pub async fn source_keys(
    source_rpc: &str,
    witness_rpc: &str,
    genesis: &str,
    authority: &str,
    output: &Path,
) -> Result<()> {
    ensure!(
        crate::local_source_rpc(source_rpc)
            && crate::local_source_rpc(witness_rpc)
            && source_rpc != witness_rpc,
        "development keys require independent loopback authorities"
    );
    let (authority, stash_uri) = match authority {
        "alice" => ("Alice", "//Alice//stash"),
        "bob" => ("Bob", "//Bob//stash"),
        other => bail!("unsupported local authority {other}"),
    };
    let signer =
        sr25519::Pair::from_string(stash_uri, None).context("derive local validator stash")?;
    let stash = signer.public().0;
    let genesis = parse_h256(genesis)?;
    fs::create_dir_all(output)?;
    let owner = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(output.join("source-keys.lock"))?;
    owner
        .try_lock()
        .context("another source-key initializer owns this journal")?;
    let source = GearApi::new(source_rpc, 3).await?;
    let witness = GearApi::new(witness_rpc, 3).await?;
    ensure!(
        source.block_number_to_hash(0).await?.0 == genesis
            && witness.block_number_to_hash(0).await?.0 == genesis,
        "source/witness genesis differs from the approved local source; HOLD"
    );
    let source_height = source
        .block_hash_to_number(source.latest_finalized_block().await?)
        .await?;
    let witness_height = witness
        .block_hash_to_number(witness.latest_finalized_block().await?)
        .await?;
    let common_height = source_height.min(witness_height);
    let common_hash = source.block_number_to_hash(common_height).await?;
    ensure!(
        witness.block_number_to_hash(common_height).await? == common_hash,
        "source/witness finalized checkpoint differs; HOLD"
    );
    let runtime = runtime_identity_at(&source, common_hash).await?;
    ensure!(
        runtime_identity_at(&witness, common_hash).await? == runtime,
        "source/witness native registration runtime differs; HOLD"
    );
    let api_id = format!("0x{}", hex::encode(sp_core::blake2_64(b"SessionKeys")));
    ensure!(
        runtime["version"]["apis"]
            .as_array()
            .is_some_and(|apis| apis
                .iter()
                .any(|api| api[0] == api_id && api[1].as_u64() == Some(2))),
        "native owner-bound registration requires finalized SessionKeys API v2; HOLD"
    );
    let path = output.join("rotation.json");
    let temporary = path.with_extension("json.tmp");
    ensure!(!temporary.exists(), "uncommitted original rotation journal exists; preserve and reconcile it before retrying; HOLD");
    let prepared: PreparedRotation = if path.exists() {
        serde_json::from_slice(&fs::read(&path)?).context("original journal is not a complete native key rotation; preserve legacy evidence; HOLD")?
    } else {
        let prepared = prepare_rotation(&source, authority, signer, genesis).await?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, &prepared)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, &path)?;
        File::open(output)?.sync_all()?;
        prepared
    };
    ensure!(
        prepared.authority == authority
            && prepared.stash == stash
            && prepared.source_genesis == genesis,
        "original source-key owner/genesis intent differs; hold saved signed bytes"
    );
    let receipt = if let Some(receipt) = reconcile_rotation(&source, &prepared).await? {
        receipt
    } else {
        submit_prepared_rotation(&source, &prepared).await?
    };
    timeout(MAX_WAIT, async {
        loop {
            let finalized = witness
                .block_hash_to_number(witness.latest_finalized_block().await?)
                .await?;
            if finalized >= receipt.block {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        ensure!(
            witness.block_number_to_hash(receipt.block).await?.0 == receipt.block_hash,
            "witness finalized original rotation hash differs; HOLD"
        );
        rotation_evidence(&witness, &prepared, H256(receipt.block_hash)).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("witness has not finalized the original native registration; HOLD")??;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}

#[cfg(test)]
mod native_registration_tests {
    use super::*;

    #[test]
    fn native_registration_rejects_legacy_and_truncated_key_ownership_material() {
        let mut keys = [1; 161];
        keys[128] = 2;
        for size in [0, 64, 256, 320, 322] {
            assert!(validate_native_keys(&keys, &vec![1; size]).is_err());
        }
        for size in [0, 128, 160, 162] {
            assert!(validate_native_keys(&vec![2; size], &[1; 321]).is_err());
        }
        keys[128] = 4;
        assert!(validate_native_keys(&keys, &[1; 321]).is_err());
    }
}
