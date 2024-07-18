use checkpoint_light_client::WASM_BINARY;
use anyhow::Error as AnyError;
use ark_bls12_381::{G1Projective as G1, G2Projective as G2};
use ark_serialize::CanonicalDeserialize;
use checkpoint_light_client_io::{
    ethereum_common::{
        base_types::{BytesFixed, FixedArray},
        beacon::{BLSPubKey, Bytes32, SignedBeaconBlockHeader, SyncAggregate, SyncCommittee},
        network::Network,
        utils as eth_utils, SLOTS_PER_EPOCH,
    },
    replay_back, sync_update,
    tree_hash::TreeHash,
    ArkScale, BeaconBlockHeader, G1TypeInfo, G2TypeInfo, Handle, HandleResult, Init,
    SyncCommitteeUpdate,
};
use gclient::{EventListener, EventProcessor, GearApi, Result};
use reqwest::{Client, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize};
use tokio::time::{self, Duration};
use std::cmp;
use parity_scale_codec::{Decode, Encode};

// https://github.com/ethereum/consensus-specs/blob/dev/specs/altair/light-client/p2p-interface.md#configuration
pub const MAX_REQUEST_LIGHT_CLIENT_UPDATES: u8 = 128;
const RPC_URL: &str = "http://127.0.0.1:5052";

const FINALITY_UPDATE_5_254_112: &[u8; 4_940] =
    include_bytes!("./sepolia-finality-update-5_254_112.json");
const FINALITY_UPDATE_5_263_072: &[u8; 4_941] =
    include_bytes!("./sepolia-finality-update-5_263_072.json");

#[derive(Deserialize)]
#[serde(untagged)]
enum LightClientHeader {
    Unwrapped(BeaconBlockHeader),
    Wrapped(Beacon),
}

#[derive(Deserialize)]
struct Beacon {
    beacon: BeaconBlockHeader,
}

#[derive(Deserialize, Debug)]
struct BeaconBlockHeaderResponse {
    data: BeaconBlockHeaderData,
}

#[derive(Deserialize, Debug)]
struct BeaconBlockHeaderData {
    header: SignedBeaconBlockHeader,
}

pub fn header_deserialize<'de, D>(deserializer: D) -> Result<BeaconBlockHeader, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let header: LightClientHeader = Deserialize::deserialize(deserializer)?;

    Ok(match header {
        LightClientHeader::Unwrapped(header) => header,
        LightClientHeader::Wrapped(header) => header.beacon,
    })
}

#[derive(Deserialize, Debug)]
pub struct Bootstrap {
    #[serde(deserialize_with = "header_deserialize")]
    pub header: BeaconBlockHeader,
    pub current_sync_committee: SyncCommittee,
    pub current_sync_committee_branch: Vec<Bytes32>,
}

#[derive(Deserialize, Debug)]
struct BootstrapResponse {
    data: Bootstrap,
}

#[derive(Deserialize)]
struct FinalityUpdateResponse {
    data: FinalityUpdate,
}

#[derive(Deserialize)]
pub struct FinalityUpdate {
    #[serde(deserialize_with = "header_deserialize")]
    pub attested_header: BeaconBlockHeader,
    #[serde(deserialize_with = "header_deserialize")]
    pub finalized_header: BeaconBlockHeader,
    pub finality_branch: Vec<Bytes32>,
    pub sync_aggregate: SyncAggregate,
    #[serde(deserialize_with = "eth_utils::deserialize_u64")]
    pub signature_slot: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    #[serde(deserialize_with = "header_deserialize")]
    pub attested_header: BeaconBlockHeader,
    pub next_sync_committee: SyncCommittee,
    pub next_sync_committee_branch: Vec<Bytes32>,
    #[serde(deserialize_with = "header_deserialize")]
    pub finalized_header: BeaconBlockHeader,
    pub finality_branch: Vec<Bytes32>,
    pub sync_aggregate: SyncAggregate,
    #[serde(deserialize_with = "eth_utils::deserialize_u64")]
    pub signature_slot: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateData {
    data: Update,
}

type UpdateResponse = Vec<UpdateData>;

async fn get<R: DeserializeOwned>(request_builder: RequestBuilder) -> Result<R> {
    let bytes = request_builder
        .send()
        .await
        .map_err(AnyError::from)?
        .bytes()
        .await
        .map_err(AnyError::from)?;

    Ok(serde_json::from_slice::<R>(&bytes).map_err(AnyError::from)?)
}

async fn get_bootstrap(client: &mut Client, checkpoint: &str) -> Result<Bootstrap> {
    let checkpoint_no_prefix = match checkpoint.starts_with("0x") {
        true => &checkpoint[2..],
        false => checkpoint,
    };

    let url = format!("{RPC_URL}/eth/v1/beacon/light_client/bootstrap/0x{checkpoint_no_prefix}",);

    get::<BootstrapResponse>(client.get(&url))
        .await
        .map(|response| response.data)
}

async fn get_finality_update(client: &mut Client) -> Result<FinalityUpdate> {
    let url = format!("{RPC_URL}/eth/v1/beacon/light_client/finality_update");

    get::<FinalityUpdateResponse>(client.get(&url))
        .await
        .map(|response| response.data)
}

async fn get_updates(client: &mut Client, period: u64, count: u8) -> Result<UpdateResponse> {
    let count = cmp::min(count, MAX_REQUEST_LIGHT_CLIENT_UPDATES);
    let url = format!(
        "{RPC_URL}/eth/v1/beacon/light_client/updates?start_period={period}&count={count}",
    );

    get::<UpdateResponse>(client.get(&url)).await
}

async fn get_block_header(client: &Client, slot: u64) -> Result<BeaconBlockHeader> {
    let url = format!("{RPC_URL}/eth/v1/beacon/headers/{slot}");

    get::<BeaconBlockHeaderResponse>(client.get(&url))
        .await
        .map(|response| response.data.header.message)
}

fn map_public_keys(compressed_public_keys: &[BLSPubKey]) -> Vec<ArkScale<G1TypeInfo>> {
    compressed_public_keys
        .iter()
        .map(|BytesFixed(pub_key_compressed)| {
            let pub_key = <G1 as CanonicalDeserialize>::deserialize_compressed_unchecked(
                &pub_key_compressed.0[..],
            )
            .unwrap();
            let ark_scale: ArkScale<G1TypeInfo> = G1TypeInfo(pub_key).into();

            ark_scale
        })
        .collect()
}

fn sync_update_from_finality(
    signature: G2,
    finality_update: FinalityUpdate,
) -> SyncCommitteeUpdate {
    SyncCommitteeUpdate {
        signature_slot: finality_update.signature_slot,
        attested_header: finality_update.attested_header,
        finalized_header: finality_update.finalized_header,
        sync_aggregate: finality_update.sync_aggregate,
        sync_committee_next_aggregate_pubkey: None,
        sync_committee_signature: G2TypeInfo(signature).into(),
        sync_committee_next_pub_keys: None,
        sync_committee_next_branch: None,
        finality_branch: finality_update
            .finality_branch
            .into_iter()
            .map(|BytesFixed(array)| array.0)
            .collect::<_>(),
    }
}

fn sync_update_from_update(update: Update) -> SyncCommitteeUpdate {
    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap();

    let next_sync_committee_keys = map_public_keys(&update.next_sync_committee.pubkeys.0);

    SyncCommitteeUpdate {
        signature_slot: update.signature_slot,
        attested_header: update.attested_header,
        finalized_header: update.finalized_header,
        sync_aggregate: update.sync_aggregate,
        sync_committee_next_aggregate_pubkey: Some(update.next_sync_committee.aggregate_pubkey),
        sync_committee_signature: G2TypeInfo(signature).into(),
        sync_committee_next_pub_keys: Some(Box::new(FixedArray(
            next_sync_committee_keys.try_into().unwrap(),
        ))),
        sync_committee_next_branch: Some(
            update
                .next_sync_committee_branch
                .into_iter()
                .map(|BytesFixed(array)| array.0)
                .collect::<_>(),
        ),
        finality_branch: update
            .finality_branch
            .into_iter()
            .map(|BytesFixed(array)| array.0)
            .collect::<_>(),
    }
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

#[tokio::test]
async fn init_and_updating() -> Result<()> {
    let mut client_http = Client::new();

    // use the latest finality header as a checkpoint for bootstrapping
    let finality_update = get_finality_update(&mut client_http).await?;
    let current_period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = get_updates(&mut client_http, current_period, 1).await?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };

    let checkpoint = update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    let bootstrap = get_bootstrap(&mut client_http, &checkpoint_hex).await?;
    let sync_update = sync_update_from_update(update);

    let pub_keys = map_public_keys(&bootstrap.current_sync_committee.pubkeys.0);
    let init = Init {
        network: Network::Sepolia,
        sync_committee_current_pub_keys: Box::new(FixedArray(pub_keys.try_into().unwrap())),
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|BytesFixed(bytes)| bytes.0)
            .collect(),
        update: sync_update,
    };

    // let client = GearApi::dev_from_path("../target/release/gear").await?;
    let client = GearApi::dev().await?;
    let mut listener = client.subscribe().await?;

    let program_id = upload_program(&client, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    for _ in 0..30 {
        let update = get_finality_update(&mut client_http).await?;

        let slot: u64 = update.finalized_header.slot;
        let current_period = eth_utils::calculate_period(slot);
        let mut updates = get_updates(&mut client_http, current_period, 1).await?;
        match updates.pop() {
            Some(update) if updates.is_empty() && update.data.finalized_header.slot >= slot => {
                println!("update sync committee");
                let payload = Handle::SyncUpdate(sync_update_from_update(update.data));
                let gas_limit = client
                    .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
                    .await?
                    .min_limit;
                println!("update gas_limit {gas_limit:?}");

                let (message_id, _) = client
                    .send_message(program_id.into(), payload, gas_limit, 0)
                    .await?;

                let (_message_id, payload, _value) = listener.reply_bytes_on(message_id).await?;
                let result_decoded = HandleResult::decode(&mut &payload.unwrap()[..]).unwrap();
                assert!(
                    matches!(result_decoded, HandleResult::SyncUpdate(result) if result.is_ok())
                );
            }

            _ => {
                println!(
                    "slot = {slot:?}, attested slot = {:?}, signature slot = {:?}",
                    update.attested_header.slot, update.signature_slot
                );
                let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
                    &update.sync_aggregate.sync_committee_signature.0 .0[..],
                );

                let Ok(signature) = signature else {
                    println!("failed to deserialize point on G2");
                    continue;
                };

                let payload = Handle::SyncUpdate(sync_update_from_finality(signature, update));

                let gas_limit = client
                    .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
                    .await?
                    .min_limit;
                println!("finality_update gas_limit {gas_limit:?}");

                let (message_id, _) = client
                    .send_message(program_id.into(), payload, gas_limit, 0)
                    .await?;

                let (_message_id, payload, _value) = listener.reply_bytes_on(message_id).await?;
                let result_decoded = HandleResult::decode(&mut &payload.unwrap()[..]).unwrap();
                assert!(
                    matches!(result_decoded, HandleResult::SyncUpdate(result) if result.is_ok())
                );
            }
        }

        println!();
        println!();

        time::sleep(Duration::from_secs(6 * 60)).await;
    }

    Ok(())
}

#[tokio::test]
async fn replaying_back() -> Result<()> {
    let mut client_http = Client::new();

    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(FINALITY_UPDATE_5_254_112).unwrap();
    let finality_update = finality_update.data;
    println!(
        "finality_update slot = {}",
        finality_update.finalized_header.slot
    );

    // This SyncCommittee operated for about 13K slots, so we make adjustments
    let current_period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = get_updates(&mut client_http, current_period - 1, 1).await?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };
    let checkpoint = update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    let bootstrap = get_bootstrap(&mut client_http, &checkpoint_hex).await?;
    println!("bootstrap slot = {}", bootstrap.header.slot);

    println!("update slot = {}", update.finalized_header.slot);
    let sync_update = sync_update_from_update(update);
    let slot_start = sync_update.finalized_header.slot;
    let slot_end = finality_update.finalized_header.slot;
    println!(
        "Replaying back from {slot_start} to {slot_end} ({} headers)",
        slot_end - slot_start
    );

    let pub_keys = map_public_keys(&bootstrap.current_sync_committee.pubkeys.0);
    let init = Init {
        network: Network::Sepolia,
        sync_committee_current_pub_keys: Box::new(FixedArray(pub_keys.try_into().unwrap())),
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|BytesFixed(bytes)| bytes.0)
            .collect(),
        update: sync_update,
    };

    // let client = GearApi::dev_from_path("../target/release/gear").await?;
    let client = GearApi::dev().await?;
    let mut listener = client.subscribe().await?;

    let program_id = upload_program(&client, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    // start to replay back
    let count_headers = 26 * SLOTS_PER_EPOCH;
    let mut requests_headers = Vec::with_capacity(count_headers as usize);
    for i in 1..count_headers {
        requests_headers.push(get_block_header(&client_http, slot_end - i));
    }

    let headers = futures::future::join_all(requests_headers)
        .await
        .into_iter()
        .filter_map(|maybe_header| maybe_header.ok())
        .collect::<Vec<_>>();

    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &finality_update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap();

    let payload = Handle::ReplayBackStart {
        sync_update: sync_update_from_finality(signature, finality_update),
        headers,
    };

    let gas_limit = client
        .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
        .await?
        .min_limit;
    println!("ReplayBackStart gas_limit {gas_limit:?}");

    let (message_id, _) = client
        .send_message(program_id.into(), payload, gas_limit, 0)
        .await?;

    let (_message_id, payload, _value) = listener.reply_bytes_on(message_id).await?;
    let result_decoded = HandleResult::decode(&mut &payload.unwrap()[..]).unwrap();
    assert!(matches!(
        result_decoded,
        HandleResult::ReplayBackStart(Ok(replay_back::StatusStart::InProgress))
    ));

    // continue to replay back
    let mut slot_end = slot_end - count_headers;
    let count_headers = 29 * SLOTS_PER_EPOCH;
    let count_batch = (slot_end - slot_start) / count_headers;

    for _batch in 0..count_batch {
        let mut requests_headers = Vec::with_capacity(count_headers as usize);
        for i in 0..count_headers {
            requests_headers.push(get_block_header(&client_http, slot_end - i));
        }

        let headers = futures::future::join_all(requests_headers)
            .await
            .into_iter()
            .filter_map(|maybe_header| maybe_header.ok())
            .collect::<Vec<_>>();

        let payload = Handle::ReplayBack(headers);

        let gas_limit = client
            .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
            .await?
            .min_limit;
        println!("ReplayBack gas_limit {gas_limit:?}");

        let (message_id, _) = client
            .send_message(program_id.into(), payload, gas_limit, 0)
            .await?;

        let (_message_id, payload, _value) = listener.reply_bytes_on(message_id).await?;
        let result_decoded = HandleResult::decode(&mut &payload.unwrap()[..]).unwrap();
        assert!(matches!(
            result_decoded,
            HandleResult::ReplayBack(Some(replay_back::Status::InProcess))
        ));

        slot_end -= count_headers;
    }

    // remaining headers
    let mut requests_headers = Vec::with_capacity(count_headers as usize);
    for i in slot_start..=slot_end {
        requests_headers.push(get_block_header(&client_http, i));
    }

    let headers = futures::future::join_all(requests_headers)
        .await
        .into_iter()
        .filter_map(|maybe_header| maybe_header.ok())
        .collect::<Vec<_>>();

    let payload = Handle::ReplayBack(headers);

    let gas_limit = client
        .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
        .await?
        .min_limit;
    println!("ReplayBack gas_limit {gas_limit:?}");

    let (message_id, _) = client
        .send_message(program_id.into(), payload, gas_limit, 0)
        .await?;

    let (_message_id, payload, _value) = listener.reply_bytes_on(message_id).await?;
    let result_decoded = HandleResult::decode(&mut &payload.unwrap()[..]).unwrap();
    assert!(matches!(
        result_decoded,
        HandleResult::ReplayBack(Some(replay_back::Status::Finished))
    ));

    Ok(())
}

#[tokio::test]
async fn sync_update_requires_replaying_back() -> Result<()> {
    let mut client_http = Client::new();

    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(FINALITY_UPDATE_5_263_072).unwrap();
    let finality_update = finality_update.data;
    println!(
        "finality_update slot = {}",
        finality_update.finalized_header.slot
    );

    let slot = finality_update.finalized_header.slot;
    let current_period = eth_utils::calculate_period(slot);
    let mut updates = get_updates(&mut client_http, current_period, 1).await?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };

    let checkpoint = update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    let bootstrap = get_bootstrap(&mut client_http, &checkpoint_hex).await?;
    let sync_update = sync_update_from_update(update);

    let pub_keys = map_public_keys(&bootstrap.current_sync_committee.pubkeys.0);
    let init = Init {
        network: Network::Sepolia,
        sync_committee_current_pub_keys: Box::new(FixedArray(pub_keys.try_into().unwrap())),
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|BytesFixed(bytes)| bytes.0)
            .collect(),
        update: sync_update,
    };

    let client = GearApi::dev().await?;
    let mut listener = client.subscribe().await?;

    let program_id = upload_program(&client, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    println!(
        "slot = {slot:?}, attested slot = {:?}, signature slot = {:?}",
        finality_update.attested_header.slot, finality_update.signature_slot
    );
    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &finality_update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap();

    let payload = Handle::SyncUpdate(sync_update_from_finality(signature, finality_update));

    let gas_limit = client
        .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
        .await?
        .min_limit;
    println!("finality_update gas_limit {gas_limit:?}");

    let (message_id, _) = client
        .send_message(program_id.into(), payload, gas_limit, 0)
        .await?;

    let (_message_id, payload, _value) = listener.reply_bytes_on(message_id).await?;
    let result_decoded = HandleResult::decode(&mut &payload.unwrap()[..]).unwrap();
    assert!(matches!(
        result_decoded,
        HandleResult::SyncUpdate(Err(sync_update::Error::ReplayBackRequired { .. }))
    ));

    Ok(())
}
