use clap::Parser;
use tokio::{
    sync::mpsc::{self, Sender},
    time::{self, Duration},
};
use reqwest::Client;
use utils::FinalityUpdate;
use pretty_env_logger::env_logger::fmt::TimestampPrecision;
use gclient::{EventListener, EventProcessor, GearApi};
use checkpoint_light_client_io::{ethereum_common::{base_types::BytesFixed, utils as eth_utils, EPOCHS_PER_SYNC_COMMITTEE, SLOTS_PER_EPOCH}, meta::State, sync_update, tree_hash::Hash256, G2TypeInfo, Handle, HandleResult, Slot, G2};
use parity_scale_codec::Decode;

#[cfg(test)]
mod tests;

mod utils;

const SIZE_CHANNEL: usize = 100_000;
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
        .filter_level(log::LevelFilter::Info)
        .format_target(false)
        .format_timestamp(Some(TimestampPrecision::Micros))
        .init();

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

    match try_to_apply_sync_update(&client, &mut listener, program_id, finality_update).await {
        Status::Ok | Status::NotActual => (),
        Status::Error => return,
        Status::ReplayBackRequired { replayed_slot, checkpoint } => replay_back(&client, &mut listener, program_id, replayed_slot, checkpoint, slot_last).await,
    }

    while let Some(finality_update) = receiver.recv().await {
        let slot = finality_update.finalized_header.slot;
        if slot == slot_last {
            continue;
        }

        match try_to_apply_sync_update(&client, &mut listener, program_id, finality_update).await {
            Status::Ok => { slot_last = slot; }
            Status::NotActual => (),
            _ => return,
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
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    replayed_slot: Option<Slot>,
    checkpoint: (Slot, Hash256),
    slot_last: Slot,
) {
    // let (slot_stored, _) = checkpoint;
    // let slot_start = match replayed_slot {
    //     Some(slot_end) => replay_back_slots(slot_stored, slot_end),
    //     None => slot_stored,
    // };

    // let period_start = 1 + eth_utils::calculate_period(slot_start);
    // let period_end = eth_utils::calculate_period(slot_last);

    // for period in period_start..period_end {

    // }

    // replay_back_slots(period_new * EPOCHS_PER_SYNC_COMMITTEE * SLOTS_PER_EPOCH, slot_last);

    todo!()
}
