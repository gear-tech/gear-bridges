use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use url::{Host, Url};

mod ethereum;
mod hoodi;
mod rehearsal;
mod source;
mod tokens;
mod tokens_soak;

#[derive(Parser)]
#[command(about = "Prover-free Vara BEEFY bridge demonstrations")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Rehearse {
        #[arg(long)]
        gear_node: PathBuf,
        #[arg(long)]
        chain_spec: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Persistent local authorities to real Hoodi; rerun the same output directory to resume.
    Hoodi {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
        #[arg(long, default_value = "wss://ethereum-hoodi-rpc.publicnode.com")]
        ethereum_rpc: String,
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
        #[arg(long, conflicts_with = "follow")]
        prepare_only: bool,
        #[arg(long)]
        follow: bool,
    },
    /// Prepare the isolated Gear token applications; never reuse message-demo programs.
    Tokens {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long)]
        expected_genesis: String,
        #[arg(long)]
        gear_suri_file: PathBuf,
        #[arg(long)]
        checkpoint: String,
        #[arg(long)]
        checkpoint_slot: u64,
        #[arg(long)]
        checkpoint_hash: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Configure fresh Gear token roles and mappings against a pinned real-Hoodi deployment.
    TokensConfigure {
        #[arg(long, default_value = "ws://127.0.0.1:9948")]
        source_rpc: String,
        #[arg(long)]
        witness_rpc: String,
        #[arg(long)]
        expected_genesis: String,
        #[arg(long)]
        gear_suri_file: PathBuf,
        #[arg(long, default_value = "wss://ethereum-hoodi-rpc.publicnode.com")]
        ethereum_rpc: String,
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        token_stack: PathBuf,
    },
    /// Provision ordinary Gear VFT and value-backed native fixtures on a fresh admitted local source.
    TokensProvisionSourceInventory {
        #[arg(long)]
        source_rpc: String,
        #[arg(long)]
        witness_rpc: String,
        #[arg(long)]
        expected_genesis: String,
        #[arg(long)]
        gear_suri_file: PathBuf,
        #[arg(long)]
        campaign_suri_file: PathBuf,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        token_stack: PathBuf,
    },
    /// Write pinned cross-chain account balances without signing or changing bridge state.
    TokensSnapshot {
        #[arg(long)]
        source_rpc: String,
        #[arg(long)]
        witness_rpc: String,
        #[arg(long)]
        ethereum_rpc: String,
        #[arg(long)]
        beacon_rpc: String,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        token_stack: PathBuf,
        #[arg(long)]
        gear_user: String,
        #[arg(long)]
        evm_user: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Read an independently witnessed, finalized Gear BEEFY bootstrap without sending transactions.
    TokensAnchor {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
    },
    /// Write a one-shot, read-only readiness record for two local Gear authorities.
    TokensSourceState {
        #[arg(long)]
        raw_spec: PathBuf,
        #[arg(long)]
        source_rpc: String,
        #[arg(long)]
        witness_rpc: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Pin deployed Hoodi token contracts after checking a signed Gear bootstrap on two authorities.
    TokensManifest {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
        #[arg(long, default_value = "wss://ethereum-hoodi-rpc.publicnode.com")]
        ethereum_rpc: String,
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long)]
        anchor: PathBuf,
        #[arg(long)]
        token_stack: PathBuf,
        #[arg(long)]
        client: String,
        #[arg(long)]
        verifier: String,
        #[arg(long)]
        queue: String,
        #[arg(long)]
        manager: String,
        #[arg(long)]
        output: PathBuf,
        /// Test-only: pin a local Hoodi-fork deployment, never qualification evidence.
        #[arg(long)]
        local_rehearsal: bool,
    },
    /// One-shot root-publication maintenance; run only while the integrated follower is stopped.
    TokensPublishRoot {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
        #[arg(long, default_value = "wss://ethereum-hoodi-rpc.publicnode.com")]
        ethereum_rpc: String,
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        publisher_state: PathBuf,
        #[arg(long)]
        registration: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify a finalized, authorized recovery activation and persist its immutable client cutover.
    TokensRecoveryActivate {
        #[arg(long, default_value = "wss://ethereum-hoodi-rpc.publicnode.com")]
        ethereum_rpc: String,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        follower_dir: PathBuf,
        #[arg(long)]
        recovery_plan: PathBuf,
    },
    /// Authenticate every saved finalized commitment without changing actor journals or submitting transactions.
    TokensHistoryAudit {
        #[arg(long)]
        source_rpc: String,
        #[arg(long)]
        witness_rpc: String,
        #[arg(long)]
        ethereum_rpc: String,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        follower_state: PathBuf,
        /// Owned durable header and original-inclusion proof material, not an acceptance cache.
        #[arg(long)]
        proof_dir: PathBuf,
    },
    /// Supervised Hoodi BEEFY actor: follow commitments and publish queued token roots.
    TokensFollow {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
        #[arg(long, default_value = "wss://ethereum-hoodi-rpc.publicnode.com")]
        ethereum_rpc: String,
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long)]
        root_wallet: PathBuf,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
        #[arg(long)]
        reconcile_once: bool,
        /// Test-only: allow loopback ws:// for a local Hoodi-fork rehearsal.
        #[arg(long)]
        local_rehearsal: bool,
        /// Pin a candidate BEEFY recovery plan; disables root publication until finalized activation.
        #[arg(long)]
        recovery_plan: Option<PathBuf>,
    },
    /// Stimulate and observe the fixed four-token Hoodi qualification; the actor runs separately.
    TokensSoak {
        #[arg(long, value_enum)]
        mode: tokens_soak::Mode,
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
        #[arg(long, default_value = "wss://ethereum-hoodi-rpc.publicnode.com")]
        ethereum_rpc: String,
        #[arg(long)]
        beacon_rpc: String,
        #[arg(long)]
        campaign_wallet: PathBuf,
        #[arg(long)]
        follower_dir: PathBuf,
        /// Read-only journal directory of the running inbound token worker.
        #[arg(long)]
        inbound_dir: PathBuf,
        /// Read-only journal directory of the running paid outbound token worker.
        #[arg(long)]
        outbound_dir: PathBuf,
        #[arg(long, env = "GEAR_CAMPAIGN_SURI")]
        campaign_suri: String,
        #[arg(long, env = "GEAR_GOVERNANCE_SURI")]
        governance_suri: String,
        #[arg(long, env = "BEEFY_ROTATION_SURI")]
        rotation_suri: String,
        #[arg(long, default_value = "alice")]
        rotation_authority: String,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        token_stack: PathBuf,
        #[arg(long)]
        raw_spec: PathBuf,
        #[arg(long)]
        source_launch_state: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Temporarily pause Gear's bridge builtin for a post-lock rejection test.
    BridgePause {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
    },
    /// Resume Gear's bridge builtin after a post-lock rejection test.
    BridgeUnpause {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
    },
    /// Read the finalized Gear bridge pause state and queue root at one block.
    BridgeStatus {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long)]
        at_block: Option<u32>,
    },
    /// Inspect Gear message identifiers for a previously finalized block.
    BridgeEvents {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long)]
        at_block: u32,
        #[arg(long)]
        full: bool,
    },
    /// Submit one authenticated outbound Gear bridge message, retaining its inclusion proof.
    Send {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "//Alice", env = "GEAR_SURI")]
        gear_suri: String,
        #[arg(long)]
        expected_genesis: String,
        #[arg(long)]
        expected_source: String,
        #[arg(long)]
        receiver: String,
        #[arg(long)]
        payload: String,
    },
}

pub(crate) fn local_source_rpc(rpc: &str) -> bool {
    let Ok(url) = Url::parse(rpc) else {
        return false;
    };
    if !matches!(url.scheme(), "ws" | "wss")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some((_, authority)) = rpc.split_once("://") else {
        return false;
    };
    if authority
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| {
            authority.is_empty() || authority.contains('@') || authority.ends_with(':')
        })
    {
        return false;
    }
    match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Rehearse {
            gear_node,
            chain_spec,
            output_dir,
        } => rehearsal::run(&gear_node, &chain_spec, &output_dir).await,
        Command::Hoodi {
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            wallet,
            output_dir,
            prepare_only,
            follow,
        } => {
            ensure!(
                local_source_rpc(&source_rpc) && local_source_rpc(&witness_rpc),
                "public dev keys: source and witness RPCs must be loopback"
            );
            hoodi::run(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &wallet,
                &output_dir,
                prepare_only,
                follow,
            )
            .await
        }
        Command::Tokens {
            source_rpc,
            expected_genesis,
            gear_suri_file,
            checkpoint,
            checkpoint_slot,
            output_dir,
            checkpoint_hash,
        } => {
            ensure!(
                local_source_rpc(&source_rpc),
                "public dev keys: source RPC must be loopback"
            );
            let gear_suri = read_setup_suri(&gear_suri_file)?;
            tokens::prepare(
                &source_rpc,
                &expected_genesis,
                gear_suri.trim(),
                &checkpoint,
                checkpoint_slot,
                &checkpoint_hash,
                &output_dir,
            )
            .await
        }
        Command::TokensConfigure {
            source_rpc,
            witness_rpc,
            expected_genesis,
            gear_suri_file,
            ethereum_rpc,
            wallet,
            deployment_manifest,
            token_stack,
        } => {
            ensure!(
                local_source_rpc(&source_rpc),
                "development-key source RPC must be loopback"
            );
            let gear_suri = read_setup_suri(&gear_suri_file)?;
            tokens::configure(
                &source_rpc,
                &witness_rpc,
                &expected_genesis,
                gear_suri.trim(),
                &ethereum_rpc,
                &wallet,
                &deployment_manifest,
                &token_stack,
            )
            .await
        }
        Command::TokensProvisionSourceInventory {
            source_rpc,
            witness_rpc,
            expected_genesis,
            gear_suri_file,
            campaign_suri_file,
            deployment_manifest,
            token_stack,
        } => {
            let setup = read_setup_suri(&gear_suri_file)?;
            let campaign = read_setup_suri(&campaign_suri_file)?;
            tokens::provision_source_inventory(
                &source_rpc,
                &witness_rpc,
                &expected_genesis,
                setup.trim(),
                campaign.trim(),
                &deployment_manifest,
                &token_stack,
            )
            .await
        }
        Command::TokensSnapshot {
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            beacon_rpc,
            deployment_manifest,
            token_stack,
            gear_user,
            evm_user,
            output,
        } => {
            tokens_soak::write_read_only_snapshot(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &beacon_rpc,
                &deployment_manifest,
                &token_stack,
                &gear_user,
                &evm_user,
                &output,
            )
            .await
        }
        Command::TokensAnchor {
            source_rpc,
            witness_rpc,
        } => tokens::anchor(&source_rpc, &witness_rpc).await,
        Command::TokensSourceState {
            raw_spec,
            source_rpc,
            witness_rpc,
            output,
        } => {
            ensure!(
                local_source_rpc(&source_rpc) && local_source_rpc(&witness_rpc),
                "source readiness endpoints must be loopback"
            );
            ensure!(
                source_rpc != witness_rpc,
                "two independent Gear authorities are required"
            );
            tokens_soak::write_source_launch_state(&raw_spec, &source_rpc, &witness_rpc, &output)
                .await
        }
        Command::TokensManifest {
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            wallet,
            anchor,
            token_stack,
            client,
            verifier,
            queue,
            manager,
            output,
            local_rehearsal,
        } => {
            tokens::manifest(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &wallet,
                &anchor,
                &token_stack,
                &client,
                &verifier,
                &queue,
                &manager,
                &output,
                local_rehearsal,
            )
            .await
        }
        Command::TokensPublishRoot {
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            wallet,
            deployment_manifest,
            publisher_state,
            registration,
            output,
        } => {
            ensure!(
                local_source_rpc(&source_rpc) && local_source_rpc(&witness_rpc),
                "public dev keys: source and witness RPCs must be loopback"
            );
            tokens::publish_root(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &wallet,
                &deployment_manifest,
                &publisher_state,
                &registration,
                &output,
            )
            .await
        }
        Command::TokensRecoveryActivate {
            ethereum_rpc,
            deployment_manifest,
            follower_dir,
            recovery_plan,
        } => {
            tokens::activate_recovery_transition(
                &ethereum_rpc,
                &deployment_manifest,
                &follower_dir,
                &recovery_plan,
            )
            .await
        }
        Command::TokensHistoryAudit {
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            deployment_manifest,
            follower_state,
            proof_dir,
        } => {
            hoodi::audit_history(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &deployment_manifest,
                &follower_state,
                &proof_dir,
            )
            .await
        }
        Command::TokensFollow {
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            wallet,
            root_wallet,
            deployment_manifest,
            output_dir,
            reconcile_once,
            local_rehearsal,
            recovery_plan,
        } => {
            hoodi::follow_tokens(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &wallet,
                &root_wallet,
                &deployment_manifest,
                &output_dir,
                reconcile_once,
                local_rehearsal,
                recovery_plan.as_deref(),
            )
            .await
        }
        Command::TokensSoak {
            mode,
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            beacon_rpc,
            campaign_wallet,
            follower_dir,
            inbound_dir,
            outbound_dir,
            campaign_suri,
            governance_suri,
            rotation_suri,
            rotation_authority,
            deployment_manifest,
            token_stack,
            raw_spec,
            source_launch_state,
            output_dir,
        } => {
            ensure!(
                local_source_rpc(&source_rpc) && local_source_rpc(&witness_rpc),
                "development-key source endpoints must be loopback"
            );
            tokens_soak::run(
                mode,
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &beacon_rpc,
                &campaign_wallet,
                &follower_dir,
                &inbound_dir,
                &outbound_dir,
                &campaign_suri,
                &governance_suri,
                &rotation_suri,
                &rotation_authority,
                &deployment_manifest,
                &token_stack,
                &raw_spec,
                &source_launch_state,
                &output_dir,
            )
            .await
        }
        Command::BridgePause { source_rpc } => {
            ensure!(
                local_source_rpc(&source_rpc),
                "public dev keys: source RPC must be loopback"
            );
            let api = gear_rpc_client::GearApi::new(&source_rpc, 3).await?;
            source::pause_bridge(&api).await
        }
        Command::BridgeUnpause { source_rpc } => {
            ensure!(
                local_source_rpc(&source_rpc),
                "public dev keys: source RPC must be loopback"
            );
            let api = gear_rpc_client::GearApi::new(&source_rpc, 3).await?;
            source::initialize_bridge(&api).await
        }
        Command::BridgeStatus {
            source_rpc,
            at_block,
        } => {
            let api = gear_rpc_client::GearApi::new(&source_rpc, 3).await?;
            source::bridge_status(&api, at_block).await
        }
        Command::BridgeEvents {
            source_rpc,
            at_block,
            full,
        } => {
            let api = gear_rpc_client::GearApi::new(&source_rpc, 3).await?;
            source::bridge_events(&api, at_block, full).await
        }
        Command::Send {
            source_rpc,
            gear_suri,
            expected_genesis,
            expected_source,
            receiver,
            payload,
        } => {
            ensure!(
                local_source_rpc(&source_rpc),
                "public dev keys: source RPC must be loopback"
            );
            use gsdk::ext::sp_core::{sr25519, Pair};
            let decode = |value: &str| hex::decode(value.strip_prefix("0x").unwrap_or(value));
            let genesis: [u8; 32] = decode(&expected_genesis)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected genesis must be 32 bytes"))?;
            let expected_source: [u8; 32] = decode(&expected_source)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected source must be 32 bytes"))?;
            let receiver: [u8; 20] = decode(&receiver)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("receiver must be 20 bytes"))?;
            let payload = decode(&payload)?;
            let actual_source = sr25519::Pair::from_string(&gear_suri, None)
                .context("derive Gear signer")?
                .public()
                .0;
            ensure!(
                actual_source == expected_source,
                "Gear signer differs from expected governance source"
            );
            let api = gear_rpc_client::GearApi::new(&source_rpc, 3).await?;
            ensure!(
                api.block_number_to_hash(0).await?.0 == genesis,
                "Gear genesis differs from expected source"
            );
            let observed = source::send_message(&api, receiver, &payload, &gear_suri).await?;
            ensure!(
                observed.message.source == expected_source,
                "queued source differs from signer"
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "block": observed.block,
                    "blockHash": format!("0x{}", hex::encode(observed.block_hash)),
                    "messageHash": format!("0x{}", hex::encode(observed.message_hash)),
                    "source": format!("0x{}", hex::encode(observed.message.source)),
                    "destination": format!("0x{}", hex::encode(observed.message.destination)),
                    "payload": format!("0x{}", hex::encode(observed.message.payload)),
                    "nonce": format!("0x{}", hex::encode(observed.message.nonce_be)),
                    "queueId": observed.snapshot.queue_id,
                    "queueRoot": format!("0x{}", hex::encode(observed.snapshot.queue_root)),
                    "retainedAtBlock": observed.retained_at_block,
                }))?
            );
            Ok(())
        }
    }
}

fn read_setup_suri(path: &std::path::Path) -> Result<String> {
    use std::{
        fs,
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt},
    };
    let before = fs::symlink_metadata(path)?;
    ensure!(
        before.file_type().is_file(),
        "setup credential must be a regular file"
    );
    let file = fs::File::open(path).context("open setup credential file")?;
    let metadata = file.metadata()?;
    ensure!(
        before.dev() == metadata.dev() && before.ino() == metadata.ino(),
        "setup credential changed while opening"
    );
    ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o777 == 0o600,
        "setup credential file must have mode 0600"
    );
    let mut suri = String::new();
    file.take(4097)
        .read_to_string(&mut suri)
        .context("read setup credential file")?;
    ensure!(
        suri.len() <= 4096 && !suri.trim().is_empty(),
        "setup credential is empty or oversized"
    );
    Ok(suri)
}

fn message_hash(message: &gear_rpc_client::dto::Message) -> beefy_relay::Hash32 {
    let mut preimage = Vec::with_capacity(84 + message.payload.len());
    preimage.extend_from_slice(&message.nonce_be);
    preimage.extend_from_slice(&message.source);
    preimage.extend_from_slice(&message.destination);
    preimage.extend_from_slice(&message.payload);
    beefy_relay::keccak256(&preimage)
}

#[cfg(test)]
mod tests {
    use super::{local_source_rpc, read_setup_suri, Cli};
    use clap::Parser;
    #[test]
    fn setup_credentials_reject_public_files_and_symlinks() {
        use std::{
            fs,
            os::unix::fs::{symlink, PermissionsExt},
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("setup.suri");
        fs::write(&path, "//Alice\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_setup_suri(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_setup_suri(&path).unwrap().trim(), "//Alice");
        let link = directory.path().join("link.suri");
        symlink(&path, &link).unwrap();
        assert!(read_setup_suri(&link).is_err());
        fs::write(&path, " \n").unwrap();
        assert!(read_setup_suri(&path).is_err());
        fs::write(&path, "x".repeat(4097)).unwrap();
        assert!(read_setup_suri(&path).is_err());
    }
    #[test]
    fn source_state_cli_requires_all_identity_inputs() {
        assert!(Cli::try_parse_from([
            "beefy-relay",
            "tokens-source-state",
            "--raw-spec",
            "chain.raw.json",
            "--source-rpc",
            "ws://127.0.0.1:9948",
            "--witness-rpc",
            "ws://127.0.0.1:9949",
            "--output",
            "launch-state.json",
        ])
        .is_ok());
        assert!(Cli::try_parse_from(["beefy-relay", "tokens-source-state"]).is_err());
    }
    #[test]
    fn development_signer_requires_a_parsed_loopback_authority() {
        for rpc in [
            "ws://127.0.0.1:9954",
            "wss://[::1]:9944",
            "ws://localhost:9954/",
        ] {
            assert!(local_source_rpc(rpc), "{rpc}");
        }
        for rpc in [
            "ws://example.com:9954",
            "ws://localhost:9954@remote.example:9954",
            "ws://@localhost:9954",
            "ws://user%40name@localhost:9954",
            "ws://localhost.evil:9954",
            "ws://127.0.0.1:99999",
            "ws://localhost:",
            "ws://127.0.0.1:9954/path",
            "ws://127.0.0.1:9954?token=secret",
            "https://127.0.0.1:9954",
        ] {
            assert!(!local_source_rpc(rpc), "{rpc}");
        }
    }
}
