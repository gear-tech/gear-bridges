use anyhow::{bail, ensure, Context, Result};
use beefy_relay::{authority_root, encode_queue_proof, Hash32};
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
use subxt_rpcs::rpc_params;
use tiny_keccak::Hasher;
use tokio::process::{Child, Command};

use crate::{
    ethereum::Ethereum,
    source::{self, CapturedCommitment, ObservedMessage, Source, SourceProof},
};

const GEAR_COMMIT: &str = "0b13f2c61b0e5d9844c7efd12727487a2fdb8c63";
const BRIDGE_COMMIT: &str = "73b0bca1dabb874a39f34d59c6d124012a5ad50c";
const SNOWBRIDGE_COMMIT: &str = "1201293e482ef052b9c3989dcf680046704fef3d";
const LIVE_LIMIT: Duration = Duration::from_secs(120);

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
    output: &Path,
    node_data: &Path,
    identity: &str,
    rpc: u16,
    p2p: u16,
    bootnode: Option<&str>,
) -> Result<Child> {
    let log = File::create(output.join(format!("{identity}.log")))?;
    let mut command = Command::new(binary);
    command
        .args([
            "--chain",
            "local",
            "--validator",
            "--force-authoring",
            "--unsafe-force-node-key-generation",
            "--enable-offchain-indexing",
            "true",
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

async fn connect(child: &mut Child, port: u16) -> Result<Source> {
    let url = format!("ws://127.0.0.1:{port}");
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("Gear exited before RPC readiness: {status}");
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Source::connect(&url)
                .await
                .context("connecting indexed source RPC and BEEFY subscription");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn proof_evidence(proof: &SourceProof, anchor: &CapturedCommitment, encoded: &[u8]) -> Value {
    json!({
        "sourceBlock": proof.source, "sourceHash": bytes(proof.source_hash), "insertionBlock": u64::from(proof.source) + 1,
        "anchorBlock": anchor.block, "anchorHash": bytes(anchor.block_hash), "anchorRoot": bytes(anchor.validated.mmr_root),
        "leafIndex": proof.proof.leaf_indices[0], "leafCount": proof.proof.leaf_count,
        "bridgeVersion": proof.snapshot.version, "queueId": proof.snapshot.queue_id, "queueRoot": bytes(proof.snapshot.queue_root),
        "snapshotPreimage": bytes(proof.snapshot.encode()), "outerLeaf": bytes(&proof.raw_leaf),
        "mmrItems": proof.proof.items.iter().map(bytes).collect::<Vec<_>>(),
        "simplifiedItems": proof.simplified.items.iter().map(bytes).collect::<Vec<_>>(),
        "proofOrder": bytes(proof.simplified.proof_order), "queueProof": bytes(encoded),
    })
}

fn message_evidence(message: &ObservedMessage) -> Value {
    json!({
        "sourceBlock": message.block, "sourceHash": bytes(message.block_hash), "messageHash": bytes(message.message_hash),
        "queueId": message.snapshot.queue_id, "queueRoot": bytes(message.snapshot.queue_root),
        "nonce": bytes(message.message.nonce_be), "source": bytes(message.message.source),
        "destination": bytes(message.message.destination), "payload": bytes(&message.message.payload),
        "messageProof": message.inclusion, "retainedAtBlock": message.retained_at_block, "delivered": false,
    })
}

fn queue_envelope(
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
        proof.snapshot.version == 0 && proof.snapshot.queue_root != [0; 32],
        "queue is not initialized and nonempty"
    );
    encode_queue_proof(
        0,
        proof.snapshot.version,
        proof.snapshot.queue_id,
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
    let set_id = anchor.validated.signed.commitment.validator_set_id;
    ensure!(
        set_id == previous.current_id || set_id == previous.next_id,
        "unknown/skipped authority set {set_id}"
    );
    let handover = source
        .proof(
            anchor
                .block
                .checked_sub(1)
                .context("zero commitment height")?,
            &anchor,
        )
        .await?;
    evidence.commitments.push(json!({
        "rawScale": bytes(&anchor.raw), "block": anchor.block, "blockHash": bytes(anchor.block_hash),
        "validatorSetId": set_id, "mmrRoot": bytes(anchor.validated.mmr_root),
        "commitmentBytes": bytes(&anchor.validated.commitment_bytes), "commitmentHash": bytes(anchor.validated.commitment_hash),
        "current": anchor.current, "next": anchor.next, "handoverProof": proof_evidence(&handover, &anchor, &[]),
        "accepted": false,
    }));
    ethereum
        .submit_commitment(
            &anchor.validated,
            &anchor.current.keys,
            &handover.leaf,
            &handover.simplified,
        )
        .await?;
    let accepted = ethereum.checkpoint().await?;
    ensure!(
        accepted.block == u64::from(anchor.block) && accepted.root == anchor.validated.mmr_root,
        "destination anchor does not match receipt"
    );
    ensure!(
        accepted.current_id == anchor.current.id && accepted.current_root == anchor.current.root,
        "destination current authority checkpoint diverged"
    );
    ensure!(
        accepted.next_id == anchor.next.id && accepted.next_root == anchor.next.root,
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
        transitions.push(json!({"from": previous.current_id, "to": accepted.current_id, "root": bytes(accepted.current_root), "block": anchor.block}));
    }
    let recorded = evidence
        .commitments
        .last_mut()
        .expect("captured commitment");
    recorded["accepted"] = json!(true);
    recorded["elapsedSeconds"] = json!(start.elapsed().as_secs_f64());
    eprintln!(
        "[{:.2}s] accepted C={} set={}",
        start.elapsed().as_secs_f64(),
        anchor.block,
        set_id
    );
    Ok(anchor)
}

async fn scenario(
    binary: &Path,
    output: &Path,
    node_data: &Path,
    children: &mut Vec<Child>,
    ethereum: &mut Ethereum,
    evidence: &mut Evidence,
    start: Instant,
) -> Result<()> {
    let reservations: Vec<_> = (0..4)
        .map(|_| TcpListener::bind("127.0.0.1:0"))
        .collect::<std::io::Result<_>>()?;
    let ports: Vec<_> = reservations
        .iter()
        .map(|listener| listener.local_addr().map(|a| a.port()))
        .collect::<std::io::Result<_>>()?;
    drop(reservations);
    evidence.phase("starting both indexed archive authorities", start);
    children.push(spawn_node(
        binary, output, node_data, "Alice", ports[0], ports[1], None,
    )?);
    let mut alice = connect(&mut children[0], ports[0]).await?;
    let peer: String = alice
        .api
        .api
        .rpc()
        .request("system_localPeerId", rpc_params![])
        .await?;
    let bootnode = format!("/ip4/127.0.0.1/tcp/{}/p2p/{peer}", ports[1]);
    children.push(spawn_node(
        binary,
        output,
        node_data,
        "Bob",
        ports[2],
        ports[3],
        Some(&bootnode),
    )?);
    let bob = connect(&mut children[1], ports[2]).await?;
    let alice_genesis: String = alice
        .api
        .api
        .rpc()
        .request("chain_getBlockHash", rpc_params![0])
        .await?;
    let bob_genesis: String = bob
        .api
        .api
        .rpc()
        .request("chain_getBlockHash", rpc_params![0])
        .await?;
    ensure!(
        alice_genesis == bob_genesis,
        "authority genesis hashes differ"
    );
    evidence.manifest["sourceGenesis"] = json!(alice_genesis);
    evidence.manifest["runtime"] = source::validate_runtime(&alice.api).await?;
    evidence.manifest["bobRuntime"] = source::validate_runtime(&bob.api).await?;
    let genesis_keys = keys("//Alice")?;
    let genesis_root = authority_root(&beefy_relay::authority_addresses(&genesis_keys)?)?;
    let genesis_wire_keys: Vec<_> = genesis_keys.iter().map(|key| key.to_vec()).collect();
    for node in [&alice, &bob] {
        let (current, next) = node.checkpoint_at(0).await?;
        ensure!(
            current.id == 0
                && next.id == 1
                && current.keys == genesis_wire_keys
                && next.keys == genesis_wire_keys,
            "genesis authority identities/order differ from trusted local checkpoint"
        );
        ensure!(
            current.root == genesis_root && next.root == genesis_root,
            "genesis authority roots differ"
        );
    }
    evidence.manifest["trustedGenesis"] = json!({"currentId": 0, "nextId": 1, "keys": genesis_keys.iter().map(bytes).collect::<Vec<_>>(), "root": bytes(genesis_root)});
    evidence.phase("registering first real BEEFY key", start);
    let funding = source::fund_stash(&alice.api).await?;
    evidence.manifest["stashFunding"] =
        json!({"extrinsicHash": bytes(funding.0), "blockHash": bytes(funding.1.0)});
    let first_rotation = alice.rotate("Alice", "//Alice//beefy-e2e-1").await?;
    evidence.manifest["firstRotation"] = serde_json::to_value(first_rotation)?;
    let first_root = authority_root(&beefy_relay::authority_addresses(&keys(
        "//Alice//beefy-e2e-1",
    )?)?)?;
    let second_root = authority_root(&beefy_relay::authority_addresses(&keys(
        "//Alice//beefy-e2e-2",
    )?)?)?;
    ensure!(
        genesis_root != first_root && first_root != second_root,
        "development key rotations did not change roots"
    );
    evidence.phase("authenticating first changed authority set", start);
    loop {
        let anchor = advance(&mut alice, ethereum, evidence, start).await?;
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
        tokio::try_join!(alice.rotate("Alice", "//Alice//beefy-e2e-2"), async {
            source::initialize_bridge(&alice.api).await?;
            source::send_message(&alice.api, receiver, b"beefy-e2e-before").await
        })?;
    evidence.manifest["secondRotation"] = serde_json::to_value(second_rotation)?;
    evidence.messages.push(message_evidence(&first));
    evidence.phase("authenticating first queue snapshot", start);
    let first_anchor = loop {
        let anchor = advance(&mut alice, ethereum, evidence, start).await?;
        if anchor.block > first.block {
            break anchor;
        }
    };
    let proof = alice.proof(first.block, &first_anchor).await?;
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
    let second = source::send_message(&alice.api, receiver, b"beefy-e2e-after").await?;
    ensure!(
        second.message.nonce_be != first.message.nonce_be
            && second.snapshot.queue_root != first.snapshot.queue_root,
        "second message did not produce a distinct queue root and nonce"
    );
    evidence.messages.push(message_evidence(&second));
    evidence.phase("authenticating second changed authority set", start);
    let second_anchor = loop {
        let anchor = advance(&mut alice, ethereum, evidence, start).await?;
        if anchor.current.root == second_root && anchor.block > second.block {
            break anchor;
        }
    };
    let old_proof = alice.proof(second.block, &second_anchor).await?;
    let old_envelope = queue_envelope(&old_proof, &second_anchor, &second)?;
    evidence.messages[1]["staleQueueInclusion"] =
        proof_evidence(&old_proof, &second_anchor, &old_envelope);
    evidence.phase(
        "advancing accepted anchor to exercise stale-proof race",
        start,
    );
    let fresh_anchor = advance(&mut alice, ethereum, evidence, start).await?;
    ensure!(
        fresh_anchor.current.root == second_root,
        "second authority root did not remain authenticated"
    );
    ethereum
        .reject_registration(second.block, second.snapshot.queue_root, old_envelope)
        .await?;
    evidence.messages[1]["staleAnchorRejected"] = json!(true);
    let fresh_proof = alice.proof(second.block, &fresh_anchor).await?;
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
    let finalized = alice.api.latest_finalized_block().await?;
    let (latest_queue_id, _) = alice.api.fetch_queue_merkle_root(finalized).await?;
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
    for child in children {
        ensure!(
            child.try_wait()?.is_none(),
            "an authority exited during the rehearsal"
        );
    }
    ensure!(
        start.elapsed() < LIVE_LIMIT,
        "live rehearsal exceeded 120 seconds"
    );
    evidence.phase("complete", start);
    Ok(())
}

pub async fn run(binary: &Path, output: &Path) -> Result<()> {
    ensure!(
        binary.is_absolute() && binary.is_file(),
        "--gear-node must be an absolute existing binary path"
    );
    fs::create_dir(output)
        .context("--output-dir must be a new directory under an existing parent")?;
    let output = output.canonicalize()?;
    let mut evidence = Evidence {
        manifest: json!({"schemaVersion": 1, "gearCommit": GEAR_COMMIT, "bridgeCommit": BRIDGE_COMMIT,
            "snowbridgeCommit": SNOWBRIDGE_COMMIT, "status": "preparing", "phase": "preparation",
            "nodeCodeHash": bytes(node_hash(binary)?), "hashAlgorithm": "keccak256", "mmrStartBlock": 1,
            "beefyActivationBlock": 1, "destinationChainId": 31337, "authorityTransitions": []}),
        commitments: Vec::new(),
        messages: Vec::new(),
    };
    evidence.save(&output, &[])?;
    let mut ethereum = match Ethereum::prepare(
        &output,
        authority_root(&beefy_relay::authority_addresses(&keys("//Alice")?)?)?,
    )
    .await
    {
        Ok(ethereum) => ethereum,
        Err(error) => {
            evidence.manifest["status"] = json!("failed");
            evidence.manifest["error"] = json!(format!("{error:#}"));
            evidence.save(&output, &[])?;
            return Err(error);
        }
    };
    match ethereum.manifest().await {
        Ok(manifest) => evidence.manifest["ethereum"] = manifest,
        Err(error) => {
            evidence.manifest["status"] = json!("failed");
            evidence.manifest["error"] = json!(format!("{error:#}"));
            evidence.save(&output, &ethereum.transactions)?;
            return Err(error);
        }
    }
    evidence.manifest["status"] = json!("running");
    let node_data = tempfile::tempdir().context("create private temporary node data")?;
    let mut children = Vec::with_capacity(2);
    let start = Instant::now();
    let mut result = tokio::time::timeout(
        LIVE_LIMIT,
        scenario(
            binary,
            &output,
            node_data.path(),
            &mut children,
            &mut ethereum,
            &mut evidence,
            start,
        ),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "120-second live deadline reached during {}",
            evidence.manifest["phase"]
        )
    })
    .and_then(|result| result);
    evidence.manifest["elapsedSeconds"] = json!(start.elapsed().as_secs_f64());
    for child in &mut children {
        let stopped: Result<()> = async {
            if child.try_wait()?.is_none() {
                child.kill().await?;
            }
            child.wait().await?;
            Ok(())
        }
        .await;
        if let Err(error) = stopped {
            evidence.manifest["cleanupError"] = json!(format!("{error:#}"));
            if result.is_ok() {
                result = Err(error).context("stopping owned Gear child");
            }
        }
    }
    evidence.manifest["status"] = json!(if result.is_ok() { "passed" } else { "failed" });
    if let Err(error) = &result {
        evidence.manifest["error"] = json!(format!("{error:#}"));
    }
    evidence.save(&output, &ethereum.transactions)?;
    result
}
