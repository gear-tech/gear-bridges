use anyhow::{ensure, Context, Result};
use beefy_relay::{local_source_rpc, source::local};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
mod token_deployment;
#[derive(Parser)]
#[command(about = "Explicit local Gear / Hoodi bootstrap and token deployment tooling")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
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
        #[arg(long)]
        owned_fixture_network: Option<PathBuf>,
    },
    TokensAnchor {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
    },
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
        #[arg(long, conflicts_with = "local_rehearsal")]
        owned_fixture_network: Option<PathBuf>,
    },
    /// Persist and finalize native owner-bound session keys for a local validator.
    SourceKeys {
        #[arg(long)]
        source_rpc: String,
        #[arg(long)]
        witness_rpc: String,
        #[arg(long)]
        expected_genesis: String,
        #[arg(long, value_parser = ["alice", "bob"], default_value = "alice")]
        authority: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Initialize and unpause the bridge builtin on a local development source.
    BridgeInitialize {
        #[arg(long)]
        source_rpc: String,
        #[arg(long)]
        expected_genesis: String,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
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
            token_deployment::prepare(
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
            owned_fixture_network,
        } => {
            ensure!(
                local_source_rpc(&source_rpc),
                "development-key source RPC must be loopback"
            );
            let gear_suri = read_setup_suri(&gear_suri_file)?;
            token_deployment::configure(
                &source_rpc,
                &witness_rpc,
                &expected_genesis,
                gear_suri.trim(),
                &ethereum_rpc,
                &wallet,
                &deployment_manifest,
                &token_stack,
                owned_fixture_network.as_deref(),
            )
            .await
        }
        Command::TokensAnchor {
            source_rpc,
            witness_rpc,
        } => token_deployment::anchor(&source_rpc, &witness_rpc).await,
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
            owned_fixture_network,
        } => {
            token_deployment::manifest(
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
                owned_fixture_network.as_deref(),
            )
            .await
        }
        Command::SourceKeys {
            source_rpc,
            witness_rpc,
            expected_genesis,
            authority,
            output_dir,
        } => {
            local::source_keys(
                &source_rpc,
                &witness_rpc,
                &expected_genesis,
                &authority,
                &output_dir,
            )
            .await
        }
        Command::BridgeInitialize {
            source_rpc,
            expected_genesis,
        } => {
            ensure!(
                local_source_rpc(&source_rpc),
                "development signer requires a loopback source"
            );
            let api = gear_rpc_client::GearApi::new(&source_rpc, 3).await?;
            ensure!(
                format!("0x{}", hex::encode(api.block_number_to_hash(0).await?.0))
                    == expected_genesis,
                "wrong source genesis"
            );
            local::initialize_bridge(&api).await
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

#[cfg(all(test, unix))]
mod tests {
    use super::read_setup_suri;
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
}
