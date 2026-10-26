use anyhow::{anyhow, Context, Result as AnyResult};
use clap::{Args, Parser, Subcommand};
use cli_utils::GearSignerArgs;
use gclient::GearApi;
use sails_rs::{
    calls::*,
    gclient::calls::{GClientRemoting, QueryExtGClient},
    prelude::*,
};
use vft_manager_client::{traits::*, vft_manager::io, Order};
mod cutover;

const SIZE_MIGRATE_BATCH: u32 = 100;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[clap(flatten)]
    gear: GearSignerArgs,

    #[command(subcommand)]
    command: CliCommands,
}

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
enum CliCommands {
    /// Authenticate boundary, cover every receipt, and drain original pre-boundary work.
    PrepareCutover(cutover::PrepareCutover),
    /// Resume custody/role handoff, seal originals, initialize successor lanes, and activate.
    FinalizeCutover(cutover::FinalizeCutover),
    /// Authenticate sealed historical originals without signing or mutating any journal.
    VerifyCutover(cutover::VerifyCutover),
    /// Abort one exact generation before any custody child; restore its prepared destination.
    AbortCutover(cutover::AbortCutover),
    ReadTransactions(ReadTransactions),
    HexEncodedMessage(HexEncodedMessageArgs),
}

#[derive(Args)]
struct HexEncodedMessageArgs {
    #[command(subcommand)]
    message: HexEncodedMessage,
}

#[derive(Subcommand)]
enum HexEncodedMessage {
    /// Message UpdateVfts. Mapping is constructed the following way [(vfts[0], vfts[1]), (vfts[2], vfts[3]), ...]
    /// so the len should be even. If there are odd items count then the last one will be ignored.
    UpdateVfts {
        vfts: Vec<ActorId>,
    },
    MapVaraToEthAddress {
        address_gear: ActorId,
        address_eth: H160,
        is_gear: bool,
    },
    RemoveVaraToEthAddress {
        vara_token_id: ActorId,
    },
    /// Encode bounded manager resolution without broadcasting.
    ResolveReceiptDeposit {
        slot: u64,
        transaction_index: u64,
        log_index: u64,
        operation_id: H256,
        child: Option<MessageId>,
    },
    /// Encode a hold using the raw SCALE Redemption snapshot, without its route.
    HoldNativeRedemption {
        operation_id: H256,
        expected_redemption_hex: String,
        #[arg(long)]
        acknowledge_irreversible_hold: bool,
        #[command(flatten)]
        native_capability: cutover::NativeCapabilityArgs,
    },
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
    let s = s.strip_prefix("0x").unwrap_or(&s);
    let data = hex::decode(s)?;

    Ok(ActorId::new(data.try_into().map_err(|_| {
        anyhow!("Input hex-string has wrong length")
    })?))
}

fn print_encoded_message(message: HexEncodedMessage) -> AnyResult<()> {
    use HexEncodedMessage::*;

    let (label, encoded_call) = match message {
        UpdateVfts { vfts } => {
            let (chunks, remainder) = vfts.as_chunks::<2>();
            if !remainder.is_empty() {
                log::warn!("slice doesn't have even length. The last item is ignored.")
            }

            (
                "UpdateVfts",
                io::UpdateVfts::encode_call(
                    chunks.iter().map(|chunk| (chunk[0], chunk[1])).collect(),
                ),
            )
        }

        MapVaraToEthAddress {
            address_gear,
            address_eth,
            is_gear,
        } => (
            "MapVaraToEthAddress",
            io::MapVaraToEthAddress::encode_call(
                address_gear,
                address_eth,
                if is_gear {
                    vft_manager_client::TokenSupply::Gear
                } else {
                    vft_manager_client::TokenSupply::Ethereum
                },
            ),
        ),
        RemoveVaraToEthAddress { vara_token_id } => (
            "RemoveVaraToEthAddress",
            io::RemoveVaraToEthAddress::encode_call(vara_token_id),
        ),
        ResolveReceiptDeposit {
            slot,
            transaction_index,
            log_index,
            operation_id,
            child,
        } => (
            "ResolveReceiptDeposit",
            io::ResolveReceiptDeposit::encode_call(
                slot,
                transaction_index,
                log_index,
                operation_id,
                child,
            ),
        ),
        HoldNativeRedemption {
            operation_id,
            expected_redemption_hex,
            acknowledge_irreversible_hold,
            native_capability,
        } => {
            let approved = native_capability
                .load()?
                .context("HOLD: explicit native capability approval required")?;
            anyhow::ensure!(
                approved.approval.wrapper.mode
                    == gear_common::native_wrapper::NativeWrapperMode::HeldEscrowV2,
                "HOLD: escrow-v1 has no HoldRedemption capability"
            );
            let bytes = hex::decode(
                expected_redemption_hex
                    .strip_prefix("0x")
                    .unwrap_or(&expected_redemption_hex),
            )
            .context("Invalid Redemption hex")?;
            let mut input = bytes.as_slice();
            let expected = vft_vara_client::Redemption::decode(&mut input)
                .context("Invalid SCALE Redemption")?;
            anyhow::ensure!(input.is_empty(), "Trailing bytes after SCALE Redemption");
            (
                "HoldRedemption",
                vft_vara_client::native_escrow::io::HoldRedemption::encode_call(
                    operation_id,
                    expected,
                    acknowledge_irreversible_hold,
                ),
            )
        }
    };

    println!(r#"{label}: "{}""#, hex::encode(encoded_call));
    Ok(())
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
    if let CliCommands::HexEncodedMessage(args) = cli.command {
        return print_encoded_message(args.message);
    }

    if let CliCommands::VerifyCutover(args) = &cli.command {
        return cutover::verify(&cli.gear.connection, args).await;
    }
    let endpoint = cli.gear.connection.get_endpoint().expect("Invalid URL");
    gear_common::signer_ownership::validate_suri(&cli.gear.suri, Some(&endpoint))?;
    let gear_api = GearApi::builder()
        .suri(cli.gear.suri)
        .uri(endpoint)
        .build()
        .await
        .context("Failed to initialize GearApi")?;

    match cli.command {
        CliCommands::PrepareCutover(args) => {
            cutover::prepare(&cli.gear.connection, &gear_api, &args).await?
        }
        CliCommands::FinalizeCutover(args) => {
            cutover::finalize(&cli.gear.connection, &gear_api, &args).await?
        }
        CliCommands::AbortCutover(args) => cutover::abort(&gear_api, &args).await?,
        CliCommands::VerifyCutover(_) => unreachable!("read-only command handled before signer"),

        CliCommands::ReadTransactions(args) => {
            let size_batch = args.size_batch.unwrap_or(SIZE_MIGRATE_BATCH);
            let vft_manager = str_to_actorid(args.vft_manager)
                .context("Unable to parse address of the VftManager")?;

            let gas_limit = gear_api
                .block_gas_limit()
                .context("Unable to get block gas limit")?;

            let signer: gsdk::signer::Signer = gear_api.into();

            let block_hash = signer
                .api()
                .legacy()
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
                    .at_block(block_hash.0.into())
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

        CliCommands::HexEncodedMessage(..) => {}
    }

    Ok(())
}
