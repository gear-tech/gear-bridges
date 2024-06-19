use crate::WASM_BINARY;
use anyhow::Error as AnyError;
use ark_bls12_381::{G1Projective as G1, G2Projective as G2};
use ark_serialize::CanonicalDeserialize;
use checkpointeths_io::{
    ethereum_common::{
        base_types::{BytesFixed, FixedArray},
        beacon::{BLSPubKey, Bytes32, SyncAggregate},
        utils as eth_utils,
    },
    tree_hash::TreeHash,
    ArkScale, BeaconBlockHeader, G1TypeInfo, G2TypeInfo, Genesis, Handle, Init, SyncCommittee,
    SyncUpdate,
};
use gclient::{EventListener, EventProcessor, GearApi, Result};
use gstd::prelude::*;
use reqwest::{Client, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize};
use std::cmp;
use tokio::time::{self, Duration};

// https://github.com/ethereum/consensus-specs/blob/dev/specs/altair/light-client/p2p-interface.md#configuration
pub const MAX_REQUEST_LIGHT_CLIENT_UPDATES: u8 = 128;
const RPC_URL: &str = "http://127.0.0.1:5052";

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

#[derive(Deserialize)]
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

#[derive(Deserialize)]
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

fn map_public_keys(compressed_public_keys: &[BLSPubKey]) -> Vec<ArkScale<G1TypeInfo>> {
    compressed_public_keys.iter()
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

fn create_sync_update(update: Update) -> SyncUpdate {
    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap();

    let next_sync_committee_keys = map_public_keys(&update
        .next_sync_committee
        .pubkeys
        .0);

    SyncUpdate {
        signature_slot: update.signature_slot,
        attested_header: update.attested_header,
        finalized_header: update.finalized_header,
        sync_aggregate: update.sync_aggregate,
        sync_committee_next: Some(Box::new(update.next_sync_committee)),
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
async fn ethereum_light_client() -> Result<()> {
    let mut client_http = Client::new();

    // use the latest finality header as a checkpoint for bootstrapping
    let finality_update = get_finality_update(&mut client_http).await?;
    let current_period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = get_updates(
        &mut client_http,
        current_period,
        1,
    )
    .await?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };

    let checkpoint = update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    let bootstrap = get_bootstrap(&mut client_http, &checkpoint_hex).await?;
    let sync_update = create_sync_update(update);

    let pub_keys = map_public_keys(&bootstrap
        .current_sync_committee
        .pubkeys
        .0);
    let init = Init {
        genesis: Genesis::Sepolia,
        sync_committee_current_pub_keys: Box::new(FixedArray(pub_keys.try_into().unwrap())),
        sync_committee_current: bootstrap.current_sync_committee,
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

    println!("program_id = {:?}", hex::encode(&program_id));

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
                let payload = Handle::SyncUpdate(create_sync_update(update.data));
                let gas_limit = client
                    .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
                    .await?
                    .min_limit;
                println!("update gas_limit {gas_limit:?}");

                let (message_id, _) = client
                    .send_message(program_id.into(), payload, gas_limit, 0)
                    .await?;

                assert!(listener.message_processed(message_id).await?.succeed());
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

                let payload = Handle::SyncUpdate(SyncUpdate {
                    signature_slot: update.signature_slot,
                    attested_header: update.attested_header,
                    finalized_header: update.finalized_header,
                    sync_aggregate: update.sync_aggregate,
                    sync_committee_next: None,
                    sync_committee_signature: G2TypeInfo(signature).into(),
                    sync_committee_next_pub_keys: None,
                    sync_committee_next_branch: None,
                    finality_branch: update
                        .finality_branch
                        .into_iter()
                        .map(|BytesFixed(array)| array.0)
                        .collect::<_>(),
                });

                let gas_limit = client
                    .calculate_handle_gas(None, program_id.into(), payload.encode(), 0, true)
                    .await?
                    .min_limit;
                println!("finality_update gas_limit {gas_limit:?}");

                let (message_id, _) = client
                    .send_message(program_id.into(), payload, gas_limit, 0)
                    .await?;

                assert!(listener.message_processed(message_id).await?.succeed());
            }
        }

        println!();
        println!();

        time::sleep(Duration::from_secs(6 * 60)).await;
    }

    Ok(())
}
