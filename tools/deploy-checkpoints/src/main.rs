use anyhow::{anyhow, Result as AnyResult};
use checkpoint_light_client::WASM_BINARY;
use checkpoint_light_client_io::{
    ethereum_common::{base_types::BytesFixed, network::Network, utils as eth_utils},
    tree_hash::TreeHash,
    Init, G2,
};
use clap::Parser;
use ethereum_beacon_client::{utils, BeaconClient};
use gclient::{EventListener, EventProcessor, GearApi, WSAddress};
use parity_scale_codec::Encode;
use std::time::Duration;

const GEAR_API_RETRIES: u8 = 3;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Address of the Gear RPC endpoint
    #[arg(
        long = "gear-endpoint",
        default_value = "wss://testnet.vara.network",
        env = "GEAR_RPC"
    )]
    gear_endpoint: String,

    /// Port of the Gear RPC endpoint
    #[arg(long = "gear-port", default_value = "443", env = "GEAR_PORT")]
    gear_port: u16,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = "//Alice", env = "GEAR_SURI")]
    gear_suri: String,

    /// Specify the endpoint providing Beacon API
    #[arg(
        long,
        default_value = "https://www.lightclientdata.org",
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
    let network = cli.network.to_lowercase();
    let network = if network == "mainnet" {
        Network::Mainnet
    } else if network == "holesky" {
        Network::Holesky
    } else if network == "sepolia" {
        Network::Sepolia
    } else {
        return Err(anyhow!("Network '{network}' is not supported"));
    };

    let beacon_client = BeaconClient::new(
        cli.beacon_endpoint,
        Some(Duration::from_secs(cli.beacon_timeout)),
    )
    .await?;

    let slot = match cli.slot_checkpoint {
        Some(slot) => slot,
        None => {
            let update = beacon_client.get_finality_update().await?;

            update.finalized_header.slot
        }
    };
    let current_period = eth_utils::calculate_period(slot);
    let mut updates = beacon_client.get_updates(current_period, 1).await?;

    println!(
        "finality_update slot = {}, period = {}",
        slot, current_period
    );

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
    };

    let client = GearApi::builder()
        .retries(GEAR_API_RETRIES)
        .suri(cli.gear_suri)
        .build(WSAddress::new(&cli.gear_endpoint, cli.gear_port))
        .await?;
    let mut listener = client.subscribe().await?;
    let program_id = upload_program(&client, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    Ok(())
}

async fn common_upload_program(
    client: &GearApi,
    code: Vec<u8>,
    payload: impl Encode,
) -> AnyResult<([u8; 32], [u8; 32])> {
    let encoded_payload = payload.encode();
    let gas_limit = client
        .calculate_upload_gas(None, code.clone(), encoded_payload, 0, true)
        .await?
        .min_limit;
    println!("init gas {gas_limit:?}");
    let (message_id, program_id, _) = client
        .upload_program(
            code,
            gclient::now_micros().to_le_bytes(),
            payload,
            gas_limit,
            0,
        )
        .await?;

    Ok((message_id.into(), program_id.into()))
}

async fn upload_program(
    client: &GearApi,
    listener: &mut EventListener,
    payload: impl Encode,
) -> AnyResult<[u8; 32]> {
    let (message_id, program_id) =
        common_upload_program(client, WASM_BINARY.to_vec(), payload).await?;

    assert!(listener
        .message_processed(message_id.into())
        .await?
        .succeed());

    Ok(program_id)
}
