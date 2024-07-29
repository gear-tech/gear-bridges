use anyhow::{Error as AnyError, Result as AnyResult};
use ark_serialize::CanonicalDeserialize;
use checkpoint_light_client_io::{
    ethereum_common::{
        base_types::{BytesFixed, FixedArray},
        beacon::{BLSPubKey, Bytes32, SignedBeaconBlockHeader, SyncAggregate, SyncCommittee},
        utils as eth_utils,
    },
    ArkScale, BeaconBlockHeader, G1TypeInfo, G2TypeInfo, SyncCommitteeKeys, SyncCommitteeUpdate,
    G1, G2, SYNC_COMMITTEE_SIZE,
};
use reqwest::{Client, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize};
use std::{cmp, error::Error, fmt};

pub mod slots_batch;

// https://github.com/ethereum/consensus-specs/blob/dev/specs/altair/light-client/p2p-interface.md#configuration
pub const MAX_REQUEST_LIGHT_CLIENT_UPDATES: u8 = 128;

#[derive(Deserialize)]
#[serde(untagged)]
pub enum LightClientHeader {
    Unwrapped(BeaconBlockHeader),
    Wrapped(Beacon),
}

#[derive(Deserialize)]
pub struct Beacon {
    pub beacon: BeaconBlockHeader,
}

#[derive(Deserialize, Debug)]
pub struct BeaconBlockHeaderResponse {
    pub data: BeaconBlockHeaderData,
}

#[derive(Deserialize, Debug)]
pub struct BeaconBlockHeaderData {
    pub header: SignedBeaconBlockHeader,
}

#[derive(Deserialize, Debug)]
pub struct Bootstrap {
    #[serde(deserialize_with = "deserialize_header")]
    pub header: BeaconBlockHeader,
    pub current_sync_committee: SyncCommittee,
    pub current_sync_committee_branch: Vec<Bytes32>,
}

#[derive(Deserialize, Debug)]
pub struct BootstrapResponse {
    pub data: Bootstrap,
}

pub fn deserialize_header<'de, D>(deserializer: D) -> Result<BeaconBlockHeader, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let header: LightClientHeader = Deserialize::deserialize(deserializer)?;

    Ok(match header {
        LightClientHeader::Unwrapped(header) => header,
        LightClientHeader::Wrapped(header) => header.beacon,
    })
}

#[derive(Deserialize)]
pub struct FinalityUpdateResponse {
    pub data: FinalityUpdate,
}

#[derive(Clone, Deserialize)]
pub struct FinalityUpdate {
    #[serde(deserialize_with = "deserialize_header")]
    pub attested_header: BeaconBlockHeader,
    #[serde(deserialize_with = "deserialize_header")]
    pub finalized_header: BeaconBlockHeader,
    pub finality_branch: Vec<Bytes32>,
    pub sync_aggregate: SyncAggregate,
    #[serde(deserialize_with = "eth_utils::deserialize_u64")]
    pub signature_slot: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    #[serde(deserialize_with = "deserialize_header")]
    pub attested_header: BeaconBlockHeader,
    pub next_sync_committee: SyncCommittee,
    pub next_sync_committee_branch: Vec<Bytes32>,
    #[serde(deserialize_with = "deserialize_header")]
    pub finalized_header: BeaconBlockHeader,
    pub finality_branch: Vec<Bytes32>,
    pub sync_aggregate: SyncAggregate,
    #[serde(deserialize_with = "eth_utils::deserialize_u64")]
    pub signature_slot: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateData {
    pub data: Update,
}

pub type UpdateResponse = Vec<UpdateData>;

#[derive(Clone, Debug)]
pub struct ErrorNotFound;

impl fmt::Display for ErrorNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt("Not found (404)", f)
    }
}

impl Error for ErrorNotFound {}

#[derive(Deserialize)]
struct CodeResponse {
    code: u64,
    message: String,
}

pub async fn get<R: DeserializeOwned>(request_builder: RequestBuilder) -> AnyResult<R> {
    let bytes = request_builder
        .send()
        .await
        .map_err(AnyError::from)?
        .bytes()
        .await
        .map_err(AnyError::from)?;

    match serde_json::from_slice::<CodeResponse>(&bytes) {
        Ok(code_response) if code_response.code == 404 => Err(ErrorNotFound.into()),
        _ => Ok(serde_json::from_slice::<R>(&bytes).map_err(AnyError::from)?),
    }
}

pub async fn get_bootstrap(
    client: &Client,
    rpc_url: &str,
    checkpoint: &str,
) -> AnyResult<Bootstrap> {
    let checkpoint_no_prefix = match checkpoint.starts_with("0x") {
        true => &checkpoint[2..],
        false => checkpoint,
    };

    let url = format!("{rpc_url}/eth/v1/beacon/light_client/bootstrap/0x{checkpoint_no_prefix}",);

    get::<BootstrapResponse>(client.get(&url))
        .await
        .map(|response| response.data)
}

pub async fn get_updates(
    client: &Client,
    rpc_url: &str,
    period: u64,
    count: u8,
) -> AnyResult<UpdateResponse> {
    let count = cmp::min(count, MAX_REQUEST_LIGHT_CLIENT_UPDATES);
    let url = format!(
        "{rpc_url}/eth/v1/beacon/light_client/updates?start_period={period}&count={count}",
    );

    get::<UpdateResponse>(client.get(&url)).await
}

pub async fn get_block_header(
    client: &Client,
    rpc_url: &str,
    slot: u64,
) -> AnyResult<BeaconBlockHeader> {
    let url = format!("{rpc_url}/eth/v1/beacon/headers/{slot}");

    get::<BeaconBlockHeaderResponse>(client.get(&url))
        .await
        .map(|response| response.data.header.message)
}

pub async fn get_finality_update(client: &Client, rpc_url: &str) -> AnyResult<FinalityUpdate> {
    let url = format!("{rpc_url}/eth/v1/beacon/light_client/finality_update");

    get::<FinalityUpdateResponse>(client.get(&url))
        .await
        .map(|response| response.data)
}

pub fn map_public_keys(
    compressed_public_keys: &FixedArray<BLSPubKey, SYNC_COMMITTEE_SIZE>,
) -> Box<SyncCommitteeKeys> {
    let keys = compressed_public_keys
        .0
        .iter()
        .map(|BytesFixed(pub_key_compressed)| {
            let pub_key = <G1 as CanonicalDeserialize>::deserialize_compressed_unchecked(
                &pub_key_compressed.0[..],
            )
            .expect("Public keys have the required size");

            let ark_scale: ArkScale<G1TypeInfo> = G1TypeInfo(pub_key).into();

            ark_scale
        })
        .collect::<Vec<_>>();

    Box::new(FixedArray(keys.try_into().expect(
        "The size of keys array is guaranteed on the type level",
    )))
}

pub fn sync_update_from_finality(
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

pub fn sync_update_from_update(signature: G2, update: Update) -> SyncCommitteeUpdate {
    let next_sync_committee_keys = map_public_keys(&update.next_sync_committee.pubkeys);

    SyncCommitteeUpdate {
        signature_slot: update.signature_slot,
        attested_header: update.attested_header,
        finalized_header: update.finalized_header,
        sync_aggregate: update.sync_aggregate,
        sync_committee_next_aggregate_pubkey: Some(update.next_sync_committee.aggregate_pubkey),
        sync_committee_signature: G2TypeInfo(signature).into(),
        sync_committee_next_pub_keys: Some(next_sync_committee_keys),
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
