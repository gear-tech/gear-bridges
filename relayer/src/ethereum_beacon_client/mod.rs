use std::{
    cmp::{self, Ordering},
    error::Error,
    fmt,
    time::Duration,
};

use anyhow::{anyhow, Error as AnyError, Result as AnyResult};
use checkpoint_light_client_io::{BeaconBlockHeader, Slot};
use ethereum_common::{
    beacon::Block as BeaconBlock,
    utils::{
        BeaconBlockHeaderResponse, BeaconBlockResponse, FinalityUpdate, FinalityUpdateResponse,
        UpdateResponse,
    },
    MAX_REQUEST_LIGHT_CLIENT_UPDATES,
};
use reqwest::{Client, ClientBuilder, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize};

pub mod slots_batch;
pub mod utils;

#[derive(Clone, Debug)]
pub struct ErrorNotFound;

impl fmt::Display for ErrorNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt("Not found (404)", f)
    }
}

impl Error for ErrorNotFound {}

#[allow(dead_code)]
#[derive(Deserialize)]
struct CodeResponse {
    code: u64,
    message: String,
}

#[derive(Clone)]
pub struct BeaconClient {
    client: Client,
    rpc_url: String,
}

impl BeaconClient {
    pub async fn new(rpc_url: String, timeout: Option<Duration>) -> AnyResult<Self> {
        let client = ClientBuilder::new();
        let client = match timeout {
            Some(timeout) => client.timeout(timeout),
            None => client,
        };

        let client = client
            .build()
            .expect("Failed to create reqwest http client");

        Ok(Self { client, rpc_url })
    }

    pub async fn get_updates(&self, period: u64, count: u8) -> AnyResult<UpdateResponse> {
        let count = cmp::min(count, MAX_REQUEST_LIGHT_CLIENT_UPDATES);
        let url = format!(
            "{}/eth/v1/beacon/light_client/updates?start_period={}&count={}",
            self.rpc_url, period, count
        );

        get::<UpdateResponse>(self.client.get(&url)).await
    }

    pub async fn get_block_header(&self, slot: u64) -> AnyResult<BeaconBlockHeader> {
        let url = format!("{}/eth/v1/beacon/headers/{}", self.rpc_url, slot);

        get::<BeaconBlockHeaderResponse>(self.client.get(&url))
            .await
            .map(|response| response.data.header.message)
    }

    pub async fn get_block_finalized(&self) -> AnyResult<BeaconBlock> {
        let url = format!("{}/eth/v2/beacon/blocks/finalized", self.rpc_url);

        get::<BeaconBlockResponse>(self.client.get(&url))
            .await
            .map(|response| response.data.message)
    }

    pub async fn get_block(&self, slot: u64) -> AnyResult<BeaconBlock> {
        let url = format!("{}/eth/v2/beacon/blocks/{}", self.rpc_url, slot);

        get::<BeaconBlockResponse>(self.client.get(&url))
            .await
            .map(|response| response.data.message)
    }

    pub async fn get_block_by_hash(&self, hash: &[u8; 32]) -> AnyResult<BeaconBlock> {
        let mut hex_encoded = [0u8; 66];
        hex_encoded[0] = b'0';
        hex_encoded[1] = b'x';

        hex::encode_to_slice(hash, &mut hex_encoded[2..]).expect("The buffer has the right size");
        let url = format!(
            "{}/eth/v2/beacon/blocks/{}",
            self.rpc_url,
            String::from_utf8_lossy(&hex_encoded)
        );

        get::<BeaconBlockResponse>(self.client.get(&url))
            .await
            .map(|response| response.data.message)
    }

    pub async fn get_finality_update(&self) -> AnyResult<FinalityUpdate> {
        let url = format!(
            "{}/eth/v1/beacon/light_client/finality_update",
            self.rpc_url
        );

        get::<FinalityUpdateResponse>(self.client.get(&url))
            .await
            .map(|response| response.data)
    }

    pub async fn request_headers(
        &self,
        slot_start: Slot,
        slot_end: Slot,
    ) -> AnyResult<Vec<BeaconBlockHeader>> {
        let batch_size = (slot_end - slot_start) as usize;
        let mut requests_headers = Vec::with_capacity(batch_size);
        for i in slot_start..slot_end {
            requests_headers.push(self.get_block_header(i));
        }

        futures::future::join_all(requests_headers)
            .await
            .into_iter()
            .filter(|maybe_header| !matches!(maybe_header, Err(e) if e.downcast_ref::<ErrorNotFound>().is_some()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                anyhow!("Failed to fetch block headers ([{slot_start}; {slot_end})): {e:?}")
            })
    }

    pub async fn find_beacon_block(
        &self,
        block_number: u64,
        block_start: &BeaconBlock,
    ) -> AnyResult<BeaconBlock> {
        match block_number.cmp(&block_start.body.execution_payload.block_number) {
            Ordering::Less => {
                return Err(anyhow!(
                    "Requested block number is behind the start beacon block"
                ))
            }
            Ordering::Equal => return Ok(block_start.clone()),
            Ordering::Greater => (),
        }

        let block_finalized = self.get_block_finalized().await?;

        let slot_start = block_start.slot + 1;
        for slot in slot_start..=block_finalized.slot {
            match self.get_block(slot).await {
                Ok(block) if block.body.execution_payload.block_number == block_number => {
                    return Ok(block)
                }
                Ok(_) => (),
                Err(e) if e.downcast_ref::<ErrorNotFound>().is_some() => {}
                Err(e) => return Err(e),
            }
        }

        Err(anyhow!("Block was not found"))
    }

    #[cfg(test)]
    pub async fn get_bootstrap(
        &self,
        checkpoint: &str,
    ) -> AnyResult<ethereum_common::utils::Bootstrap> {
        let checkpoint_no_prefix = match checkpoint.starts_with("0x") {
            true => &checkpoint[2..],
            false => checkpoint,
        };

        let url = format!(
            "{}/eth/v1/beacon/light_client/bootstrap/0x{checkpoint_no_prefix}",
            self.rpc_url
        );

        get::<ethereum_common::utils::BootstrapResponse>(self.client.get(&url))
            .await
            .map(|response| response.data)
    }
}

async fn get<R: DeserializeOwned>(request_builder: RequestBuilder) -> AnyResult<R> {
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
