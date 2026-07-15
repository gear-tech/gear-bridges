use anyhow::{Context, Result as AnyResult};
use clap::{ArgGroup, Parser};
use cli_utils::GearConnectionArgs;
use primitive_types::H256;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
#[command(group(
    ArgGroup::new("block")
        .required(true)
        .multiple(false)
        .args(["block_number", "block_hash"])
))]
struct Cli {
    #[clap(flatten)]
    gear: GearConnectionArgs,

    /// Gear block number to fetch a signed block ID for.
    #[arg(long)]
    block_number: Option<u32>,

    /// Gear block hash to fetch a signed block ID for.
    #[arg(long)]
    block_hash: Option<H256>,
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();
    let endpoint = cli.gear.get_endpoint()?;
    let api = gear_rpc_client::GearApi::new(&endpoint, cli.gear.max_reconnect_attempts)
        .await
        .context("Failed to connect to Gear RPC")?;

    let (requested_block_number, requested_block_hash) = match (cli.block_number, cli.block_hash) {
        (Some(block_number), None) => {
            let block_hash = api
                .block_number_to_hash(block_number)
                .await
                .with_context(|| format!("Failed to fetch hash for block #{block_number}"))?;

            (block_number, block_hash)
        }
        (None, Some(block_hash)) => {
            let block_number = api
                .block_hash_to_number(block_hash)
                .await
                .with_context(|| format!("Failed to fetch number for block {block_hash:?}"))?;

            (block_number, block_hash)
        }
        _ => unreachable!("clap enforces exactly one block selector"),
    };

    let (justification, _) = api
        .grandpa_prove_finality(requested_block_number)
        .await
        .with_context(|| {
            format!("Failed to fetch finality proof after block #{requested_block_number}")
        })?;
    let signed_block_hash = H256::from(justification.commit.target_hash.0);

    println!("requested_block_number = {requested_block_number}");
    println!("requested_block_hash = {requested_block_hash:?}");
    println!(
        "signed_block_number = {}",
        justification.commit.target_number
    );
    println!("signed_block_hash = {signed_block_hash:?}");
    println!(
        "signed_block_id = ({}, {:?})",
        justification.commit.target_number, signed_block_hash
    );
    println!("justification_round = {}", justification.round);
    println!(
        "precommit_count = {}",
        justification.commit.precommits.len()
    );

    Ok(())
}
