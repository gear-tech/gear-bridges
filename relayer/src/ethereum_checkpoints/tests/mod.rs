use crate::ethereum_beacon_client::{utils, BeaconClient};
use checkpoint_light_client::WASM_BINARY;
use checkpoint_light_client_io::{
    ethereum_common::{
        base_types::BytesFixed,
        beacon::SyncAggregate,
        network::Network,
        utils::{self as eth_utils, BootstrapResponse, FinalityUpdateResponse, UpdateData},
        SLOTS_PER_EPOCH,
    },
    replay_back::Status,
    sync_update,
    tree_hash::TreeHash,
    Handle, HandleResult, Init, G2,
};
use ethereum_common::utils::{BeaconBlockHeaderResponse, Bootstrap, Update};
use gclient::{EventListener, EventProcessor, GearApi, Result, WSAddress};
use parity_scale_codec::{Decode, Encode};
use ruzstd::StreamingDecoder;
use sp_core::crypto::DEV_PHRASE;
use std::io::Read;
use tokio::{sync::Mutex, time::Duration};

static LOCK: Mutex<u32> = Mutex::const_new(0);

const SEPOLIA_FINALITY_UPDATE_5_263_072: &[u8; 4_941] =
    include_bytes!("./sepolia-finality-update-5_263_072.json");
const SEPOLIA_UPDATE_640: &[u8; 57_202] = include_bytes!("./sepolia-update-640.json");
const SEPOLIA_BOOTSTRAP_640: &[u8; 54_328] = include_bytes!("./sepolia-bootstrap-640.json");

const HOLESKY_UPDATE_368: &[u8; 30_468] = include_bytes!("./holesky-update-368.json.zst");
const HOLESKY_BOOTSTRAP_368: &[u8; 29_297] = include_bytes!("./holesky-bootstrap-368.json.zst");
const HOLESKY_HEADERS: &[u8; 452_109] = include_bytes!("./headers.json.zst");
const HOLESKY_FINALITY_UPDATE_3_014_736: &[u8; 4_893] =
    include_bytes!("./holesky-finality-update-3_016_736.json");
const HOLESKY_FINALITY_UPDATE_3_014_768: &[u8; 4_932] =
    include_bytes!("./holesky-finality-update-3_016_768.json");
const HOLESKY_FINALITY_UPDATE_3_014_799: &[u8; 4_980] =
    include_bytes!("./holesky-finality-update-3_016_799.json");

struct NodeClient(pub GearApi);

impl NodeClient {
    async fn new() -> Result<Self> {
        let api = GearApi::dev().await?;
        let mut lock = LOCK.lock().await;

        let salt = *lock;
        *lock += 1;

        let suri = format!("{DEV_PHRASE}//ethereum_checkpoints{salt}:");
        let api2 = GearApi::init_with(WSAddress::dev(), suri).await?;

        let account_id: &[u8; 32] = api2.account_id().as_ref();
        api.transfer_keep_alive((*account_id).into(), 100_000_000_000_000)
            .await?;

        Ok(Self(api2))
    }

    async fn calculate_handle_gas(&self, program_id: [u8; 32], payload: &Handle) -> Result<u64> {
        Ok(self
            .0
            .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
            .await?
            .min_limit)
    }
}

#[track_caller]
fn decode(sync_aggregate: &SyncAggregate) -> G2 {
    <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap()
}

async fn common_upload_program(
    client: &GearApi,
    code: Vec<u8>,
    payload: impl Encode,
) -> Result<([u8; 32], [u8; 32])> {
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
) -> Result<[u8; 32]> {
    let (message_id, program_id) =
        common_upload_program(client, WASM_BINARY.to_vec(), payload).await?;

    assert!(listener
        .message_processed(message_id.into())
        .await?
        .succeed());

    Ok(program_id)
}

async fn calculate_gas_and_send(
    program_id: [u8; 32],
    payload: Handle,
    client: &NodeClient,
) -> Result<(u64, HandleResult)> {
    let gas_limit = client.calculate_handle_gas(program_id, &payload).await?;

    let mut listener = client.0.subscribe().await?;
    let (message_id, _) = client
        .0
        .send_message(program_id.into(), payload, gas_limit, 0)
        .await?;

    let (_message_id, payload, _value) = listener.reply_bytes_on(message_id).await?;
    let payload = payload.map_err(|e| anyhow::anyhow!("No payload: {e:?}"))?;

    Ok((gas_limit, HandleResult::decode(&mut &payload[..])?))
}

fn get_bootstrap_and_update() -> (Bootstrap, Update) {
    let mut decoder = StreamingDecoder::new(&HOLESKY_BOOTSTRAP_368[..]).unwrap();
    let mut bootstrap = Vec::new();
    decoder.read_to_end(&mut bootstrap).unwrap();
    let BootstrapResponse { data: bootstrap } = serde_json::from_slice(&bootstrap[..]).unwrap();

    let mut decoder = StreamingDecoder::new(&HOLESKY_UPDATE_368[..]).unwrap();
    let mut update = Vec::new();
    decoder.read_to_end(&mut update).unwrap();
    let mut updates: Vec<UpdateData> = serde_json::from_slice(&update[..]).unwrap();

    (bootstrap, updates.pop().map(|u| u.data).unwrap())
}

fn construct_init(network: Network, update: Update, bootstrap: Bootstrap) -> Init {
    let checkpoint_update = update.finalized_header.tree_hash_root();
    let checkpoint_bootstrap = bootstrap.header.tree_hash_root();
    assert_eq!(
        checkpoint_update,
        checkpoint_bootstrap,
        "checkpoint_update = {}, checkpoint_bootstrap = {}",
        hex::encode(checkpoint_update),
        hex::encode(checkpoint_bootstrap)
    );

    let sync_update = utils::sync_update_from_update(decode(&update.sync_aggregate), update);
    let pub_keys = utils::map_public_keys(&bootstrap.current_sync_committee.pubkeys);

    Init {
        network,
        sync_committee_current_pub_keys: pub_keys,
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|BytesFixed(bytes)| bytes.0)
            .collect(),
        update: sync_update,
    }
}

async fn live_init(network: Network, rpc_url: String) -> Result<()> {
    let beacon_client = BeaconClient::new(rpc_url, Some(Duration::from_secs(120)))
        .await
        .expect("Failed to connect to beacon node");

    // use the latest finalized block as a checkpoint for bootstrapping
    let finalized_block = beacon_client.get_block_finalized().await?;
    let slot = finalized_block.slot;
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

    let client = NodeClient::new().await?;
    let mut listener = client.0.subscribe().await?;
    let init = construct_init(network, update, bootstrap);
    let program_id = upload_program(&client.0, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    Ok(())
}

#[ignore]
#[tokio::test]
async fn live_init_holesky() -> Result<()> {
    live_init(Network::Holesky, "http://34.159.93.103:50000".to_string()).await
}

#[ignore]
#[tokio::test]
async fn live_init_mainnet() -> Result<()> {
    live_init(
        Network::Mainnet,
        "https://www.lightclientdata.org".to_string(),
    )
    .await
}

#[tokio::test]
async fn init_holesky() -> Result<()> {
    let (bootstrap, update) = get_bootstrap_and_update();
    let client = NodeClient::new().await?;
    let mut listener = client.0.subscribe().await?;
    let init = construct_init(Network::Holesky, update, bootstrap);
    let program_id = upload_program(&client.0, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    Ok(())
}

#[tokio::test]
async fn replay_back_and_updating() -> Result<()> {
    let client = NodeClient::new().await?;
    let mut listener = client.0.subscribe().await?;
    let (bootstrap, update) = get_bootstrap_and_update();
    let init = construct_init(Network::Holesky, update, bootstrap);
    let program_id = upload_program(&client.0, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(HOLESKY_FINALITY_UPDATE_3_014_736).unwrap();
    let finality_update = finality_update.data;

    let mut decoder = StreamingDecoder::new(&HOLESKY_HEADERS[..]).unwrap();
    let mut headers = Vec::new();
    decoder.read_to_end(&mut headers).unwrap();

    let headers: Vec<BeaconBlockHeaderResponse> = serde_json::from_slice(&headers[..]).unwrap();

    // start to replay back
    let size_batch = 40 * SLOTS_PER_EPOCH as usize;
    let payload = Handle::ReplayBackStart {
        sync_update: utils::sync_update_from_finality(
            decode(&finality_update.sync_aggregate),
            finality_update,
        ),
        headers: headers
            .iter()
            .rev()
            .take(size_batch)
            .map(|r| r.data.header.message.clone())
            .collect(),
    };

    let (gas_limit, result) = calculate_gas_and_send(program_id, payload, &client).await?;
    println!("ReplayBackStart gas_limit {gas_limit:?}");

    assert!(
        matches!(result, HandleResult::ReplayBackStart(Ok(_))),
        "result = {result:?}"
    );

    // replaying the blocks back
    let payload = Handle::ReplayBack(
        headers
            .iter()
            .rev()
            .skip(size_batch)
            .map(|r| r.data.header.message.clone())
            .collect(),
    );
    let (gas_limit, result) = calculate_gas_and_send(program_id, payload, &client).await?;
    println!("ReplayBack gas_limit {gas_limit:?}");

    assert!(
        matches!(result, HandleResult::ReplayBack(Some(Status::Finished))),
        "result = {result:?}"
    );

    // updating
    let finality_updates = vec![
        {
            let finality_update: FinalityUpdateResponse =
                serde_json::from_slice(HOLESKY_FINALITY_UPDATE_3_014_768).unwrap();

            finality_update.data
        },
        {
            let finality_update: FinalityUpdateResponse =
                serde_json::from_slice(HOLESKY_FINALITY_UPDATE_3_014_799).unwrap();

            finality_update.data
        },
    ];
    for update in finality_updates {
        println!(
            "slot = {:?}, attested slot = {:?}, signature slot = {:?}",
            update.finalized_header.slot, update.attested_header.slot, update.signature_slot
        );

        let payload = Handle::SyncUpdate(utils::sync_update_from_finality(
            decode(&update.sync_aggregate),
            update,
        ));

        let (gas_limit, result) = calculate_gas_and_send(program_id, payload, &client).await?;
        println!("gas_limit {gas_limit:?}");

        assert!(
            matches!(
                result,
                HandleResult::SyncUpdate(Ok(_) | Err(sync_update::Error::LowVoteCount))
            ),
            "result = {result:?}"
        );

        println!();
        println!();
    }

    Ok(())
}

#[tokio::test]
async fn sync_update_requires_replaying_back() -> Result<()> {
    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(SEPOLIA_FINALITY_UPDATE_5_263_072).unwrap();
    let finality_update = finality_update.data;
    println!(
        "finality_update slot = {}",
        finality_update.finalized_header.slot
    );

    let slot = finality_update.finalized_header.slot;

    let BootstrapResponse { data: bootstrap } =
        serde_json::from_slice(SEPOLIA_BOOTSTRAP_640).unwrap();
    let mut updates: Vec<UpdateData> = serde_json::from_slice(SEPOLIA_UPDATE_640).unwrap();
    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };

    let client = NodeClient::new().await?;
    let mut listener = client.0.subscribe().await?;
    let init = construct_init(Network::Sepolia, update, bootstrap);
    let program_id = upload_program(&client.0, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    println!(
        "slot = {slot:?}, attested slot = {:?}, signature slot = {:?}",
        finality_update.attested_header.slot, finality_update.signature_slot
    );

    let payload = Handle::SyncUpdate(utils::sync_update_from_finality(
        decode(&finality_update.sync_aggregate),
        finality_update,
    ));
    let (gas_limit, result) = calculate_gas_and_send(program_id, payload, &client).await?;
    println!("finality_update gas_limit {gas_limit:?}");

    assert!(
        matches!(
            result,
            HandleResult::SyncUpdate(Err(sync_update::Error::ReplayBackRequired { .. }))
        ),
        "result = {result:?}"
    );

    Ok(())
}
