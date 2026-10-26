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

    /// Explicit signer URI; omitted only for dry-run/read-only commands.
    #[arg(long, env = "GEAR_SURI", default_value = "")]
    gear_suri: String,

    #[clap(flatten)]
    beacon: BeaconConnectionArgs,

    /// Independently approved, recent weak-subjectivity Beacon block root (32-byte hex).
    #[arg(long, env = "TRUSTED_BOOTSTRAP_ROOT")]
    trusted_bootstrap_root: String,

    /// Independently pinned network genesis validators root (32-byte hex).
    #[arg(long, env = "TRUSTED_GENESIS_VALIDATORS_ROOT")]
    trusted_genesis_validators_root: String,

    /// Explicit independently pinned genesis time for an owned Electra consensus fixture.
    /// Requires loopback Gear and Beacon endpoints; never infers trust from their responses.
    #[arg(long)]
    owned_fixture_genesis_time: Option<u64>,

    /// Optional assertion of the trusted bootstrap sync-committee period by slot.
    #[arg(long, env = "SLOT_CHECKPOINT")]
    slot_checkpoint: Option<u64>,

    /// Specify salt for the send_recv call (hex string)
    #[arg(long, env)]
    salt: Option<String>,
}

fn is_loopback_endpoint(endpoint: &str, schemes: &[&str]) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    schemes.contains(&url.scheme())
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && match url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        }
}

fn select_network(
    trusted_genesis: [u8; 32],
    fixture_genesis_time: Option<u64>,
    gear_endpoint: &str,
    beacon_endpoint: &str,
) -> AnyResult<Network> {
    if let Some(genesis_time) = fixture_genesis_time {
        ensure!(
            is_loopback_endpoint(gear_endpoint, &["ws", "wss"])
                && is_loopback_endpoint(beacon_endpoint, &["http", "https"]),
            "owned fixture requires loopback Gear and Beacon endpoints"
        );
        ensure!(
            trusted_genesis != [0; 32]
                && Network::from_genesis_validators_root(trusted_genesis).is_none(),
            "owned fixture must have its own independently pinned genesis validators root"
        );
        Ok(Network::LocalFixture {
            genesis_time,
            genesis_validators_root: trusted_genesis.into(),
        })
    } else {
        Network::from_genesis_validators_root(trusted_genesis)
            .ok_or_else(|| anyhow!("unsupported trusted Ethereum network"))
    }
}

fn authenticate_genesis(network: &Network, root: [u8; 32], time: u64) -> AnyResult<()> {
    ensure!(
        root == network.genesis_validators_root().0 && time == network.genesis_time(),
        "Beacon provider network/genesis mismatch"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();

    let endpoint = cli.gear_connection.get_endpoint()?;

    println!("Using Gear endpoint: {endpoint}");

    let trusted_genesis: [u8; 32] =
        hex::decode(cli.trusted_genesis_validators_root.trim_start_matches("0x"))?
            .try_into()
            .map_err(|_| anyhow!("trusted genesis validators root must be 32 bytes"))?;
    let trusted_bootstrap: [u8; 32] =
        hex::decode(cli.trusted_bootstrap_root.trim_start_matches("0x"))?
            .try_into()
            .map_err(|_| anyhow!("trusted bootstrap root must be 32 bytes"))?;
    let network = select_network(
        trusted_genesis,
        cli.owned_fixture_genesis_time,
        &endpoint,
        &cli.beacon.beacon_endpoint,
    )?;
    let beacon_client = BeaconClient::new(
        cli.beacon.beacon_endpoint,
        cli.beacon.timeout.map(Duration::from_secs),
    )
    .await?;
    let genesis = beacon_client.get_genesis().await?;
    authenticate_genesis(
        &network,
        genesis
            .data
            .genesis_validators_root
            .try_into()
            .map_err(|_| anyhow!("Beacon genesis validators root must be 32 bytes"))?,
        genesis.data.genesis_time,
    )?;
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

    gear_common::signer_ownership::validate_suri(&cli.gear_suri, Some(&endpoint))?;
    let api = GearApi::builder()
        .suri(cli.gear_suri)
        .uri(endpoint)
        .build()
        .await?;

    let gas_limit = api.block_gas_limit()? / 100 * 95;
    let estimated_gas = {
        let payload = {
            let mut result = checkpoint_light_client_factory::io::Init::ROUTE.to_vec();
            init.encode_to(&mut result);

            result
        };

        api.calculate_upload_gas(None, WASM_BINARY.to_vec(), payload, 0, false)
            .await
            .context("calculate checkpoint initialization gas")?
            .min_limit
    };
    ensure!(
        estimated_gas <= gas_limit,
        "checkpoint initialization exceeds outer gas ceiling"
    );
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

    #[test]
    fn owned_fixture_is_explicit_loopback_and_genesis_bound() {
        let root = [1; 32];
        let gear = "ws://127.0.0.1:9944";
        let beacon = "http://127.0.0.1:5052";
        assert!(select_network(root, None, gear, beacon).is_err());
        let network = select_network(root, Some(1_800_000_000), gear, beacon).unwrap();
        assert!(authenticate_genesis(&network, root, 1_800_000_000).is_ok());
        assert!(authenticate_genesis(&network, [2; 32], 1_800_000_000).is_err());
        assert!(authenticate_genesis(&network, root, 1_800_000_001).is_err());
        for (gear, beacon) in [
            ("wss://testnet.vara.network", beacon),
            (gear, "https://ethereum-beacon-api.publicnode.com"),
            ("ws://127.0.0.1.evil:9944", beacon),
            (gear, "http://localhost@evil:5052"),
            (gear, "http://user@127.0.0.1:5052"),
            ("http://127.0.0.1:9944", beacon),
        ] {
            assert!(select_network(root, Some(1_800_000_000), gear, beacon).is_err());
        }
        assert!(select_network(
            root,
            Some(1_800_000_000),
            "ws://[::1]:9944",
            "http://localhost:5052"
        )
        .is_ok());
        assert!(select_network([0; 32], Some(1_800_000_000), gear, beacon).is_err());
        let hoodi = Network::Hoodi.genesis_validators_root().0;
        assert!(matches!(
            select_network(hoodi, None, gear, beacon).unwrap(),
            Network::Hoodi
        ));
        assert!(select_network(hoodi, Some(1_800_000_000), gear, beacon).is_err());
        let root_hex = hex::encode(root);
        let parsed = Cli::try_parse_from([
            "checkpoints-tool",
            "--ethereum-beacon-rpc",
            beacon,
            "--trusted-bootstrap-root",
            &root_hex,
            "--trusted-genesis-validators-root",
            &root_hex,
            "--owned-fixture-genesis-time",
            "1800000000",
        ])
        .unwrap();
        assert_eq!(parsed.owned_fixture_genesis_time, Some(1_800_000_000));
    }
}
