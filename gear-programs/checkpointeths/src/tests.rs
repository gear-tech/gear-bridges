use ark_bls12_381::{G1Projective as G1, G2Projective as G2};
use ark_serialize::CanonicalDeserialize;
use gclient::{EventListener, EventProcessor, GearApi, Result};
use gstd::prelude::*;
use serde::{Deserialize, de::DeserializeOwned};
use std::cmp;
use checkpointeths_io::{
    ethereum_common::{base_types::BytesFixed, beacon::{Bytes32, SyncAggregate}, utils as eth_utils}, tree_hash::TreeHash, BeaconBlockHeader, Genesis, Init, SyncCommittee
};
use crate::WASM_BINARY;
use anyhow::Error as AnyError;
use reqwest::{Client, RequestBuilder};

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

async fn get<R: DeserializeOwned>(request_builder: RequestBuilder) -> Result<R> {
    let bytes = request_builder
        .send()
        .await
        .map_err(AnyError::from)?
        .bytes()
        .await
        .map_err(AnyError::from)?;

    Ok(serde_json::from_slice::<R>(&bytes)
        .map_err(AnyError::from)?
    )
}

async fn get_bootstrap(client: &mut Client, checkpoint: &str) -> Result<Bootstrap> {
    let checkpoint_no_prefix = match checkpoint.starts_with("0x") {
        true => &checkpoint[2..],
        false => checkpoint,
    };

    let url = format!(
        "{RPC_URL}/eth/v1/beacon/light_client/bootstrap/0x{checkpoint_no_prefix}",
    );

    get::<BootstrapResponse>(client.get(&url)).await
        .map(|response| response.data)
}

async fn get_finality_update(client: &mut Client) -> Result<FinalityUpdate> {
    let url = format!("{RPC_URL}/eth/v1/beacon/light_client/finality_update");

    get::<FinalityUpdateResponse>(client.get(&url)).await
        .map(|response| response.data)
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
    let mut client = Client::new();

    // use the latest finality header as a checkpoint for bootstrapping
    let finality_update = get_finality_update(&mut client)
        .await?;
    let checkpoint = finality_update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    let bootstrap = get_bootstrap(&mut client, &checkpoint_hex).await?;

    let pub_keys = bootstrap
        .current_sync_committee
        .pubkeys
        .0
        .iter()
        .map(|pub_key_compressed| {
            <G1 as CanonicalDeserialize>::deserialize_compressed_unchecked(pub_key_compressed.as_ref()).unwrap()
        })
        .collect::<Vec<_>>();
    let init = Init {
        genesis: Genesis::Sepolia,
        finalized_header: finality_update.finalized_header,
        checkpoint,
        sync_committee_current_pub_keys: pub_keys.into(),
        sync_committee_current: bootstrap.current_sync_committee,
        sync_committee_current_branch: bootstrap.current_sync_committee_branch.into_iter().map(|BytesFixed(bytes)| bytes.0).collect(),
    };

    // let client = GearApi::dev_from_path("../target/release/gear").await?;
    let client = GearApi::dev().await?;
    let mut listener = client.subscribe().await?;

    let program_id = upload_program(
        &client,
        &mut listener,
        init,
    )
    .await?;

    println!("program_id = {:?}", hex::encode(&program_id));

    Ok(())
}
