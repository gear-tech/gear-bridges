use crate::{connect_to_node, DEFAULT_BALANCE};
use checkpoint_light_client::WASM_BINARY;
use checkpoint_light_client_client::{
    checkpoint_light_client_factory::io as factory_io,
    service_replay_back::events::ServiceReplayBackEvents, service_sync_update,
    service_sync_update::events::ServiceSyncUpdateEvents, traits::*,
};
use checkpoint_light_client_io::{Error, Init, ReplayBackError, ReplayBackStatus, G2};
use ethereum_beacon_client::utils;
use ethereum_common::{
    base_types::BytesFixed,
    beacon::SyncAggregate,
    network::Network,
    tree_hash::TreeHash,
    utils::{
        BeaconBlockHeaderResponse, Bootstrap, BootstrapResponse, FinalityUpdateResponse, Update,
        UpdateData,
    },
    SLOTS_PER_EPOCH,
};
use futures::StreamExt;
use gclient::{EventProcessor, GearApi, Result};
use ruzstd::StreamingDecoder;
use sails_rs::{
    calls::*,
    events::{EventIo, Listener},
    gclient::calls::*,
    prelude::*,
};
use std::io::Read;

const SEPOLIA_FINALITY_UPDATE_5_263_072: &[u8; 4_941] =
    include_bytes!("./chain-data/sepolia-finality-update-5_263_072.json");
const SEPOLIA_UPDATE_640: &[u8; 57_202] = include_bytes!("./chain-data/sepolia-update-640.json");
const SEPOLIA_BOOTSTRAP_640: &[u8; 54_328] =
    include_bytes!("./chain-data/sepolia-bootstrap-640.json");

const HOLESKY_UPDATE_368: &[u8; 30_468] =
    include_bytes!("./chain-data/holesky-update-368.json.zst");
const HOLESKY_BOOTSTRAP_368: &[u8; 29_297] =
    include_bytes!("./chain-data/holesky-bootstrap-368.json.zst");
const HOLESKY_HEADERS: &[u8; 452_109] = include_bytes!("./chain-data/headers.json.zst");
const HOLESKY_FINALITY_UPDATE_3_014_736: &[u8; 4_893] =
    include_bytes!("./chain-data/holesky-finality-update-3_016_736.json");
const HOLESKY_FINALITY_UPDATE_3_014_768: &[u8; 4_932] =
    include_bytes!("./chain-data/holesky-finality-update-3_016_768.json");
const HOLESKY_FINALITY_UPDATE_3_014_799: &[u8; 4_980] =
    include_bytes!("./chain-data/holesky-finality-update-3_016_799.json");

#[track_caller]
fn decode_signature(sync_aggregate: &SyncAggregate) -> G2 {
    <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .unwrap()
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

    let sync_aggregate_encoded = update.sync_aggregate.encode();
    let sync_update =
        utils::sync_update_from_update(decode_signature(&update.sync_aggregate), update);
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
        sync_aggregate_encoded,
    }
}

async fn calculate_upload_gas(api: &GearApi, code_id: CodeId, init: &Init) -> Result<u64> {
    let origin = H256::from_slice(api.account_id().as_ref());
    let payload = {
        let mut payload = factory_io::Init::ROUTE.to_vec();
        init.encode_to(&mut payload);

        payload
    };

    Ok(api
        .calculate_create_gas(Some(origin.0.into()), code_id, payload, 0, true)
        .await?
        .min_limit)
}

async fn calculate_gas<T: ActionIo>(
    api: &GearApi,
    program_id: ActorId,
    params: &T::Params,
) -> Result<u64> {
    let origin = H256::from_slice(api.account_id().as_ref());
    let payload = T::encode_call(params);

    Ok(api
        .calculate_handle_gas(Some(origin.0.into()), program_id, payload, 0, true)
        .await?
        .min_limit)
}

#[tokio::test]
async fn init_holesky() -> Result<()> {
    let (bootstrap, update) = get_bootstrap_and_update();

    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "checkpoint-light-client",
        &[WASM_BINARY],
    )
    .await;
    let api = conn.api;
    let api = api.with(&conn.accounts[0].2).unwrap();
    let code_id = conn.code_ids[0];
    let salt = conn.salt;
    let factory = checkpoint_light_client_client::CheckpointLightClientFactory::new(
        GClientRemoting::new(api.clone()),
    );

    let init = construct_init(Network::Holesky, update, bootstrap);
    let gas_limit = calculate_upload_gas(&api, code_id, &init).await?;
    let program_id = factory
        .init(init)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .unwrap();

    println!(
        "program_id = {:?}, gas_limit = {gas_limit}",
        hex::encode(program_id)
    );

    Ok(())
}

#[tokio::test]
async fn sync_update_requires_replaying_back() -> Result<()> {
    use checkpoint_light_client_client::service_sync_update::io;

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

    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "checkpoint-light-client",
        &[WASM_BINARY],
    )
    .await;
    let api = conn.api.with(&conn.accounts[0].2).unwrap();
    let code_id = conn.code_ids[0];
    let salt = conn.salt;
    let factory = checkpoint_light_client_client::CheckpointLightClientFactory::new(
        GClientRemoting::new(api.clone()),
    );

    let init = construct_init(Network::Sepolia, update, bootstrap);
    let gas_limit = calculate_upload_gas(&api, code_id, &init).await?;
    let program_id = factory
        .init(init)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .unwrap();

    println!(
        "program_id = {:?}, gas_limit = {gas_limit}",
        hex::encode(program_id)
    );

    println!();
    println!();

    println!(
        "slot = {slot:?}, attested slot = {:?}, signature slot = {:?}",
        finality_update.attested_header.slot, finality_update.signature_slot
    );

    let mut service =
        checkpoint_light_client_client::ServiceSyncUpdate::new(GClientRemoting::new(api.clone()));
    let (gas_limit, (update, sync_aggregate_encoded)) = {
        let sync_aggregate_encoded = finality_update.sync_aggregate.encode();
        let params = (
            utils::sync_update_from_finality(
                decode_signature(&finality_update.sync_aggregate),
                finality_update,
            ),
            sync_aggregate_encoded,
        );

        (
            calculate_gas::<io::Process>(&api, program_id, &params).await?,
            params,
        )
    };

    println!("process gas_limit = {gas_limit}");
    let result = service
        .process(update, sync_aggregate_encoded)
        .send_recv(program_id)
        .await
        .unwrap();

    assert!(
        matches!(result, Err(Error::ReplayBackRequired { .. })),
        "result = {result:?}"
    );

    Ok(())
}

#[tokio::test]
async fn replay_back_and_updating() -> Result<()> {
    use checkpoint_light_client_client::{
        service_replay_back::io as replay_back_io, service_sync_update::io as sync_update_io,
    };

    let (bootstrap, update) = get_bootstrap_and_update();

    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "checkpoint-light-client",
        &[WASM_BINARY],
    )
    .await;

    let api = conn.api.with(&conn.accounts[0].2).unwrap();
    let code_id = conn.code_ids[0];
    let salt = conn.salt;

    let factory = checkpoint_light_client_client::CheckpointLightClientFactory::new(
        GClientRemoting::new(api.clone()),
    );

    let init = construct_init(Network::Holesky, update, bootstrap);
    let gas_limit = calculate_upload_gas(&api, code_id, &init).await?;
    let program_id = factory
        .init(init)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, salt)
        .await
        .unwrap();

    println!(
        "program_id = {:?}, gas_limit = {gas_limit}",
        hex::encode(program_id)
    );

    println!();
    println!();

    let finality_update: FinalityUpdateResponse =
        serde_json::from_slice(HOLESKY_FINALITY_UPDATE_3_014_736).unwrap();
    let finality_update = finality_update.data;

    let mut decoder = StreamingDecoder::new(&HOLESKY_HEADERS[..]).unwrap();
    let mut headers = Vec::new();
    decoder.read_to_end(&mut headers).unwrap();

    let headers_all: Vec<BeaconBlockHeaderResponse> = serde_json::from_slice(&headers[..]).unwrap();
    let size_batch = 30 * SLOTS_PER_EPOCH as usize;
    let mut service =
        checkpoint_light_client_client::ServiceReplayBack::new(GClientRemoting::new(api.clone()));
    let sync_aggregate_encoded = finality_update.sync_aggregate.encode();
    let signature = decode_signature(&finality_update.sync_aggregate);

    // attempt to process next headers of inactive backreplaying should fail
    let result = service
        .process(
            headers_all
                .iter()
                .rev()
                .skip(size_batch)
                .map(|r| r.data.header.message.clone())
                .collect(),
        )
        .send_recv(program_id)
        .await
        .unwrap();

    assert!(
        matches!(result, Err(ReplayBackError::NotStarted)),
        "result = {result:?}"
    );

    // start to replay back
    let (gas_limit, (sync_update, sync_aggregate_encoded, headers)) = {
        let sync_update = utils::sync_update_from_finality(signature, finality_update.clone());
        let params = (
            sync_update,
            sync_aggregate_encoded,
            headers_all
                .iter()
                .rev()
                .take(size_batch)
                .map(|r| r.data.header.message.clone())
                .collect(),
        );

        (
            calculate_gas::<replay_back_io::Start>(&api, program_id, &params).await?,
            params,
        )
    };

    println!("replay_back_io::Start gas_limit = {gas_limit}");
    let result = service
        .start(sync_update, sync_aggregate_encoded.clone(), headers)
        .send_recv(program_id)
        .await
        .unwrap();

    assert!(
        matches!(result, Ok(ReplayBackStatus::InProcess)),
        "result = {result:?}"
    );

    // second attempt to start backreplay should fail
    let sync_update = utils::sync_update_from_finality(signature, finality_update);
    let result = service
        .start(
            sync_update.clone(),
            sync_aggregate_encoded,
            headers_all
                .iter()
                .rev()
                .take(size_batch)
                .map(|r| r.data.header.message.clone())
                .collect(),
        )
        .send_recv(program_id)
        .await
        .unwrap();

    assert!(
        matches!(result, Err(ReplayBackError::AlreadyStarted)),
        "result = {result:?}"
    );
    let mut listener = api.subscribe().await.unwrap();
    // replaying the blocks back
    let headers = headers_all
        .iter()
        .rev()
        .skip(size_batch)
        .map(|r| r.data.header.message.clone())
        .collect();
    let gas_limit = calculate_gas::<replay_back_io::Process>(&api, program_id, &headers).await?;
    println!("replay_back_io::Process gas_limit = {gas_limit}");
    let result = service
        .process(headers.clone())
        .send_recv(program_id)
        .await
        .unwrap();

    listener
        .proc_many(
            |event| match event {
                gclient::Event::Gear(gclient::GearEvent::UserMessageSent { message, .. }) => {
                    if message.source().into_bytes() == program_id.into_bytes()
                        && message.destination().into_bytes() == [0; 32]
                    {
                        let ServiceReplayBackEvents::NewCheckpoint {
                            slot,
                            tree_hash_root,
                        } = ServiceReplayBackEvents::decode_event(&message.payload_bytes())
                            .unwrap();

                        assert!(headers.iter().any(|header| {
                            header.slot == slot && header.tree_hash_root() == tree_hash_root
                        }));

                        Some(())
                    } else {
                        None
                    }
                }
                _ => None,
            },
            |res| (res, true),
        )
        .await
        .unwrap();

    assert!(
        matches!(result, Ok(ReplayBackStatus::Finished)),
        "result = {result:?}"
    );

    // updating
    let mut service =
        checkpoint_light_client_client::ServiceSyncUpdate::new(GClientRemoting::new(api.clone()));
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

        let (gas_limit, (update, sync_aggregate_encoded)) = {
            let sync_aggregate_encoded = update.sync_aggregate.encode();
            let params = (
                utils::sync_update_from_finality(decode_signature(&update.sync_aggregate), update),
                sync_aggregate_encoded,
            );

            (
                calculate_gas::<sync_update_io::Process>(&api, program_id, &params).await?,
                params,
            )
        };
        let remoting = GClientRemoting::new(api.clone());
        let mut listener = service_sync_update::events::listener(remoting);
        let mut stream = listener.listen().await.expect("failed to listen to events");
        println!("process gas_limit = {gas_limit}");
        let result = service
            .process(update.clone(), sync_aggregate_encoded)
            .send_recv(program_id)
            .await
            .unwrap();

        if let Ok(()) = result {
            println!("waiting for events");

            loop {
                let (
                    actor_id,
                    ServiceSyncUpdateEvents::NewCheckpoint {
                        slot,
                        tree_hash_root,
                    },
                ) = stream.next().await.expect("failed to get next event");
                assert_eq!(actor_id, program_id);
                if slot != update.finalized_header.slot {
                    println!(
                        "slot mismatch: expected {}, got {}",
                        update.finalized_header.slot, slot
                    );
                    continue;
                }

                println!(
                    "NewCheckpoint: slot = {}, tree_hash_root = {}",
                    slot,
                    hex::encode(tree_hash_root)
                );
                assert_eq!(tree_hash_root, update.finalized_header.tree_hash_root());
                break;
            }
        } else {
            println!("update failed...");
        }

        assert!(
            matches!(result, Ok(_) | Err(Error::LowVoteCount)),
            "result = {result:?}"
        );

        println!();
        println!();
    }
    Ok(())
}
