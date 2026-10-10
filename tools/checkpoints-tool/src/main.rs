use anyhow::{anyhow, ensure, Context, Result as AnyResult};
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
use gclient::GearApi;
use gear_core::ids::prelude::*;
use parity_scale_codec::Encode;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use std::time::Duration;

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

    /// Independently approved, recent weak-subjectivity Beacon block root (32-byte hex).
    #[arg(long, env = "TRUSTED_BOOTSTRAP_ROOT")]
    trusted_bootstrap_root: String,

    /// Independently pinned network genesis validators root (32-byte hex).
    #[arg(long, env = "TRUSTED_GENESIS_VALIDATORS_ROOT")]
    trusted_genesis_validators_root: String,

    /// Optional assertion of the trusted bootstrap sync-committee period by slot.
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

    let endpoint = cli.gear_connection.get_endpoint()?;

    println!("Using Gear endpoint: {endpoint}");

    let beacon_client = BeaconClient::new(
        cli.beacon.beacon_endpoint,
        cli.beacon.timeout.map(Duration::from_secs),
    )
    .await?;

    let trusted_genesis: [u8; 32] =
        hex::decode(cli.trusted_genesis_validators_root.trim_start_matches("0x"))?
            .try_into()
            .map_err(|_| anyhow!("trusted genesis validators root must be 32 bytes"))?;
    let trusted_bootstrap: [u8; 32] =
        hex::decode(cli.trusted_bootstrap_root.trim_start_matches("0x"))?
            .try_into()
            .map_err(|_| anyhow!("trusted bootstrap root must be 32 bytes"))?;
    let network = Network::from_genesis_validators_root(trusted_genesis)
        .ok_or_else(|| anyhow!("unsupported trusted Ethereum network"))?;
    let genesis = beacon_client.get_genesis().await?;
    ensure!(
        genesis.data.genesis_validators_root == trusted_genesis
            && genesis.data.genesis_time == network.genesis_time(),
        "Beacon provider network/genesis mismatch"
    );
    println!("Using Ethereum network: '{network:?}'");
    let checkpoint_hex = hex::encode(trusted_bootstrap);
    let bootstrap = beacon_client.get_bootstrap(&checkpoint_hex).await?;
    ensure!(
        bootstrap.header.tree_hash_root().0 == trusted_bootstrap,
        "bootstrap header differs from independently trusted root"
    );
    let slot = bootstrap.header.slot;
    if let Some(expected_slot) = cli.slot_checkpoint {
        ensure!(
            eth_utils::calculate_period(expected_slot) == eth_utils::calculate_period(slot),
            "bootstrap period differs from requested trusted period"
        );
    }
    let current_period = eth_utils::calculate_period(slot);
    let mut updates = beacon_client.get_updates(current_period, 1).await?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };
    println!(
        "finality_update slot = {}, period = {current_period}",
        update.finalized_header.slot
    );

    ensure!(
        bootstrap.header.slot <= update.finalized_header.slot
            && eth_utils::calculate_period(bootstrap.header.slot)
                == eth_utils::calculate_period(update.finalized_header.slot),
        "bootstrap must precede update in its sync committee period"
    );
    println!(
        "checkpoint slot = {}, hash = {}",
        update.finalized_header.slot,
        hex::encode(update.finalized_header.tree_hash_root().0)
    );
    println!("bootstrap slot = {slot}, hash = {checkpoint_hex}");

    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .map_err(|e| anyhow!("Failed to decode signature: {e:?}"))?;

    let sync_aggregate_encoded = update.sync_aggregate.encode();
    let sync_update = utils::sync_update_from_update(signature, update);
    let pub_keys = utils::map_public_keys(&bootstrap.current_sync_committee.pubkeys);

    let init = Init {
        network,
        trusted_bootstrap_root: trusted_bootstrap.into(),
        bootstrap_header: bootstrap.header,
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
        .suri(cli.gear_suri)
        .uri(endpoint)
        .build()
        .await?;

    let gas_limit = {
        let payload = {
            let mut result = checkpoint_light_client_factory::io::Init::ROUTE.to_vec();
            init.encode_to(&mut result);

            result
        };

        api.calculate_upload_gas(None, WASM_BINARY.to_vec(), payload, 0, true)
            .await
            .context("calculate checkpoint initialization gas")?
            .min_limit
    };
    let code_id = api
        .upload_code(WASM_BINARY)
        .await
        .map(|(code_id, _)| code_id)
        .unwrap_or_else(|_| CodeId::generate(WASM_BINARY));

    println!("Using code_id = {code_id:?}");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_bootstrap_and_genesis_pins_are_required_cli_inputs() {
        let beacon = [
            "checkpoints-tool",
            "--ethereum-beacon-rpc",
            "http://127.0.0.1:5052",
        ];
        let root = "11".repeat(32);
        assert!(Cli::try_parse_from(beacon).is_err());
        let mut bootstrap_only = beacon.to_vec();
        bootstrap_only.extend(["--trusted-bootstrap-root", root.as_str()]);
        assert!(Cli::try_parse_from(bootstrap_only.clone()).is_err());
        bootstrap_only.extend(["--trusted-genesis-validators-root", root.as_str()]);
        let parsed = Cli::try_parse_from(bootstrap_only).unwrap();
        assert_eq!(parsed.trusted_bootstrap_root, root);
        assert_eq!(parsed.trusted_genesis_validators_root, root);
    }
}
