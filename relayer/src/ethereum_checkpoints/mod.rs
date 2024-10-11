use super::*;
use anyhow::{anyhow, Result as AnyResult};
use checkpoint_light_client_io::{
    ethereum_common::{utils as eth_utils, MAX_REQUEST_LIGHT_CLIENT_UPDATES, SLOTS_PER_EPOCH},
    meta::ReplayBack,
    tree_hash::Hash256,
    Handle, HandleResult, Slot, SyncCommitteeUpdate, G2,
};
use ethereum_beacon_client::{slots_batch::Iter as SlotsBatchIter, BeaconClient};
use futures::{
    future::{self, Either},
    pin_mut,
};
use gclient::{EventProcessor, GearApi, WSAddress};
use parity_scale_codec::Decode;
use tokio::{
    signal::unix::{self, SignalKind},
    sync::mpsc::{self, Sender},
    time::{self, Duration},
};

#[cfg(test)]
mod tests;

mod metrics;
mod replay_back;
mod sync_update;

const SIZE_CHANNEL: usize = 100_000;
const SIZE_BATCH: u64 = 30 * SLOTS_PER_EPOCH;
const COUNT_FAILURE: usize = 3;
const DELAY_SECS_UPDATE_REQUEST: u64 = 30;
// The constant is intentionally duplicated since vara-runtime is too heavy dependency.
const UNITS: u128 = 1_000_000_000_000;

pub async fn relay(args: RelayCheckpointsArgs) {
    log::info!("Started");

    let RelayCheckpointsArgs {
        program_id,
        beacon_endpoint,
        beacon_timeout: _, // TODO: Use this.
        vara_domain,
        vara_port,
        vara_suri,
        prometheus_args: PrometheusArgs {
            endpoint: endpoint_prometheus,
        },
    } = args;

    let program_id = ethereum_beacon_client::try_from_hex_encoded(&program_id)
        .expect("Expecting correct ProgramId");

    let beacon_client = BeaconClient::new(beacon_endpoint.clone());

    let mut signal_interrupt = unix::signal(SignalKind::interrupt()).expect("Set SIGINT handler");

    let (sender, mut receiver) = mpsc::channel(SIZE_CHANNEL);

    sync_update::spawn_receiver(beacon_client.clone(), sender);

    let client = GearApi::init_with(WSAddress::new(vara_domain, vara_port), vara_suri)
        .await
        .expect("GearApi client should be created");

    let gas_limit_block = client
        .block_gas_limit()
        .expect("Block gas limit should be determined");

    // use 95% of block gas limit for all extrinsics
    let gas_limit = gas_limit_block / 100 * 95;
    log::info!("Gas limit for extrinsics: {gas_limit}");

    let sync_update = receiver
        .recv()
        .await
        .expect("Updates receiver should be open before the loop");

    let mut slot_last = sync_update.finalized_header.slot;

    match sync_update::try_to_apply(&client, program_id, sync_update.clone(), gas_limit).await {
        Err(e) => {
            log::error!("{e:?}");
            return;
        }
        Ok(Err(sync_update::Error::ReplayBackRequired {
            replay_back,
            checkpoint,
        })) => {
            if let Err(e) = replay_back::execute(
                &beacon_client,
                &client,
                program_id,
                gas_limit,
                replay_back,
                checkpoint,
                sync_update,
            )
            .await
            {
                log::error!("{e:?}. Exiting");
                return;
            }
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

    update_total_balance(&client, &update_metrics).await;

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

        let committee_update = sync_update.sync_committee_next_pub_keys.is_some();
        if !committee_update {
            update_metrics.total_fetched_finality_updates.inc();
        }

        if slot == slot_last {
            continue;
        }

        match sync_update::try_to_apply(&client, program_id, sync_update, gas_limit).await {
            Ok(Ok(_)) => {
                slot_last = slot;

                if committee_update {
                    update_metrics.processed_committee_updates.inc();
                } else {
                    update_metrics.processed_finality_updates.inc();
                }
            }
            Ok(Err(sync_update::Error::ReplayBackRequired { .. })) => {
                log::error!("Replay back within the main loop. Exiting");
                return;
            }
            Ok(Err(e)) => {
                log::error!("The program failed with: {e:?}. Skipping");
                if let sync_update::Error::NotActual = e {
                    slot_last = slot;
                }
            }
            Err(e) => {
                log::error!("{e:?}");
                return;
            }
        }

        update_total_balance(&client, &update_metrics).await;
    }
}

async fn update_total_balance(client: &GearApi, update_metrics: &metrics::Updates) {
    match client.total_balance(client.account_id()).await {
        Ok(total_balance) => {
            let total_balance = total_balance / UNITS;
            let total_balance: i64 = total_balance.try_into().unwrap_or(i64::MAX);

            update_metrics.account_total_balance.set(total_balance);
        }
        Err(e) => log::error!("Unable to get total balance: {e:?}"),
    }
}
