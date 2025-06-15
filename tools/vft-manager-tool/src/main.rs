use clap::{Args, Parser, Subcommand};
use gclient::{GearApi, WSAddress};
use anyhow::{anyhow, Context, Result as AnyResult};
use sails_rs::{gclient::calls::GClientRemoting, prelude::*};

const SIZE_MIGRATE_BATCH: u32 = 100;

#[derive(Args)]
pub struct GearArgs {
    /// Domain of the Gear RPC endpoint
    #[arg(
        long,
        default_value = "ws://127.0.0.1",
        env
    )]
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
    MigrateTransactions(MigrateTransactions),
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

fn str_to_actorid(s: String) -> AnyResult<ActorId> {
    let s = if &s[..2] == "0x" { &s[2..] } else { &s };
    let data = hex::decode(s)?;

    Ok(ActorId::new(data.try_into().map_err(|_| anyhow!("Input hex-string has wrong length"))?))
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let _ = dotenv::dotenv();

    pretty_env_logger::formatted_timed_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .format_timestamp_secs()
        .parse_default_env()
        .init();

    let cli = Cli::parse();
    let args_gear = &cli.args_gear;
    match cli.command {
        CliCommands::MigrateTransactions(args) => {
            let size_batch = args.size_batch.unwrap_or(SIZE_MIGRATE_BATCH);
            let vft_manager_old = str_to_actorid(args.old_vft_manager)
                .context("Unable to parse address of the old VftManager")?;
            let vft_manager_new = str_to_actorid(args.new_vft_manager)
                .context("Unable to parse address of the new VftManager")?;

            let address = WSAddress::new(&args_gear.gear_domain, Some(args_gear.gear_port));
            let gear_api = GearApi::builder()
                .suri(&args_gear.gear_suri)
                .build(address)
                .await
                .context("Failed to initialize old GearApi")?;
            let gas_limit = gear_api
                .block_gas_limit()
                .context("Unable to get block gas limit")?;
            let remoting = GClientRemoting::new(gear_api);
            
            gear_common::migrate_transactions(gas_limit, size_batch, remoting.clone(), vft_manager_old, remoting, vft_manager_new).await?;
        }
    }

    Ok(())
}
