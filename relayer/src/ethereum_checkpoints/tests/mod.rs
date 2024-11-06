use crate::ethereum_beacon_client::{slots_batch, utils, BeaconClient};
use checkpoint_light_client::WASM_BINARY;
use checkpoint_light_client_io::{
    ethereum_common::{
        base_types::BytesFixed,
        beacon::SyncAggregate,
        network::Network,
        utils::{self as eth_utils, BootstrapResponse, FinalityUpdateResponse, UpdateData},
        SLOTS_PER_EPOCH,
    },
    sync_update,
    tree_hash::TreeHash,
    Handle, HandleResult, Init, G2,
};
use gclient::{EventListener, EventProcessor, GearApi, Result};
use parity_scale_codec::{Decode, Encode};
use tokio::{
    sync::{Mutex, MutexGuard},
    time::{self, Duration},
};

static LOCK: Mutex<()> = Mutex::const_new(());

const RPC_URL: &str = "http://34.159.93.103:50000";

const FINALITY_UPDATE_5_263_072: &[u8; 4_941] =
    include_bytes!("./sepolia-finality-update-5_263_072.json");
const UPDATE_640: &[u8; 57_202] = include_bytes!("./sepolia-update-640.json");
const BOOTSTRAP_640: &[u8; 54_328] = include_bytes!("./sepolia-bootstrap-640.json");

struct Guard<'a> {
    _lock: MutexGuard<'a, ()>,
    pub client: &'a GearApi,
}

// The struct purpose is to avoid the following error:
// GearSDK(Subxt(Rpc(ClientError(Call(Custom(ErrorObject { code: ServerError(1014), message: "Priority is too low: (16 vs 16)", data: Some(RawValue("The transaction has too low priority to replace another transaction already in the pool.")) }))))))
struct NodeClient(GearApi);

impl NodeClient {
    async fn new() -> Result<Self> {
        Ok(Self(GearApi::dev().await?))
    }

    async fn calculate_handle_gas(&self, program_id: [u8; 32], payload: &Handle) -> Result<u64> {
        Ok(self
            .0
            .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
            .await?
            .min_limit)
    }

    async fn lock(&self) -> Guard<'_> {
        let _lock = LOCK.lock().await;

        Guard {
            _lock,
            client: &self.0,
        }
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

    let (_message_id, payload, _value) = {
        let lock = client.lock().await;

        let mut listener = lock.client.subscribe().await?;

        let (message_id, _) = lock
            .client
            .send_message(program_id.into(), payload, gas_limit, 0)
            .await?;

        listener.reply_bytes_on(message_id).await?
    };

    let payload = payload.map_err(|e| anyhow::anyhow!("No payload: {e:?}"))?;
    Ok((gas_limit, HandleResult::decode(&mut &payload[..])?))
}

async fn init(network: Network, rpc_url: String) -> Result<()> {
    let beacon_client = BeaconClient::new(rpc_url, None)
        .await
        .expect("Failed to connect to beacon node");

    // use the latest finality header as a checkpoint for bootstrapping
    let finality_update = beacon_client.get_finality_update().await?;
    let slot = finality_update.finalized_header.slot;
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
    let sync_update = utils::sync_update_from_update(decode(&update.sync_aggregate), update);

    println!("bootstrap slot = {}", bootstrap.header.slot);

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

    let client = NodeClient::new().await?;
    let program_id = {
        let lock = client.lock().await;

        let mut listener = lock.client.subscribe().await?;

        upload_program(lock.client, &mut listener, init).await?
    };

    println!("program_id = {:?}", hex::encode(program_id));

    Ok(())
}

#[ignore]
#[tokio::test]
async fn init_holesky() -> Result<()> {
    init(Network::Holesky, RPC_URL.into()).await
}

#[ignore]
#[tokio::test]
async fn init_mainnet() -> Result<()> {
    init(Network::Mainnet, "https://www.lightclientdata.org".into()).await
}

#[ignore]
#[tokio::test]
async fn replay_back_and_updating() -> Result<()> {
    let beacon_client = BeaconClient::new(RPC_URL.to_string(), Some(Duration::from_secs(120)))
        .await
        .expect("Failed to connect to beacon node");

    // use the latest finality header as a checkpoint for bootstrapping
    let mut finality_update = beacon_client.get_finality_update().await?;
    let current_period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = beacon_client.get_updates(current_period, 1).await?;

    println!(
        "finality_update slot = {}, period = {}",
        finality_update.finalized_header.slot, current_period
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
    let sync_update = utils::sync_update_from_update(decode(&update.sync_aggregate), update);
    let slot_start = sync_update.finalized_header.slot;

    println!("bootstrap slot = {}", bootstrap.header.slot);

    let pub_keys = utils::map_public_keys(&bootstrap.current_sync_committee.pubkeys);
    let init = Init {
        network: Network::Holesky,
        sync_committee_current_pub_keys: pub_keys,
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|BytesFixed(bytes)| bytes.0)
            .collect(),
        update: sync_update,
    };

    let client = NodeClient::new().await?;
    let program_id = {
        let lock = client.lock().await;

        let mut listener = lock.client.subscribe().await?;

        upload_program(lock.client, &mut listener, init).await?
    };

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    // ensure finality update is different from the sync update
    loop {
        if finality_update.finalized_header.slot != slot_start {
            break;
        }

        time::sleep(Duration::from_secs(6)).await;
        finality_update = beacon_client.get_finality_update().await?;
    }

    // start to replay back
    let mut slot_last = finality_update.finalized_header.slot;
    let mut slots_batch_iter =
        slots_batch::Iter::new(slot_start, slot_last, 30 * SLOTS_PER_EPOCH).unwrap();
    if let Some((slot_start, slot_end)) = slots_batch_iter.next() {
        let payload = Handle::ReplayBackStart {
            sync_update: utils::sync_update_from_finality(
                decode(&finality_update.sync_aggregate),
                finality_update,
            ),
            headers: beacon_client.request_headers(slot_start, slot_end).await?,
        };

        let (gas_limit, result) = calculate_gas_and_send(program_id, payload, &client).await?;
        println!("ReplayBackStart gas_limit {gas_limit:?}");

        assert!(
            matches!(result, HandleResult::ReplayBackStart(Ok(_))),
            "result = {result:?}"
        );
    }

    // replaying the blocks back
    for (slot_start, slot_end) in slots_batch_iter {
        let payload =
            Handle::ReplayBack(beacon_client.request_headers(slot_start, slot_end).await?);
        let (gas_limit, result) = calculate_gas_and_send(program_id, payload, &client).await?;
        println!("ReplayBack gas_limit {gas_limit:?}");

        assert!(
            matches!(result, HandleResult::ReplayBack(Some(_))),
            "result = {result:?}"
        );
    }

    // updating
    const COUNT: usize = 1;
    let mut processed = 0;
    while processed < COUNT {
        time::sleep(Duration::from_secs(6)).await;

        let update = beacon_client.get_finality_update().await?;
        let slot: u64 = update.finalized_header.slot;
        if slot == slot_last {
            continue;
        }

        slot_last = slot;
        processed += 1;

        let current_period = eth_utils::calculate_period(slot);
        let mut updates = beacon_client.get_updates(current_period, 1).await?;
        let payload = Handle::SyncUpdate(match updates.pop() {
            Some(update) if updates.is_empty() && update.data.finalized_header.slot >= slot => {
                println!("update sync committee");

                utils::sync_update_from_update(decode(&update.data.sync_aggregate), update.data)
            }

            _ => {
                println!(
                    "slot = {slot:?}, attested slot = {:?}, signature slot = {:?}",
                    update.attested_header.slot, update.signature_slot
                );

                utils::sync_update_from_finality(decode(&update.sync_aggregate), update)
            }
        });

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

#[ignore]
#[tokio::test]
async fn sync_update_requires_replaying_back() -> Result<()> {
    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(FINALITY_UPDATE_5_263_072).unwrap();
    let finality_update = finality_update.data;
    println!(
        "finality_update slot = {}",
        finality_update.finalized_header.slot
    );

    let slot = finality_update.finalized_header.slot;
    let mut updates: Vec<UpdateData> = serde_json::from_slice(UPDATE_640).unwrap();

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };

    let BootstrapResponse { data: bootstrap } = serde_json::from_slice(BOOTSTRAP_640).unwrap();

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
    let init = Init {
        network: Network::Sepolia,
        sync_committee_current_pub_keys: pub_keys,
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|BytesFixed(bytes)| bytes.0)
            .collect(),
        update: sync_update,
    };

    let client = NodeClient::new().await?;
    let program_id = {
        let lock = client.lock().await;

        let mut listener = lock.client.subscribe().await?;

        upload_program(lock.client, &mut listener, init).await?
    };

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
