use anyhow::{bail, ensure, Context, Result};
use beefy_relay::{authority_root, encode_queue_proof, outer_leaf_hash, Hash32};
use gear_rpc_client::GearApi;
use serde_json::{json, Value};
use sp_core::Pair;
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use subxt::utils::H256;
use subxt_rpcs::rpc_params;
use tiny_keccak::Hasher;
use tokio::process::{Child, Command};

use crate::{
    ethereum::Ethereum,
    source::{self, CapturedCommitment, ObservedMessage, Source, SourceProof},
};

const GEAR_BASELINE_COMMIT: &str = "0b13f2c61b0e5d9844c7efd12727487a2fdb8c63";
const BRIDGE_BASELINE_COMMIT: &str = "73b0bca1dabb874a39f34d59c6d124012a5ad50c";
const SNOWBRIDGE_BASELINE_COMMIT: &str = "1201293e482ef052b9c3989dcf680046704fef3d";
const PREPARATION_LIMIT: Duration = Duration::from_secs(900);
const LIVE_LIMIT: Duration = Duration::from_secs(1_200);

fn bytes(bytes: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn node_hash(path: &Path) -> Result<Hash32> {
    let mut file = File::open(path)?;
    let mut hasher = tiny_keccak::Keccak::v256();
    let mut buffer = [0; 65536];
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        hasher.update(&buffer[..len]);
    }
    let mut hash = [0; 32];
    hasher.finalize(&mut hash);
    Ok(hash)
}

fn keys(alice: &str) -> Result<Vec<[u8; 33]>> {
    [alice, "//Bob"]
        .iter()
        .map(|uri| {
            sp_core::ecdsa::Pair::from_string(uri, None)
                .map(|pair| pair.public().0)
                .map_err(|error| anyhow::anyhow!("invalid fixed development derivation: {error:?}"))
        })
        .collect()
}

struct Evidence {
    manifest: Value,
    commitments: Vec<Value>,
    messages: Vec<Value>,
}

impl Evidence {
    fn phase(&mut self, phase: &str, start: Instant) {
        self.manifest["phase"] = json!(phase);
        self.manifest["elapsedSeconds"] = json!(start.elapsed().as_secs_f64());
        eprintln!("[{:.2}s] {phase}", start.elapsed().as_secs_f64());
    }

    fn save(&self, output: &Path, transactions: &[Value]) -> Result<()> {
        fs::write(
            output.join("manifest.json"),
            serde_json::to_vec_pretty(&self.manifest)?,
        )?;
        fs::write(
            output.join("messages.json"),
            serde_json::to_vec_pretty(&self.messages)?,
        )?;
        fs::write(
            output.join("transactions.json"),
            serde_json::to_vec_pretty(transactions)?,
        )?;
        let mut history = File::create(output.join("commitments.jsonl"))?;
        for commitment in &self.commitments {
            serde_json::to_writer(&mut history, commitment)?;
            history.write_all(b"\n")?;
        }
        Ok(())
    }
}

fn spawn_node(
    binary: &Path,
    chain_spec: &Path,
    output: &Path,
    node_data: &Path,
    identity: &str,
    ports: (u16, u16),
    bootnode: Option<&str>,
) -> Result<Child> {
    let (rpc, p2p) = ports;
    let log = File::create(output.join(format!("{identity}.log")))?;
    let mut command = Command::new(binary);
    command
        .args([
            "--chain",
            chain_spec
                .to_str()
                .context("chain spec path must be UTF-8")?,
            "--validator",
            "--force-authoring",
            "--unsafe-force-node-key-generation",
            "--enable-offchain-indexing",
            "true",
            "--offchain-worker",
            "always",
            "--state-pruning",
            "archive",
            "--blocks-pruning",
            "archive",
            "--rpc-methods",
            "unsafe",
            "--no-mdns",
            "--no-telemetry",
            "--no-prometheus",
        ])
        .arg(format!("--{}", identity.to_ascii_lowercase()))
        .arg("--base-path")
        .arg(node_data.join(identity))
        .arg("--rpc-port")
        .arg(rpc.to_string())
        .arg("--listen-addr")
        .arg(format!("/ip4/127.0.0.1/tcp/{p2p}"))
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .kill_on_drop(true);
    if let Some(bootnode) = bootnode {
        command.arg("--bootnodes").arg(bootnode);
    }
    command
        .spawn()
        .with_context(|| format!("starting owned {identity} node"))
}

async fn connect_rpc(child: &mut Child, port: u16) -> Result<GearApi> {
    let url = format!("ws://127.0.0.1:{port}");
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("Gear exited before RPC readiness: {status}");
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return GearApi::new(&url, 3)
                .await
                .context("connecting indexed source RPC");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) fn proof_evidence(
    proof: &SourceProof,
    anchor: &CapturedCommitment,
    encoded: &[u8],
) -> Value {
    json!({
        "sourceBlock": proof.source,
        "sourceHash": bytes(proof.source_hash),
        "insertionBlock": u64::from(proof.source) + 1,
        "anchorBlock": anchor.block,
        "anchorHash": bytes(anchor.block_hash),
        "anchorRoot": bytes(anchor.validated.mmr_root),
    "bridgeDomain": bytes(proof.snapshot.bridge_domain),
        "sourceTimestampMs": proof.snapshot.source_timestamp_ms,
        "initialized": proof.snapshot.initialized,
        "queueId": proof.snapshot.queue_id,
        "queueRoot": bytes(proof.snapshot.queue_root),
        "snapshotPreimage": bytes(proof.snapshot.encode()),
        "outerLeaf": bytes(&proof.raw_leaf),
        "outerLeafHash": bytes(outer_leaf_hash(&proof.leaf).expect("source leaf was validated")),
        "leafIndex": proof.proof.leaf_indices[0],
        "leafCount": proof.proof.leaf_count,
        "mmrItems": proof.proof.items.iter().map(bytes).collect::<Vec<_>>(),
        "simplifiedItems": proof.simplified.items.iter().map(bytes).collect::<Vec<_>>(),
        "proofOrder": bytes(proof.simplified.proof_order),
        "queueProof": bytes(encoded),
    })
}

pub(crate) fn freshness_evidence(proof: &SourceProof) -> Result<Value> {
    Ok(json!({
    "bridgeDomain": bytes(proof.snapshot.bridge_domain),
        "sourceTimestampMs": proof.snapshot.source_timestamp_ms,
        "initialized": proof.snapshot.initialized,
        "queueId": proof.snapshot.queue_id,
        "queueRoot": bytes(proof.snapshot.queue_root),
        "snapshotPreimage": bytes(proof.snapshot.encode()),
        "bridgeCommitment": bytes(proof.snapshot.hash()),
        "outerLeaf": bytes(&proof.raw_leaf),
        "outerLeafHash": bytes(outer_leaf_hash(&proof.leaf)?),
        "leafIndex": proof.proof.leaf_indices[0],
        "leafCount": proof.proof.leaf_count,
        "mmrItems": proof.proof.items.iter().map(bytes).collect::<Vec<_>>(),
        "simplifiedItems": proof.simplified.items.iter().map(bytes).collect::<Vec<_>>(),
        "proofOrder": bytes(proof.simplified.proof_order),
    }))
}

pub(crate) fn message_evidence(message: &ObservedMessage) -> Value {
    json!({
        "sourceBlock": message.block,
        "sourceHash": bytes(message.block_hash),
        "messageHash": bytes(message.message_hash),
    "bridgeDomain": bytes(message.snapshot.bridge_domain),
        "sourceTimestampMs": message.snapshot.source_timestamp_ms,
        "initialized": message.snapshot.initialized,
        "queueId": message.snapshot.queue_id,
        "queueRoot": bytes(message.snapshot.queue_root),
        "nonce": bytes(message.message.nonce_be),
        "source": bytes(message.message.source),
        "destination": bytes(message.message.destination),
        "payload": bytes(&message.message.payload),
        "messageProof": message.inclusion,
        "retainedAtBlock": message.retained_at_block,
        "delivered": false,
    })
}

pub(crate) fn queue_envelope(
    proof: &SourceProof,
    anchor: &CapturedCommitment,
    message: &ObservedMessage,
) -> Result<Vec<u8>> {
    ensure!(
        proof.source == message.block && proof.source_hash == message.block_hash,
        "message source identity changed"
    );
    ensure!(
        proof.snapshot == message.snapshot && proof.leaf.leaf_extra == message.snapshot.hash(),
        "MMR leaf does not authenticate the historical message queue snapshot"
    );
    ensure!(
        proof.snapshot.initialized && proof.snapshot.queue_root != [0; 32],
        "queue is not initialized and nonempty"
    );
    encode_queue_proof(
        &proof.snapshot,
        u64::from(anchor.block),
        anchor.validated.mmr_root,
        &proof.leaf,
        &proof.simplified.items,
        proof.simplified.proof_order,
    )
}

async fn advance(
    source: &mut Source,
    ethereum: &mut Ethereum,
    evidence: &mut Evidence,
    start: Instant,
) -> Result<CapturedCommitment> {
    let anchor = source
        .next_commitment()
        .await
        .context("capturing next complete signed BEEFY commitment")?;
    let previous = ethereum.checkpoint().await?;
    ensure!(
        u64::from(anchor.block) > previous.block,
        "source history is not strictly increasing"
    );
    ensure!(
        anchor.finalized_height >= anchor.block,
        "source commitment was not finalized"
    );
    let set_id = anchor.validated.signed.commitment.validator_set_id;
    ensure!(
        set_id == previous.current_id || set_id == previous.next_id,
        "unknown/skipped authority set {set_id}"
    );
    let freshness = source
        .proof(
            anchor
                .block
                .checked_sub(1)
                .context("zero commitment height")?,
            &anchor,
        )
        .await?;
    ensure!(
        freshness.snapshot.bridge_domain == source.bridge_domain
            && freshness.snapshot.source_timestamp_ms >= previous.source_timestamp_ms,
        "freshness metadata regressed or changed source identity"
    );
    let freshness_json = freshness_evidence(&freshness)?;
    evidence.commitments.push(json!({
        "rawScale": bytes(&anchor.raw),
        "block": anchor.block,
        "blockHash": bytes(anchor.block_hash),
        "finalizedHeight": anchor.finalized_height,
        "validatorSetId": set_id,
        "mmrRoot": bytes(anchor.validated.mmr_root),
        "commitmentBytes": bytes(&anchor.validated.commitment_bytes),
        "commitmentHash": bytes(anchor.validated.commitment_hash),
        "current": anchor.current,
        "next": anchor.next,
        "freshnessProof": freshness_json,
        "accepted": false,
    }));
    ethereum
        .submit_commitment(
            &anchor,
            &freshness.leaf,
            &freshness.snapshot,
            &freshness.simplified,
            None,
            |_, _| Ok(()),
        )
        .await?;
    let accepted = ethereum.checkpoint().await?;
    ensure!(
        accepted.block == u64::from(anchor.block)
            && accepted.root == anchor.validated.mmr_root
            && accepted.root != [0; 32],
        "destination anchor does not match receipt or has a zero root"
    );
    ensure!(
        accepted.source_timestamp_ms == freshness.snapshot.source_timestamp_ms,
        "destination accepted timestamp differs from freshness witness"
    );
    ensure!(
        accepted.current_id == anchor.current.id
            && accepted.current_len == anchor.current.keys.len() as u64
            && accepted.current_root == anchor.current.root,
        "destination current authority checkpoint diverged"
    );
    ensure!(
        accepted.next_id == anchor.next.id
            && accepted.next_len == anchor.next.keys.len() as u64
            && accepted.next_root == anchor.next.root,
        "destination next authority checkpoint diverged"
    );
    if set_id != previous.current_id {
        ensure!(
            accepted.current_id == previous.current_id + 1,
            "non-successive authority transition"
        );
        let transitions = evidence.manifest["authorityTransitions"]
            .as_array_mut()
            .expect("initialized transition evidence");
        transitions.push(json!({
            "from": previous.current_id,
            "to": accepted.current_id,
            "root": bytes(accepted.current_root),
            "block": anchor.block,
        }));
    }
    let recorded = evidence
        .commitments
        .last_mut()
        .expect("captured commitment");
    recorded["accepted"] = json!(true);
    recorded["acceptedSourceTimestampMs"] = json!(accepted.source_timestamp_ms);
    recorded["elapsedSeconds"] = json!(start.elapsed().as_secs_f64());
    eprintln!(
        "[{:.2}s] accepted C={} set={} sourceTimestampMs={}",
        start.elapsed().as_secs_f64(),
        anchor.block,
        set_id,
        accepted.source_timestamp_ms
    );
    Ok(anchor)
}

async fn prepare_phase(
    binary: &Path,
    chain_spec: &Path,
    output: &Path,
    node_data: &Path,
    children: &mut Vec<Child>,
    evidence: &mut Evidence,
) -> Result<(Source, Ethereum)> {
    let reservations: Vec<_> = (0..4)
        .map(|_| TcpListener::bind("127.0.0.1:0"))
        .collect::<std::io::Result<_>>()?;
    let ports: Vec<_> = reservations
        .iter()
        .map(|listener| listener.local_addr().map(|a| a.port()))
        .collect::<std::io::Result<_>>()?;
    drop(reservations);
    evidence.phase("starting both indexed archive authorities", Instant::now());
    children.push(spawn_node(
        binary,
        chain_spec,
        output,
        node_data,
        "Alice",
        (ports[0], ports[1]),
        None,
    )?);
    let alice_api = connect_rpc(&mut children[0], ports[0]).await?;
    let peer: String = alice_api
        .api
        .rpc()
        .request("system_localPeerId", rpc_params![])
        .await?;
    let bootnode = format!("/ip4/127.0.0.1/tcp/{}/p2p/{peer}", ports[1]);
    children.push(spawn_node(
        binary,
        chain_spec,
        output,
        node_data,
        "Bob",
        (ports[2], ports[3]),
        Some(&bootnode),
    )?);
    let bob_api = connect_rpc(&mut children[1], ports[2]).await?;
    let (mut alice, bob) = Source::connect_pair(alice_api, bob_api).await?;
    ensure!(
        alice.source_genesis == bob.source_genesis
            && alice.bridge_domain == bob.bridge_domain
            && alice.mmr_start_block == bob.mmr_start_block
            && alice.beefy_activation_block == bob.beefy_activation_block,
        "archive authorities disagree on source identity or activation"
    );
    evidence.manifest["sourceGenesis"] = json!(bytes(alice.source_genesis));
    evidence.manifest["bridgeDomain"] = json!(bytes(alice.bridge_domain));
    evidence.manifest["mmrStartBlock"] = json!(alice.mmr_start_block);
    evidence.manifest["beefyActivationBlock"] = json!(alice.beefy_activation_block);
    evidence.manifest["runtime"] = source::validate_legacy_runtime(&alice.api).await?;
    evidence.manifest["bobRuntime"] = source::validate_legacy_runtime(&bob.api).await?;

    let (bootstrap, bootstrap_freshness) = loop {
        let candidate = alice
            .next_commitment()
            .await
            .context("capturing finalized bootstrap commitment")?;
        ensure!(
            candidate.finalized_height >= candidate.block,
            "bootstrap commitment is not finalized"
        );
        if u64::from(candidate.block) <= alice.mmr_start_block {
            continue;
        }
        let proof = alice
            .proof(
                candidate
                    .block
                    .checked_sub(1)
                    .context("bootstrap commitment at zero")?,
                &candidate,
            )
            .await?;
        if proof.snapshot.source_timestamp_ms == 0 {
            continue;
        }
        break (candidate, proof);
    };
    ensure!(
        bootstrap_freshness.snapshot.bridge_domain == alice.bridge_domain
            && u64::from(bootstrap_freshness.source) + 1 == u64::from(bootstrap.block),
        "bootstrap freshness witness does not bind source identity and parent block"
    );
    let anchor_hash = H256(bootstrap.block_hash);
    let (bob_current, bob_next) = bob.checkpoint_at_hash(anchor_hash).await?;
    let bob_freshness = bob
        .proof(
            bootstrap
                .block
                .checked_sub(1)
                .context("bootstrap commitment at zero")?,
            &bootstrap,
        )
        .await?;
    let bob_timestamp = bob
        .api
        .fetch_timestamp(H256(bob_freshness.source_hash))
        .await?;
    ensure!(
        bob_timestamp == bootstrap_freshness.snapshot.source_timestamp_ms
            && bob_current == bootstrap.current
            && bob_next == bootstrap.next
            && bob_freshness.snapshot == bootstrap_freshness.snapshot
            && bob_freshness.raw_leaf == bootstrap_freshness.raw_leaf,
        "second archive authority disagrees at the finalized bootstrap checkpoint"
    );
    let bootstrap_witness = freshness_evidence(&bootstrap_freshness)?;
    evidence.manifest["bootstrap"] = json!({
        "block": bootstrap.block,
        "blockHash": bytes(bootstrap.block_hash),
        "finalizedHeight": bootstrap.finalized_height,
        "mmrRoot": bytes(bootstrap.validated.mmr_root),
        "sourceTimestampMs": bootstrap_freshness.snapshot.source_timestamp_ms,
        "current": bootstrap.current,
        "next": bootstrap.next,
        "freshnessProof": bootstrap_witness,
    });
    evidence.phase(
        "deploying immutable destination-bound v2 contracts",
        Instant::now(),
    );
    let ethereum = Ethereum::prepare(
        output,
        &bootstrap,
        &bootstrap_freshness.snapshot,
        alice.mmr_start_block,
    )
    .await?;
    Ok((alice, ethereum))
}

async fn scenario(
    source: &mut Source,
    children: &mut [Child],
    ethereum: &mut Ethereum,
    evidence: &mut Evidence,
    start: Instant,
) -> Result<()> {
    let baseline = ethereum.checkpoint().await?;
    let first_root = authority_root(&beefy_relay::authority_addresses(&keys(
        "//Alice//beefy-e2e-1",
    )?)?)?;
    let second_root = authority_root(&beefy_relay::authority_addresses(&keys(
        "//Alice//beefy-e2e-2",
    )?)?)?;
    ensure!(
        baseline.current_root != first_root && first_root != second_root,
        "development key rotations did not change roots"
    );
    evidence.phase("registering first real BEEFY key", start);
    let funding = source::fund_stash(&source.api).await?;
    evidence.manifest["stashFunding"] =
        json!({"extrinsicHash": bytes(funding.0), "blockHash": bytes(funding.1.0)});
    let first_rotation = source.rotate("Alice", "//Alice//beefy-e2e-1").await?;
    evidence.manifest["firstRotation"] = serde_json::to_value(first_rotation)?;
    evidence.phase("authenticating first changed authority set", start);
    loop {
        let anchor = advance(source, ethereum, evidence, start).await?;
        if anchor.current.root == first_root {
            break;
        }
    }
    evidence.phase(
        "scheduling second rotation and enqueuing first message",
        start,
    );
    let receiver = ethereum.receiver_address();
    let (second_rotation, first) =
        tokio::try_join!(source.rotate("Alice", "//Alice//beefy-e2e-2"), async {
            source::initialize_bridge(&source.api).await?;
            source::send_message(&source.api, receiver, b"beefy-e2e-before", "//Charlie").await
        })?;
    evidence.manifest["secondRotation"] = serde_json::to_value(second_rotation)?;
    evidence.messages.push(message_evidence(&first));
    evidence.phase("authenticating first queue snapshot", start);
    let first_anchor = loop {
        let anchor = advance(source, ethereum, evidence, start).await?;
        if anchor.block > first.block {
            break anchor;
        }
    };
    let proof = source.proof(first.block, &first_anchor).await?;
    let envelope = queue_envelope(&proof, &first_anchor, &first)?;
    evidence.messages[0]["queueInclusion"] = proof_evidence(&proof, &first_anchor, &envelope);
    ethereum
        .register(first.block, first.snapshot.queue_root, envelope)
        .await?;
    ethereum
        .reject_early(first.block, &first.message, &first.inclusion)
        .await?;
    evidence.messages[0]["earlyDeliveryRejected"] = json!(true);
    ethereum.advance_time().await?;
    ethereum
        .deliver(first.block, &first.message, &first.inclusion)
        .await?;
    ethereum
        .reject_replay(first.block, &first.message, &first.inclusion)
        .await?;
    evidence.messages[0]["replayRejected"] = json!(true);
    evidence.messages[0]["delivered"] = json!(true);
    evidence.messages[0]["deliverySeconds"] = json!(start.elapsed().as_secs_f64());
    evidence.phase("retaining second message before natural queue clear", start);
    let second =
        source::send_message(&source.api, receiver, b"beefy-e2e-after", "//Charlie").await?;
    ensure!(
        second.message.nonce_be != first.message.nonce_be
            && second.snapshot.queue_root != first.snapshot.queue_root,
        "second message did not produce a distinct queue root and nonce"
    );
    evidence.messages.push(message_evidence(&second));
    evidence.phase("authenticating second changed authority set", start);
    let second_anchor = loop {
        let anchor = advance(source, ethereum, evidence, start).await?;
        if anchor.current.root == second_root && anchor.block > second.block {
            break anchor;
        }
    };
    let old_proof = source.proof(second.block, &second_anchor).await?;
    let old_envelope = queue_envelope(&old_proof, &second_anchor, &second)?;
    evidence.messages[1]["staleQueueInclusion"] =
        proof_evidence(&old_proof, &second_anchor, &old_envelope);
    evidence.phase(
        "advancing accepted anchor to exercise stale-proof race",
        start,
    );
    let fresh_anchor = advance(source, ethereum, evidence, start).await?;
    ensure!(
        fresh_anchor.current.root == second_root,
        "second authority root did not remain authenticated"
    );
    ethereum
        .reject_registration(second.block, second.snapshot.queue_root, old_envelope)
        .await?;
    evidence.messages[1]["staleAnchorRejected"] = json!(true);
    let fresh_proof = source.proof(second.block, &fresh_anchor).await?;
    let fresh_envelope = queue_envelope(&fresh_proof, &fresh_anchor, &second)?;
    evidence.messages[1]["queueInclusion"] =
        proof_evidence(&fresh_proof, &fresh_anchor, &fresh_envelope);
    ethereum
        .register(second.block, second.snapshot.queue_root, fresh_envelope)
        .await?;
    ethereum.advance_time().await?;
    ethereum
        .deliver(second.block, &second.message, &second.inclusion)
        .await?;
    ethereum
        .reject_replay(second.block, &second.message, &second.inclusion)
        .await?;
    evidence.messages[1]["replayRejected"] = json!(true);
    evidence.messages[1]["delivered"] = json!(true);
    evidence.messages[1]["deliverySeconds"] = json!(start.elapsed().as_secs_f64());
    let finalized = source.api.latest_finalized_block().await?;
    let (latest_queue_id, _) = source.api.fetch_queue_merkle_root(finalized).await?;
    ensure!(
        latest_queue_id > second.snapshot.queue_id,
        "retained message was not delivered across a natural queue clear"
    );
    evidence.manifest["queueIdAfterClear"] = json!(latest_queue_id);
    ensure!(
        evidence.manifest["authorityTransitions"]
            .as_array()
            .expect("initialized transitions")
            .len()
            >= 2,
        "fewer than two authenticated successive authority transitions"
    );
    for child in children.iter_mut() {
        ensure!(
            child.try_wait()?.is_none(),
            "an authority exited during the rehearsal"
        );
    }
    ensure!(
        start.elapsed() < LIVE_LIMIT,
        "live rehearsal exceeded 1200 seconds"
    );
    evidence.phase("complete", start);
    Ok(())
}

async fn stop_children(children: &mut [Child]) -> Result<()> {
    let mut first_error = None;
    for child in children {
        let stopped: Result<()> = async {
            if child.try_wait()?.is_none() {
                child.kill().await?;
            }
            child.wait().await?;
            Ok(())
        }
        .await;
        if let Err(error) = stopped {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error).context("stopping owned Gear child");
    }
    Ok(())
}

pub async fn run(binary: &Path, chain_spec: &Path, output: &Path) -> Result<()> {
    ensure!(
        binary.is_absolute() && binary.is_file(),
        "--gear-node must be an absolute existing binary path"
    );
    ensure!(
        chain_spec.is_absolute() && chain_spec.is_file(),
        "--chain-spec must be an absolute existing raw chain spec"
    );
    fs::create_dir(output)
        .context("--output-dir must be a new directory under an existing parent")?;
    let output = output.canonicalize()?;
    let mut evidence = Evidence {
        manifest: json!({
            "schemaVersion": 2,
            "gearBaselineCommit": GEAR_BASELINE_COMMIT,
            "bridgeBaselineCommit": BRIDGE_BASELINE_COMMIT,
            "chainSpecHash": bytes(node_hash(chain_spec)?),
            "snowbridgeBaselineCommit": SNOWBRIDGE_BASELINE_COMMIT,
            "status": "preparing",
            "phase": "preparation",
            "nodeCodeHash": bytes(node_hash(binary)?),
            "hashAlgorithm": "keccak256",
            "destinationChainId": 31337,
            "authorityTransitions": [],
        }),
        commitments: Vec::new(),
        messages: Vec::new(),
    };
    evidence.save(&output, &[])?;
    let node_data = tempfile::tempdir().context("create private temporary node data")?;
    let mut children = Vec::with_capacity(2);
    let preparation_start = Instant::now();
    let preparation = tokio::time::timeout(
        PREPARATION_LIMIT,
        prepare_phase(
            binary,
            chain_spec,
            &output,
            node_data.path(),
            &mut children,
            &mut evidence,
        ),
    )
    .await
    .map_err(|_| anyhow::anyhow!("15-minute preparation deadline reached"))
    .and_then(|result| result);
    let (mut source, mut ethereum) = match preparation {
        Ok(value) => value,
        Err(error) => {
            evidence.manifest["status"] = json!("failed");
            evidence.manifest["error"] = json!(format!("{error:#}"));
            evidence.manifest["preparationElapsedSeconds"] =
                json!(preparation_start.elapsed().as_secs_f64());
            let _ = stop_children(&mut children).await;
            evidence.save(&output, &[])?;
            return Err(error);
        }
    };
    evidence.manifest["preparationElapsedSeconds"] =
        json!(preparation_start.elapsed().as_secs_f64());
    match ethereum.manifest().await {
        Ok(manifest) => evidence.manifest["ethereum"] = manifest,
        Err(error) => {
            evidence.manifest["status"] = json!("failed");
            evidence.manifest["error"] = json!(format!("{error:#}"));
            let _ = stop_children(&mut children).await;
            evidence.save(&output, &ethereum.transactions)?;
            return Err(error);
        }
    }
    evidence.manifest["status"] = json!("running");
    let live_start = Instant::now();
    let mut result = tokio::time::timeout(
        LIVE_LIMIT,
        scenario(
            &mut source,
            &mut children,
            &mut ethereum,
            &mut evidence,
            live_start,
        ),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "20-minute live deadline reached during {}",
            evidence.manifest["phase"]
        )
    })
    .and_then(|result| result);
    evidence.manifest["liveElapsedSeconds"] = json!(live_start.elapsed().as_secs_f64());
    if let Err(error) = stop_children(&mut children).await {
        evidence.manifest["cleanupError"] = json!(format!("{error:#}"));
        if result.is_ok() {
            result = Err(error);
        }
    }
    evidence.manifest["status"] = json!(if result.is_ok() { "passed" } else { "failed" });
    if let Err(error) = &result {
        evidence.manifest["error"] = json!(format!("{error:#}"));
    }
    evidence.save(&output, &ethereum.transactions)?;
    result
}
