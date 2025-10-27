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
use ethereum_beacon_client::{utils, BeaconClient};
use gclient::{GearApi, WSAddress};
use parity_scale_codec::Encode;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use std::time::Duration;
use url::Url;

const GEAR_API_RETRIES: u8 = 3;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Perform a dry run without actual deployment
    #[arg(long, default_value_t = true, env = "DRY_RUN")]
    dry_run: bool,
    /// Address of the Gear RPC endpoint
    #[arg(
        long = "gear-url",
        default_value = "wss://testnet.vara.network:443",
        env = "GEAR_RPC"
    )]
    gear_url: Url,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = "//Alice", env = "GEAR_SURI")]
    gear_suri: String,

    /// Specify the endpoint providing Beacon API
    #[arg(
        long,
        default_value = "https://ethereum-beacon-api.publicnode.com",
        env = "BEACON_ENDPOINT"
    )]
    beacon_endpoint: String,

    /// Specify the timeout in seconds for requests to the Beacon API endpoint
    #[arg(long, default_value = "120", env = "BEACON_TIMEOUT")]
    beacon_timeout: u64,

    /// Specify the Ethereum network (Mainnet, Holesky or Sepolia)
    #[arg(long, default_value = "Mainnet", env = "NETWORK")]
    network: String,

    /// Specify the checkpoint slot for bootstrapping. If it is None then the header from
    /// the latest finality update is used to get the slot.
    #[arg(long, env = "SLOT_CHECKPOINT")]
    slot_checkpoint: Option<u64>,
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();

    let gear_host = cli
        .gear_url
        .host_str()
        .ok_or_else(|| anyhow!("Invalid Gear URL: {}", cli.gear_url))?
        .to_string();
    let gear_port = cli
        .gear_url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("Cannot determine port from Gear URL: {}", cli.gear_url))?;

    let scheme = cli.gear_url.scheme();

    let endpoint = format!("{}://{}", scheme, gear_host);

    println!("Using Gear endpoint: {}:{}", endpoint, gear_port);

    let beacon_client = BeaconClient::new(
        cli.beacon_endpoint,
        Some(Duration::from_secs(cli.beacon_timeout)),
    )
    .await?;

    let genesis = beacon_client.get_genesis().await?;

    let network = Network::from_genesis_validators_root(&genesis.data.genesis_validators_root)
        .ok_or_else(|| {
            anyhow!(
                "Failed to determine network from genesis validators root: {}",
                hex::encode(genesis.data.genesis_validators_root)
            )
        })?;

    println!("Using Ethereum network: '{:?}'", network);

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
        println!("Dry run enabled, not deploying the program.");
        return Ok(());
    }

    let api = GearApi::builder()
        .retries(GEAR_API_RETRIES)
        .suri(cli.gear_suri)
        .build(WSAddress::new(endpoint, gear_port))
        .await?;
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
    let (code_id, _) = api.upload_code(WASM_BINARY).await?;
    let factory = checkpoint_light_client_client::CheckpointLightClientFactory::new(
        GClientRemoting::new(api.clone()),
    );
    let program_id = factory
        .init(init)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .map_err(|e| anyhow!("Failed to construct program: {e:?}"))?;

    println!("program_id = {:?}", hex::encode(program_id));

    Ok(())
}
