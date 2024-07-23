use clap::Parser;
use tokio::{
    sync::mpsc::{self, Sender},
    time::{self, Duration},
};
use reqwest::Client;
use utils::{slots_batch, FinalityUpdate, Update, MAX_REQUEST_LIGHT_CLIENT_UPDATES};
use pretty_env_logger::env_logger::fmt::TimestampPrecision;
use gclient::{EventListener, EventProcessor, GearApi};
use checkpoint_light_client_io::{ethereum_common::{base_types::BytesFixed, utils as eth_utils, EPOCHS_PER_SYNC_COMMITTEE, SLOTS_PER_EPOCH}, meta::State, replay_back, sync_update, tree_hash::Hash256, G2TypeInfo, Handle, HandleResult, Slot, SyncCommitteeUpdate, G2};
use parity_scale_codec::Decode;
use utils::slots_batch::Iter as SlotsBatchIter;

#[cfg(test)]
mod tests;

mod utils;

const SIZE_CHANNEL: usize = 100_000;
const SIZE_BATCH: u64 = 26 * SLOTS_PER_EPOCH;
const COUNT_FAILURE: usize = 3;
const DELAY_SECS_FINALITY_REQUEST: u64 = 30;

#[derive(Debug, Parser)]
struct Args {
    /// Specify ProgramId of the Checkpoint-light-client program
    #[arg(long)]
    program_id: String,

    /// Specify an endpoint providing Beacon API
    #[arg(long)]
    beacon_endpoint: String,

    /// Address of the VARA RPC endpoint
    #[arg(
        long,
        env = "VARA_RPC"
    )]
    vara_endpoint: String,
}

enum Status {
    Ok,
    NotActual,
    Error,
    ReplayBackRequired {
        replayed_slot: Option<Slot>,
        checkpoint: (Slot, Hash256),
    },
}

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .format_timestamp(Some(TimestampPrecision::Micros))
        .parse_default_env()
        .init();

    log::info!("Started");

    let Args {
        program_id,
        beacon_endpoint,
        vara_endpoint,
    } = Args::parse();

    let program_id_no_prefix = match program_id.starts_with("0x") {
        true => &program_id[2..],
        false => &program_id,
    };

    let Some(program_id) = hex::decode(program_id_no_prefix)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok()) else {
            log::error!("Incorrect ProgramId");
            return;
    };

    let (sender, mut receiver) = mpsc::channel(SIZE_CHANNEL);
    let client_http = Client::new();

    spawn_finality_update_receiver(client_http.clone(), beacon_endpoint.clone(), sender, Duration::from_secs(DELAY_SECS_FINALITY_REQUEST));

    let client = match GearApi::dev().await {
        Ok(client) => client,
        Err(e) => {
            log::error!("Unable to create GearApi client: {e:?}");

            return;
        }
    };

    let mut listener = match client.subscribe().await {
        Ok(listener) => listener,
        Err(e) => {
            log::error!("Unable to create events listener: {e:?}");

            return;
        }
    };

    let finality_update = match receiver.recv().await {
        Some(finality_update) => finality_update,
        None => {
            log::info!("Receiver of FinalityUpdates has been closed. Exiting.");

            return;
        }
    };

    let mut slot_last = finality_update.finalized_header.slot;

    match try_to_apply_sync_update(&client, &mut listener, program_id, finality_update.clone()).await {
        Status::Ok | Status::NotActual => (),
        Status::Error => return,
        Status::ReplayBackRequired { replayed_slot, checkpoint } => replay_back(&client_http, &beacon_endpoint, &client, &mut listener, program_id, replayed_slot, checkpoint, finality_update).await,
    }

    while let Some(finality_update) = receiver.recv().await {
        let slot = finality_update.finalized_header.slot;
        if slot == slot_last {
            continue;
        }

        match try_to_apply_sync_update(&client, &mut listener, program_id, finality_update).await {
            Status::Ok => { slot_last = slot; }
            Status::NotActual => (),
            _ => continue,
        }
    }
}

fn spawn_finality_update_receiver(
    client_http: Client,
    beacon_endpoint: String,
    sender: Sender<FinalityUpdate>,
    delay: Duration,
) {
    tokio::spawn(async move {
        log::info!("Update receiver spawned");

        let mut failures = 0;

        loop {
            match utils::get_finality_update(&client_http, &beacon_endpoint).await {
                Ok(value) => {
                    if sender.send(value).await.is_err() {
                        return;
                    }
                }

                Err(e) => {
                    log::error!("Unable to fetch FinalityUpdate: {e:?}");

                    failures += 1;
                    if failures >= COUNT_FAILURE {
                        return;
                    }

                    continue;
                }
            };

            time::sleep(delay).await;
        }
    });
 }

 async fn try_to_apply_sync_update(
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    finality_update: FinalityUpdate,
) -> Status {
    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &finality_update.sync_aggregate.sync_committee_signature.0 .0[..],
    );

    let Ok(signature) = signature else {
        log::error!("Failed to deserialize point on G2");
        return Status::Error;
    };

    let payload = Handle::SyncUpdate(utils::sync_update_from_finality(signature, finality_update));

    let (message_id, _) = match client
        .send_message(program_id.into(), payload, 700_000_000_000, 0)
        .await
        {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to send message: {e:?}");

                return Status::Error;
            }
        };

    let (_message_id, payload, _value) = match listener
        .reply_bytes_on(message_id)
        .await
        {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to get reply: {e:?}");

                return Status::Error;
            }
        };
    let result_decoded = HandleResult::decode(&mut &payload.unwrap()[..]).unwrap();
    log::debug!("Handle result = {result_decoded:?}");
    match result_decoded {
        HandleResult::SyncUpdate(Ok(())) => Status::Ok,
        HandleResult::SyncUpdate(Err(sync_update::Error::NotActual)) => Status::NotActual,
        HandleResult::SyncUpdate(Err(sync_update::Error::ReplayBackRequired {
            replayed_slot,
            checkpoint
        })) => Status::ReplayBackRequired { replayed_slot, checkpoint },
        _ => Status::Error,
    }
 }

 async fn replay_back(
    client_http: &Client,
    beacon_endpoint: &str,
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    replayed_slot: Option<Slot>,
    checkpoint: (Slot, Hash256),
    finality_update: FinalityUpdate,
) {
    log::info!("Replaying back started");

    let (mut slot_start, _) = checkpoint;
    if let Some(slot_end) = replayed_slot {
        let Some(slots_batch_iter) = SlotsBatchIter::new(slot_start, slot_end, SIZE_BATCH) else {
            log::error!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_end = {slot_end}.");

            return;
        };

        replay_back_slots(client_http, beacon_endpoint, client, listener, program_id, slots_batch_iter)
            .await;

        return;
    }

    let period_start = 1 + eth_utils::calculate_period(slot_start);
    let updates = match utils::get_updates(&client_http, beacon_endpoint, period_start, MAX_REQUEST_LIGHT_CLIENT_UPDATES).await
    {
        Ok(updates) => updates,
        Err(e) => {
            log::error!("Failed to get updates for period {period_start}: {e:?}");

            return;
        }
    };
    
    for update in updates {
        let slot_end = update.data.finalized_header.slot;
        let Some(mut slots_batch_iter) = SlotsBatchIter::new(slot_start, slot_end, SIZE_BATCH) else {
            log::error!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_end (update) = {slot_end}.");

            return;
        };

        slot_start = slot_end;

        let sync_update = utils::sync_update_from_update(update.data);
        if replay_back_slots_start(client_http, beacon_endpoint, client, listener, program_id, slots_batch_iter.next(), sync_update).await.is_none() {
            return;
        }

        if replay_back_slots(client_http, beacon_endpoint, client, listener, program_id, slots_batch_iter).await.is_none() {
            return;
        }
    }

    let slot_last = finality_update.finalized_header.slot;
    let Some(mut slots_batch_iter) = SlotsBatchIter::new(slot_start, slot_last, SIZE_BATCH) else {
        log::error!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_last = {slot_last}.");

        return;
    };

    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &finality_update.sync_aggregate.sync_committee_signature.0 .0[..],
    );

    let Ok(signature) = signature else {
        log::error!("replay_back; Failed to deserialize point on G2");
        return;
    };

    let sync_update = utils::sync_update_from_finality(signature, finality_update);
    if replay_back_slots_start(client_http, beacon_endpoint, client, listener, program_id, slots_batch_iter.next(), sync_update).await.is_none() {
        return;
    }

    replay_back_slots(client_http, beacon_endpoint, client, listener, program_id, slots_batch_iter).await;

    log::info!("Replaying back finished");
}

async fn replay_back_slots(
    client_http: &Client,
    beacon_endpoint: &str,
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    slots_batch_iter: SlotsBatchIter,
) -> Option<()> {
    for (slot_start, slot_end) in slots_batch_iter {
        replay_back_slots_inner(client_http, beacon_endpoint, client, listener, program_id, slot_start, slot_end)
            .await?;
    }

    Some(())
}

async fn replay_back_slots_inner(
    client_http: &Client,
    beacon_endpoint: &str,
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    slot_start: Slot,
    slot_end: Slot,
) -> Option<()> {
    let batch_size = (slot_end - slot_start) as usize;
    let mut requests_headers = Vec::with_capacity(batch_size);
    for i in slot_start..slot_end {
        requests_headers.push(utils::get_block_header(&client_http, &beacon_endpoint, i));
    }

    let headers = futures::future::join_all(requests_headers)
        .await
        .into_iter()
        .filter_map(|maybe_header| maybe_header.ok())
        .collect::<Vec<_>>();

    let payload = Handle::ReplayBack(headers);

    let (message_id, _) = client
        .send_message(program_id.into(), payload, 700_000_000_000, 0)
        .await
        .map_err(|e| log::error!("Failed to send ReplayBack message: {e:?}"))
        .ok()?;

    let (_message_id, payload, _value) = listener.reply_bytes_on(message_id)
        .await
        .map_err(|e| log::error!("Failed to get reply to ReplayBack message: {e:?}"))
        .ok()?;
    let payload = payload.map_err(|e| log::error!("Failed to get replay payload to ReplayBack: {e:?}")).ok()?;
    let result_decoded = HandleResult::decode(&mut &payload[..])
        .map_err(|e| log::error!("Failed to decode HandleResult of ReplayBack: {e:?}")).ok()?;
    
    log::debug!("replay_back_slots_inner; result_decoded = {result_decoded:?}");

    matches!(
        result_decoded,
        HandleResult::ReplayBack(Some(replay_back::Status::InProcess | replay_back::Status::Finished))
    ).then_some(())
}

async fn replay_back_slots_start(
    client_http: &Client,
    beacon_endpoint: &str,
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    slots: Option<(Slot, Slot)>,
    sync_update: SyncCommitteeUpdate,
) -> Option<()> {
    let Some((slot_start, slot_end)) = slots else {
        return Some(());
    };

    let mut requests_headers = Vec::with_capacity(SIZE_BATCH as usize);
    for i in slot_start..slot_end {
        requests_headers.push(utils::get_block_header(&client_http, beacon_endpoint, i));
    }

    let headers = futures::future::join_all(requests_headers)
        .await
        .into_iter()
        .filter_map(|maybe_header| maybe_header.ok())
        .collect::<Vec<_>>();

    let payload = Handle::ReplayBackStart {
        sync_update,
        headers,
    };

    let (message_id, _) = client
        .send_message(program_id.into(), payload, 700_000_000_000, 0)
        .await
        .map_err(|e| log::error!("Failed to send ReplayBackStart message: {e:?}"))
        .ok()?;

    let (_message_id, payload, _value) = listener.reply_bytes_on(message_id)
        .await
        .map_err(|e| log::error!("Failed to get reply to ReplayBackStart message: {e:?}"))
        .ok()?;
    let payload = payload.map_err(|e| log::error!("Failed to get replay payload to ReplayBackStart: {e:?}")).ok()?;
    let result_decoded = HandleResult::decode(&mut &payload[..])
        .map_err(|e| log::error!("Failed to decode HandleResult of ReplayBackStart: {e:?}")).ok()?;
    
    log::debug!("replay_back_slots_start; result_decoded = {result_decoded:?}");

    matches!(
        result_decoded,
        HandleResult::ReplayBackStart(Ok(replay_back::StatusStart::InProgress| replay_back::StatusStart::Finished))
    ).then_some(())
}
