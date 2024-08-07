use super::*;
use anyhow::{anyhow, Result as AnyResult};
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
const SIZE_BATCH: u64 = 40 * SLOTS_PER_EPOCH;
const COUNT_FAILURE: usize = 3;
const DELAY_SECS_UPDATE_REQUEST: u64 = 30;

pub async fn relay(args: RelayCheckpointsArgs) {
    log::info!("Started");

    let RelayCheckpointsArgs {
        program_id,
        beacon_endpoint,
        vara_domain,
        vara_port,
        vara_suri,
        prometheus_args: PrometheusArgs {
            endpoint: endpoint_prometheus,
        },
    } = args;

    let program_id_no_prefix = match program_id.starts_with("0x") {
        true => &program_id[2..],
        false => &program_id,
    };

    let program_id = hex::decode(program_id_no_prefix)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .expect("Expecting correct ProgramId");

    let mut signal_interrupt = unix::signal(SignalKind::interrupt()).expect("Set SIGINT handler");

    let (sender, mut receiver) = mpsc::channel(SIZE_CHANNEL);
    let client_http = Client::new();

    sync_update::spawn_receiver(client_http.clone(), beacon_endpoint.clone(), sender);

    let client = GearApi::init_with(WSAddress::new(vara_domain, vara_port), vara_suri)
        .await
        .expect("GearApi client should be created");

    let gas_limit_block = client
        .block_gas_limit()
        .expect("Block gas limit should be determined");

    // use 95% of block gas limit for all extrinsics
    let gas_limit = gas_limit_block / 100 * 95;
    log::info!("Gas limit for extrinsics: {gas_limit}");

    let mut listener = client
        .subscribe()
        .await
        .expect("Events listener should be created");

    let sync_update = receiver
        .recv()
        .await
        .expect("Updates receiver should be open before the loop");

    let mut slot_last = sync_update.finalized_header.slot;

    match sync_update::try_to_apply(
        &client,
        &mut listener,
        program_id,
        sync_update.clone(),
        gas_limit,
    )
    .await
    {
        Err(e) => {
            log::error!("{e:?}");
            return;
        }
        Ok(Err(sync_update::Error::ReplayBackRequired {
            replay_back,
            checkpoint,
        })) => {
            if let Err(e) = replay_back::execute(
                &client_http,
                &beacon_endpoint,
                &client,
                &mut listener,
                program_id,
                gas_limit,
                replay_back.map(|r| r.last_header),
                checkpoint,
                sync_update,
            )
            .await
            {
                log::error!("{e:?}");
            }

            log::info!("Exiting");

            return;
        }
        Ok(Ok(_) | Err(sync_update::Error::NotActual)) => (),
        _ => {
            slot_last = 0;
        }
    }

    let update_metrics = metrics::Updates::new();
    MetricsBuilder::new()
        .register_service(&update_metrics)
        .build()
        .run(endpoint_prometheus)
        .await;

    log::info!("Metrics service spawned");

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
        let slot = sync_update.finalized_header.slot;

        update_metrics
            .fetched_sync_update_slot
            .set(i64::from_le_bytes(slot.to_le_bytes()));

        if slot == slot_last {
            update_metrics.total_fetched_finality_updates.inc();

            continue;
        }

        let committee_update = sync_update.sync_committee_next_pub_keys.is_some();
        match sync_update::try_to_apply(&client, &mut listener, program_id, sync_update, gas_limit)
            .await
        {
            Ok(Ok(_)) => {
                slot_last = slot;

                if committee_update {
                    update_metrics.total_fetched_committee_updates.inc();
                    update_metrics.processed_committee_updates.inc();
                } else {
                    update_metrics.total_fetched_finality_updates.inc();
                    update_metrics.processed_finality_updates.inc();
                }
            }
            Ok(Err(sync_update::Error::ReplayBackRequired { .. })) => {
                log::info!("Replay back within the main loop. Exiting");
                return;
            }
            Ok(Err(e)) => log::info!("The program failed with: {e:?}. Skipping"),
            Err(e) => {
                log::error!("{e:?}");
                return;
            }
        }
    }
}
