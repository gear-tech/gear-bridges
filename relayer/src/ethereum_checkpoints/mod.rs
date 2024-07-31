use super::*;
use checkpoint_light_client_io::{
    ethereum_common::{utils as eth_utils, SLOTS_PER_EPOCH},
    tree_hash::Hash256,
    Handle, HandleResult, Slot, SyncCommitteeUpdate, G2,
};
use futures::{
    future::{self, Either},
    pin_mut,
};
use gclient::{EventListener, EventProcessor, GearApi, WSAddress};
use metrics::Message as MetricMessage;
use parity_scale_codec::Decode;
use reqwest::Client;
use tokio::{
    signal::unix::{self, SignalKind},
    sync::mpsc::{self, Sender},
    time::{self, Duration},
};
use utils::{slots_batch::Iter as SlotsBatchIter, MAX_REQUEST_LIGHT_CLIENT_UPDATES};

#[cfg(test)]
mod tests;

mod metrics;
mod replay_back;
mod sync_update;
mod utils;

const SIZE_CHANNEL: usize = 100_000;
const SIZE_BATCH: u64 = 44 * SLOTS_PER_EPOCH;
const COUNT_FAILURE: usize = 3;
const DELAY_SECS_FINALITY_REQUEST: u64 = 30;

enum Status {
    Ok,
    NotActual,
    Error,
    ReplayBackRequired {
        replayed_slot: Option<Slot>,
        checkpoint: (Slot, Hash256),
    },
}

pub async fn relay(
    args: RelayCheckpointsArgs,
) {
    log::info!("Started");

    let Ok(mut signal_interrupt) = unix::signal(SignalKind::interrupt()) else {
        log::error!("Failed to set SIGINT handler");
        return;
    };

    let RelayCheckpointsArgs {
        program_id,
        beacon_endpoint,
        vara_domain,
        vara_port,
        suri,
        prometheus_args: PrometheusArgs {
            endpoint: endpoint_prometheus,
        },
    } = args;

    let sender_metrics = metrics::spawn(endpoint_prometheus);

    let program_id_no_prefix = match program_id.starts_with("0x") {
        true => &program_id[2..],
        false => &program_id,
    };

    let Some(program_id) = hex::decode(program_id_no_prefix)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    else {
        log::error!("Incorrect ProgramId");
        return;
    };

    let (sender, mut receiver) = mpsc::channel(SIZE_CHANNEL);
    let client_http = Client::new();

    sync_update::spawn_receiver(
        client_http.clone(),
        beacon_endpoint.clone(),
        sender,
        Duration::from_secs(DELAY_SECS_FINALITY_REQUEST),
    );

    let client = match GearApi::init_with(WSAddress::new(vara_domain, vara_port), suri).await {
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

    let sync_update = match receiver.recv().await {
        Some(finality_update) => finality_update,
        None => {
            log::info!("Updates receiver has been closed before the loop. Exiting");

            return;
        }
    };

    let mut slot_last = sync_update.finalized_header.slot;

    match sync_update::try_to_apply(&client, &mut listener, program_id, sync_update.clone()).await {
        Status::Ok | Status::NotActual => (),
        Status::Error => return,
        Status::ReplayBackRequired {
            replayed_slot,
            checkpoint,
        } => {
            replay_back::execute(
                &client_http,
                &beacon_endpoint,
                &client,
                &mut listener,
                program_id,
                replayed_slot,
                checkpoint,
                sync_update,
            )
            .await;
            log::info!("Exiting");
            return;
        }
    }

    loop {
        let future_interrupt = signal_interrupt.recv();
        pin_mut!(future_interrupt);

        let future_update = receiver.recv();
        pin_mut!(future_update);

        let sync_update = match future::select(future_interrupt, future_update).await {
            Either::Left((_interrupted, _)) => {
                log::info!("Caught SIGINT. Exiting");
                return;
            }

            Either::Right((Some(sync_update), _)) => sync_update,
            Either::Right((None, _)) => {
                log::info!("Updates receiver has been closed. Exiting");
                return;
            }
        };

        let committee_update = sync_update.sync_committee_next_pub_keys.is_some();
        let slot = sync_update.finalized_header.slot;
        if slot == slot_last {
            let metric_message = MetricMessage {
                slot,
                committee_update,
                processed: false,
            };

            if sender_metrics.send(metric_message).await.is_err() {
                log::error!("Failed to update metrics. Exiting");
                return;
            }

            continue;
        }

        match sync_update::try_to_apply(&client, &mut listener, program_id, sync_update).await {
            Status::Ok => {
                slot_last = slot;

                let metric_message = MetricMessage {
                    slot,
                    committee_update,
                    processed: true,
                };

                if sender_metrics.send(metric_message).await.is_err() {
                    log::error!("Failed to update metrics. Exiting");
                    return;
                }
            }
            Status::NotActual => (),
            Status::ReplayBackRequired { .. } => {
                log::info!("Exiting");
                return;
            }
            _ => continue,
        }
    }
}
