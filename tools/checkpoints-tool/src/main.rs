use anyhow::{anyhow, Result as AnyResult};
use checkpoint_light_client::WASM_BINARY;
use checkpoint_light_client_client::{checkpoint_light_client_factory, traits::*};
use checkpoint_light_client_io::{
    ethereum_common::{
        base_types::BytesFixed, network::Network, tree_hash::TreeHash, utils as eth_utils,
    },
    Init, G2,
};
use clap::Parser;
use cli_utils::{BeaconConnectionArgs, GearConnectionArgs};
use ethereum_beacon_client::{utils, BeaconClient};
use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use parity_scale_codec::Encode;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use std::time::Duration;

const GEAR_API_RETRIES: u8 = 3;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Perform a dry run without actual deployment
    #[arg(long, default_value_t = false, env, num_args=0..=1)]
    dry_run: bool,

    #[clap(flatten)]
    gear_connection: GearConnectionArgs,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = "//Alice", env = "GEAR_SURI")]
    gear_suri: String,

    #[clap(flatten)]
    beacon: BeaconConnectionArgs,

    /// Specify the checkpoint slot for bootstrapping. If it is None then the header from
    /// the latest finality update is used to get the slot.
    #[arg(long, env = "SLOT_CHECKPOINT")]
    slot_checkpoint: Option<u64>,

    /// Specify salt for the send_recv call (hex string)
    #[arg(long, env)]
    salt: Option<String>,
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();

    let (gear_host, gear_port) = cli.gear_connection.get_host_port()?;

    println!("Using Gear endpoint: {gear_host}:{gear_port}");

    let beacon_client = BeaconClient::new(
        cli.beacon.beacon_endpoint,
        cli.beacon.timeout.map(Duration::from_secs),
    )
    .await?;

    let genesis = beacon_client.get_genesis().await?;

    let network =
        Network::from_genesis_validators_root(genesis.data.genesis_validators_root[..].try_into()?)
            .ok_or_else(|| {
                anyhow!(
                    "Failed to determine network from genesis validators root: {}",
                    hex::encode(genesis.data.genesis_validators_root)
                )
            })?;

    println!("Using Ethereum network: '{network:?}'");

    let slot = match cli.slot_checkpoint {
        Some(slot) => slot,
        None => {
            let update = beacon_client.get_finality_update().await?;

            update.finalized_header.slot
        }
    };
    let current_period = eth_utils::calculate_period(slot);
    let mut updates = beacon_client.get_updates(current_period, 1).await?;

    println!("finality_update slot = {slot}, period = {current_period}");

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };

    let checkpoint = update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    println!(
        "checkpoint slot = {}, hash = {}",
        update.finalized_header.slot, checkpoint_hex
    );

    let bootstrap = beacon_client.get_bootstrap(&checkpoint_hex).await?;
    println!("bootstrap slot = {}", bootstrap.header.slot);

    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .map_err(|e| anyhow!("Failed to decode signature: {e:?}"))?;

    let sync_aggregate_encoded = update.sync_aggregate.encode();
    let sync_update = utils::sync_update_from_update(signature, update);
    let pub_keys = utils::map_public_keys(&bootstrap.current_sync_committee.pubkeys);

    let init = Init {
        network,
        sync_committee_current_pub_keys: pub_keys,
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|BytesFixed(bytes)| bytes.0)
            .collect(),
        update: sync_update,
        sync_aggregate_encoded,
    };

    if cli.dry_run {
        println!(
            "Dry run enabled, not deploying the program, run with `--dry-run false` to deploy."
        );
        return Ok(());
    }

    let api = GearApi::builder()
        .retries(GEAR_API_RETRIES)
        .suri(cli.gear_suri)
        .build(WSAddress::new(gear_host, gear_port))
        .await?;

    let code_id = api
        .upload_code(WASM_BINARY)
        .await
        .map(|(code_id, _)| code_id)
        .unwrap_or_else(|_| CodeId::generate(WASM_BINARY));

    println!("Using code_id = {code_id:?}");

    let gas_limit = {
        let payload = {
            let mut result = checkpoint_light_client_factory::io::Init::ROUTE.to_vec();
            init.encode_to(&mut result);

            result
        };

        api.calculate_upload_gas(None, WASM_BINARY.to_vec(), payload, 0, true)
            .await?
            .min_limit
    };
    let factory = checkpoint_light_client_client::CheckpointLightClientFactory::new(
        GClientRemoting::new(api.clone()),
    );

    // Parse salt from hex string if provided
    let salt = match &cli.salt {
        Some(salt_str) => {
            let hex_str = salt_str.trim().strip_prefix("0x").unwrap_or(salt_str);
            hex::decode(hex_str).map_err(|e| anyhow!("Invalid hex salt '{hex_str}': {e}"))?
        }
        None => vec![],
    };

    let program_id = factory
        .init(init)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .map_err(|e| anyhow!("Failed to construct program: {e:?}"))?;

    println!("program_id = {program_id:?}");

    Ok(())
}
