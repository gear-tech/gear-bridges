use anyhow::Result;
use beefy_relay::{ethereum::PublicEthereumNetwork, follower, root_publication, source};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
#[derive(Parser)]
#[command(about = "Durable Vara BEEFY actor for an explicitly selected Ethereum deployment")]
struct Cli {
    #[arg(long, global = true, value_enum, default_value = "hoodi")]
    ethereum_network: PublicEthereumNetwork,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    TokensPublishRoot {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
        #[arg(long, default_value = "https://hoodi.drpc.org")]
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
        #[arg(long)]
        owned_fixture_network: Option<PathBuf>,
    },
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
        #[arg(long)]
        owned_fixture_network: Option<PathBuf>,
    },
    TokensFollow {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long, default_value = "ws://127.0.0.1:9945")]
        witness_rpc: String,
        #[arg(long, default_value = "https://hoodi.drpc.org")]
        ethereum_rpc: String,
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long)]
        root_wallet: PathBuf,
        #[arg(long)]
        deployment_manifest: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
        /// Untrusted constructor locator; canonical signed creation evidence is still required.
        #[arg(long)]
        client_deployment_transaction: Option<alloy::primitives::B256>,
        #[arg(long)]
        reconcile_once: bool,
        /// Test-only: allow loopback ws:// for a local Hoodi-fork rehearsal.
        #[arg(long)]
        local_rehearsal: bool,
        #[arg(long, conflicts_with = "local_rehearsal")]
        owned_fixture_network: Option<PathBuf>,
    },
    BridgeStatus {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long)]
        at_block: Option<u32>,
    },
    BridgeEvents {
        #[arg(long, default_value = "ws://127.0.0.1:9944")]
        source_rpc: String,
        #[arg(long)]
        at_block: u32,
        #[arg(long)]
        full: bool,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::TokensPublishRoot {
            source_rpc,
            witness_rpc,
            ethereum_rpc,
            wallet,
            deployment_manifest,
            publisher_state,
            registration,
            output,
            owned_fixture_network,
        } => {
            root_publication::publish_root(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &wallet,
                &deployment_manifest,
                &publisher_state,
                &registration,
                &output,
                owned_fixture_network.as_deref(),
                cli.ethereum_network,
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
            owned_fixture_network,
        } => {
            follower::audit_history(
                (&source_rpc, &witness_rpc),
                &ethereum_rpc,
                &deployment_manifest,
                &follower_state,
                &proof_dir,
                owned_fixture_network.as_deref(),
                cli.ethereum_network,
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
            client_deployment_transaction,
            reconcile_once,
            local_rehearsal,
            owned_fixture_network,
        } => {
            follower::follow_tokens(
                &source_rpc,
                &witness_rpc,
                &ethereum_rpc,
                &wallet,
                &root_wallet,
                &deployment_manifest,
                &output_dir,
                reconcile_once,
                local_rehearsal,
                owned_fixture_network.as_deref(),
                client_deployment_transaction,
                cli.ethereum_network,
            )
            .await
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
    }
}
