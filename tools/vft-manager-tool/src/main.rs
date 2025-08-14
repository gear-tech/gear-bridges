use anyhow::{anyhow, Context, Result as AnyResult};
use clap::{Args, Parser, Subcommand};
use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, gclient::calls::{GClientRemoting, QueryExt}, prelude::*};
use vft_manager::WASM_BINARY;
use vft_manager_client::{traits::*, InitConfig, Order};

const SIZE_MIGRATE_BATCH: u32 = 100;

#[derive(Args)]
pub struct GearArgs {
    /// Domain of the Gear RPC endpoint
    #[arg(long, default_value = "ws://127.0.0.1", env)]
    pub gear_domain: String,

    /// Port of the Gear RPC endpoint
    #[arg(long, default_value = "9944", env)]
    pub gear_port: u16,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = "//Alice", env)]
    pub gear_suri: String,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[clap(flatten)]
    args_gear: GearArgs,

    #[command(subcommand)]
    command: CliCommands,
}

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
enum CliCommands {
    DeployUpgraded(DeployUpgraded),
    MigrateTransactions(MigrateTransactions),
    ReadTransactions(ReadTransactions),
}

#[derive(Args)]
struct DeployUpgraded {
    #[arg(long, help = format!("Size of migration batch. Default: {SIZE_MIGRATE_BATCH}"))]
    size_batch: Option<u32>,

    /// Flag determines whether to ignore non empty list of failed bridging requests
    #[arg(long)]
    ignore_non_empty_message_tracker: bool,

    /// ActorId of the source VFT-manager contract
    #[arg(long)]
    vft_manager: String,
}

#[derive(Args)]
struct MigrateTransactions {
    #[arg(long, help = format!("Size of migration batch. Default: {SIZE_MIGRATE_BATCH}"))]
    size_batch: Option<u32>,

    /// ActorId of the source VFT-manager contract (old)
    #[arg(long)]
    old_vft_manager: String,

    /// ActorId of the destination VFT-manager contract (new). Provided Gear parameters should have an account
    /// with admin priveleges
    #[arg(long)]
    new_vft_manager: String,
}

#[derive(Args)]
struct ReadTransactions {
    #[arg(long, help = format!("Size of batch. Default: {SIZE_MIGRATE_BATCH}"))]
    size_batch: Option<u32>,

    #[arg(long)]
    block_number: u32,

    /// ActorId of the VFT-manager contract
    #[arg(long)]
    vft_manager: String,
}

fn str_to_actorid(s: String) -> AnyResult<ActorId> {
    let s = if &s[..2] == "0x" { &s[2..] } else { &s };
    let data = hex::decode(s)?;

    Ok(ActorId::new(data.try_into().map_err(|_| {
        anyhow!("Input hex-string has wrong length")
    })?))
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let _ = dotenv::dotenv();

    pretty_env_logger::formatted_timed_builder()
        .filter_level(log::LevelFilter::Info)
        .format_target(false)
        .format_timestamp_secs()
        .parse_default_env()
        .init();

    let cli = Cli::parse();
    let args_gear = &cli.args_gear;
    let address = WSAddress::new(&args_gear.gear_domain, Some(args_gear.gear_port));
    let gear_api = GearApi::builder()
        .suri(&args_gear.gear_suri)
        .build(address)
        .await
        .context("Failed to initialize GearApi")?;

    match cli.command {
        CliCommands::MigrateTransactions(args) => {
            let size_batch = args.size_batch.unwrap_or(SIZE_MIGRATE_BATCH);
            let vft_manager_old = str_to_actorid(args.old_vft_manager)
                .context("Unable to parse address of the old VftManager")?;
            let vft_manager_new = str_to_actorid(args.new_vft_manager)
                .context("Unable to parse address of the new VftManager")?;

            let gas_limit = gear_api
                .block_gas_limit()
                .context("Unable to get block gas limit")?;
            let remoting = GClientRemoting::new(gear_api);

            gear_common::migrate_transactions(
                gas_limit,
                size_batch,
                remoting.clone(),
                vft_manager_old,
                remoting,
                vft_manager_new,
            )
            .await?;
        }

        CliCommands::DeployUpgraded(args) => deploy_upgraded(gear_api, args).await?,

        CliCommands::ReadTransactions(args) => {
            let size_batch = args.size_batch.unwrap_or(SIZE_MIGRATE_BATCH);
            let vft_manager = str_to_actorid(args.vft_manager)
                .context("Unable to parse address of the VftManager")?;

            let gas_limit = gear_api
                .block_gas_limit()
                .context("Unable to get block gas limit")?;

            let signer: gsdk::signer::Signer = gear_api.into();

            let block_hash = signer.api().rpc()
                .chain_get_block_hash(Some(args.block_number.into()))
                .await?
                .ok_or_else(|| anyhow!("Block #{} not present on RPC node", args.block_number))?;

            let gear_api = GearApi::from(signer);
            let remoting = GClientRemoting::new(gear_api);
            let service = vft_manager_client::VftManager::new(remoting);
            let mut cursor = 0;
            loop {
                let transactions = service
                    .transactions(Order::Direct, cursor, size_batch)
                    .with_gas_limit(gas_limit)
                    .at_block(block_hash)
                    .recv(vft_manager)
                    .await
                    .map_err(|e| anyhow!("{e:?}"))?;
                let len = transactions.len();
                
                log::info!("transactions (from {cursor}): {transactions:?}");

                cursor += size_batch;

                if (len as u32) < size_batch {
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn deploy_upgraded(gear_api: GearApi, args: DeployUpgraded) -> AnyResult<()> {
    let DeployUpgraded {
        size_batch,
        ignore_non_empty_message_tracker,
        vft_manager,
    } = args;
    let vft_manager_old = str_to_actorid(vft_manager)?;

    let gas_limit = gear_api
        .block_gas_limit()
        .context("Unable to get block gas limit")?;
    let remoting = GClientRemoting::new(gear_api.clone());
    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    let is_paused = service
        .is_paused()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    if !is_paused {
        return Err(anyhow!("VftManager is running"));
    }

    let msg_tracker = service
        .request_briding_msg_tracker_state(0, 1)
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    if !ignore_non_empty_message_tracker && !msg_tracker.is_empty() {
        return Err(anyhow!("VftManager has non empty message tracker"));
    }

    let historical_proxy_address = service
        .historical_proxy_address()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    let gear_bridge_builtin = service
        .gear_bridge_builtin()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    let config = service
        .get_config()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let erc20_manager_address = service
        .erc_20_manager_address()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    let vara_to_eth_addresses = service
        .vara_to_eth_addresses()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    let admin_pause = service
        .pause_admin()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    let admin = service
        .admin()
        .recv(vft_manager_old)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let code_id = gear_api
        .upload_code(WASM_BINARY)
        .await
        .map(|(code_id, ..)| code_id)
        .unwrap_or_else(|err| {
            log::debug!("Failed to upload code: {err}");
            CodeId::generate(WASM_BINARY)
        });

    let factory = vft_manager_client::VftManagerFactory::new(remoting.clone());
    let vft_manager_new = factory
        .new(InitConfig {
            gear_bridge_builtin,
            historical_proxy_address,
            config,
        })
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    let size_batch = size_batch.unwrap_or(SIZE_MIGRATE_BATCH);
    gear_common::migrate_transactions(
        gas_limit,
        size_batch,
        remoting.clone(),
        vft_manager_old,
        remoting,
        vft_manager_new,
    )
    .await?;

    if let Some(erc20_manager_address) = erc20_manager_address {
        service
            .update_erc_20_manager_address(erc20_manager_address)
            .with_gas_limit(gas_limit)
            .send_recv(vft_manager_new)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;
    }

    for (vara_token_id, eth_token_id, supply_type) in vara_to_eth_addresses {
        service
            .map_vara_to_eth_address(vara_token_id, eth_token_id, supply_type)
            .with_gas_limit(gas_limit)
            .send_recv(vft_manager_new)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;
    }

    service
        .set_pause_admin(admin_pause)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_new)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    service
        .set_admin(admin)
        .with_gas_limit(gas_limit)
        .send_recv(vft_manager_new)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;

    log::info!("Address of the new VftManager: {vft_manager_new}");

    Ok(())
}
