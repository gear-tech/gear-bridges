use super::utils::{self, slots_batch, FinalityUpdateResponse};
use checkpoint_light_client::WASM_BINARY;
use checkpoint_light_client_io::{
    ethereum_common::{
        base_types::BytesFixed, network::Network, utils as eth_utils, SLOTS_PER_EPOCH,
    },
    replay_back, sync_update,
    tree_hash::TreeHash,
    Handle, HandleResult, Init, G2,
};
use gclient::{EventListener, EventProcessor, GearApi, Result};
use parity_scale_codec::{Decode, Encode};
use reqwest::Client;
use std::env;
use tokio::time::{self, Duration};

const RPC_URL: &str = "http://127.0.0.1:5052";

const FINALITY_UPDATE_5_254_112: &[u8; 4_940] =
    include_bytes!("./sepolia-finality-update-5_254_112.json");
const FINALITY_UPDATE_5_263_072: &[u8; 4_941] =
    include_bytes!("./sepolia-finality-update-5_263_072.json");

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
    let client_http = Client::new();

    let rpc_url = env::var("RPC_URL").unwrap_or(RPC_URL.into());

    // use the latest finality header as a checkpoint for bootstrapping
    let finality_update = utils::get_finality_update(&client_http, &rpc_url).await?;
    let current_period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = utils::get_updates(&client_http, &rpc_url, current_period, 1).await?;

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

    let bootstrap = utils::get_bootstrap(&client_http, &rpc_url, &checkpoint_hex).await?;

    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap();
    let sync_update = utils::sync_update_from_update(signature, update);

    println!("bootstrap slot = {}", bootstrap.header.slot);

    let pub_keys = utils::map_public_keys(&bootstrap.current_sync_committee.pubkeys);
    let network = match env::var("NETWORK") {
        Ok(network) if network == "Holesky" => Network::Holesky,
        Ok(network) if network == "Mainnet" => Network::Mainnet,
        _ => Network::Sepolia,
    };
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

    // let client = GearApi::dev_from_path("../target/release/gear").await?;
    let client = GearApi::dev().await?;
    let mut listener = client.subscribe().await?;

    let program_id = upload_program(&client, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    if env::var("UPDATING").is_err() {
        return Ok(());
    }

    for _ in 0..30 {
        let update = utils::get_finality_update(&client_http, &rpc_url).await?;

        let slot: u64 = update.finalized_header.slot;
        let current_period = eth_utils::calculate_period(slot);
        let mut updates = utils::get_updates(&client_http, &rpc_url, current_period, 1).await?;
        match updates.pop() {
            Some(update) if updates.is_empty() && update.data.finalized_header.slot >= slot => {
                println!("update sync committee");
                let signature =
                    <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
                        &update.data.sync_aggregate.sync_committee_signature.0 .0[..],
                    )
                    .unwrap();
                let payload =
                    Handle::SyncUpdate(utils::sync_update_from_update(signature, update.data));
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

                let payload =
                    Handle::SyncUpdate(utils::sync_update_from_finality(signature, update));

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
    let client_http = Client::new();

    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(FINALITY_UPDATE_5_254_112).unwrap();
    let finality_update = finality_update.data;
    println!(
        "finality_update slot = {}",
        finality_update.finalized_header.slot
    );

    // This SyncCommittee operated for about 13K slots, so we make adjustments
    let current_period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = utils::get_updates(&client_http, RPC_URL, current_period - 1, 1).await?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };
    let checkpoint = update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    let bootstrap = utils::get_bootstrap(&client_http, RPC_URL, &checkpoint_hex).await?;
    println!("bootstrap slot = {}", bootstrap.header.slot);

    println!("update slot = {}", update.finalized_header.slot);
    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap();
    let sync_update = utils::sync_update_from_update(signature, update);
    let slot_start = sync_update.finalized_header.slot;
    let slot_end = finality_update.finalized_header.slot;
    println!(
        "Replaying back from {slot_start} to {slot_end} ({} headers)",
        slot_end - slot_start
    );

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

    // let client = GearApi::dev_from_path("../target/release/gear").await?;
    let client = GearApi::dev().await?;
    let mut listener = client.subscribe().await?;

    let program_id = upload_program(&client, &mut listener, init).await?;

    println!("program_id = {:?}", hex::encode(program_id));

    println!();
    println!();

    let batch_size = 44 * SLOTS_PER_EPOCH;
    let mut slots_batch_iter = slots_batch::Iter::new(slot_start, slot_end, batch_size).unwrap();
    // start to replay back
    if let Some((slot_start, slot_end)) = slots_batch_iter.next() {
        let mut requests_headers = Vec::with_capacity(batch_size as usize);
        for i in slot_start..slot_end {
            requests_headers.push(utils::get_block_header(&client_http, RPC_URL, i));
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
            sync_update: utils::sync_update_from_finality(signature, finality_update),
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
    }

    // replaying the blocks back
    for (slot_start, slot_end) in slots_batch_iter {
        let mut requests_headers = Vec::with_capacity(batch_size as usize);
        for i in slot_start..slot_end {
            requests_headers.push(utils::get_block_header(&client_http, RPC_URL, i));
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
            HandleResult::ReplayBack(Some(
                replay_back::Status::InProcess | replay_back::Status::Finished
            ))
        ));
    }

    Ok(())
}

#[tokio::test]
async fn sync_update_requires_replaying_back() -> Result<()> {
    let client_http = Client::new();

    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(FINALITY_UPDATE_5_263_072).unwrap();
    let finality_update = finality_update.data;
    println!(
        "finality_update slot = {}",
        finality_update.finalized_header.slot
    );

    let slot = finality_update.finalized_header.slot;
    let current_period = eth_utils::calculate_period(slot);
    let mut updates = utils::get_updates(&client_http, RPC_URL, current_period, 1).await?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => unreachable!("Requested single update"),
    };

    let checkpoint = update.finalized_header.tree_hash_root();
    let checkpoint_hex = hex::encode(checkpoint);

    let bootstrap = utils::get_bootstrap(&client_http, RPC_URL, &checkpoint_hex).await?;
    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap();
    let sync_update = utils::sync_update_from_update(signature, update);

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

    let payload = Handle::SyncUpdate(utils::sync_update_from_finality(signature, finality_update));

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
