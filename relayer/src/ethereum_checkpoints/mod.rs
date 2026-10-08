use anyhow::{anyhow, Result as AnyResult};
use futures::{
    future::{self, Either},
    pin_mut,
};
use gclient::GearApi;
use parity_scale_codec::Encode;
use primitive_types::H256;
use tokio::{
    signal::unix::{self, SignalKind},
    sync::mpsc::{self, Sender},
    time::{self, Duration},
};

use checkpoint_light_client_io::{
    Error, Hash256, ReplayBack, Slot, Update as SyncCommitteeUpdate, G2,
};
use ethereum_beacon_client::{slots_batch::Iter as SlotsBatchIter, BeaconClient};
use ethereum_common::{utils as eth_utils, MAX_REQUEST_LIGHT_CLIENT_UPDATES};
use sails_rs::{calls::*, gclient::calls::*};
use utils_prometheus::MeteredService;

mod metrics;
mod replay_back;
mod sync_update;

const SIZE_CHANNEL: usize = 100_000;
const COUNT_FAILURE: usize = 3;
const DELAY_SECS_UPDATE_REQUEST: u64 = 30;

struct SyncUpdate {
    sync_update: SyncCommitteeUpdate,
    sync_aggregate_encoded: Vec<u8>,
}

pub struct Relayer {
    program_id: H256,

    beacon_client: BeaconClient,
    gear_api: GearApi,

    metrics: metrics::Updates,

    size_batch: u64,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl Relayer {
    pub fn new(
        program_id: H256,
        beacon_client: BeaconClient,
        gear_api: GearApi,
        size_batch: u64,
    ) -> Self {
        Self {
            program_id,
            beacon_client,
            gear_api,
            metrics: metrics::Updates::new(),
            size_batch,
        }
    }

    pub async fn run(self) -> AnyResult<()> {
        log::info!("Started");

        let mut signal_interrupt =
            unix::signal(SignalKind::interrupt()).expect("Set SIGINT handler");

        let (sender, mut receiver) = mpsc::channel(SIZE_CHANNEL);

        sync_update::spawn_receiver(self.beacon_client.clone(), sender);

        let gas_limit_block = self
            .gear_api
            .block_gas_limit()
            .expect("Block gas limit should be determined");

        // use 95% of block gas limit for all extrinsics
        let gas_limit = gas_limit_block / 100 * 95;
        log::info!("Gas limit for extrinsics: {gas_limit}");

        let mut slot_last = 0;
        let remoting = GClientRemoting::new(self.gear_api.clone());

        update_total_balance(&self.gear_api, &self.metrics).await;

        loop {
            let future_interrupt = signal_interrupt.recv();
            pin_mut!(future_interrupt);

            let future_update = receiver.recv();
            pin_mut!(future_update);

            let SyncUpdate {
                sync_update,
                sync_aggregate_encoded,
            } = match future::select(future_interrupt, future_update).await {
                Either::Left((_interrupted, _)) => {
                    log::info!("Caught SIGINT. Exiting");
                    return Ok(());
                }

                Either::Right((Some(sync_update), _)) => sync_update,
                Either::Right((None, _)) => {
                    return Err(anyhow!("Checkpoint update receiver closed unexpectedly"));
                }
            };
            let slot = sync_update.finalized_header.slot;

            self.metrics
                .fetched_sync_update_slot
                .set(i64::from_le_bytes(slot.to_le_bytes()));

            let committee_update = sync_update.sync_committee_next_pub_keys.is_some();
            if !committee_update {
                self.metrics.total_fetched_finality_updates.inc();
            }

            if slot <= slot_last {
                continue;
            }

            let result = sync_update::try_to_apply(
                &remoting,
                self.program_id.0,
                sync_update.clone(),
                sync_aggregate_encoded.clone(),
                gas_limit,
            )
            .await?;
            let result = match result {
                Err(Error::ReplayBackRequired {
                    replay_back,
                    checkpoint,
                }) => {
                    replay_back::execute(replay_back::Args {
                        beacon_client: &self.beacon_client,
                        remoting: &remoting,
                        program_id: self.program_id.0,
                        gas_limit,
                        replay_back,
                        checkpoint,
                        sync_update,
                        size_batch: self.size_batch,
                        sync_aggregate_encoded,
                    })
                    .await?;
                    Ok(())
                }
                result => result,
            };
            match result {
                Ok(()) => {
                    slot_last = slot;

                    if committee_update {
                        self.metrics.processed_committee_updates.inc();
                    } else {
                        self.metrics.processed_finality_updates.inc();
                    }
                }
                Err(Error::StateChanged) => {
                    log::info!(
                        "Checkpoint state changed during verification; retrying fresh updates"
                    );
                }
                Err(e) => {
                    log::error!("The program failed with: {e:?}. Skipping");
                    if let Error::NotActual = e {
                        slot_last = slot;
                    }
                }
            }

            update_total_balance(&self.gear_api, &self.metrics).await;
        }
    }
}

async fn update_total_balance(client: &GearApi, update_metrics: &metrics::Updates) {
    match client.total_balance(client.account_id()).await {
        Ok(total_balance) => {
            let total_balance = total_balance / gear_common::UNITS;
            let total_balance: i64 = total_balance.try_into().unwrap_or(i64::MAX);

            update_metrics.account_total_balance.set(total_balance);
        }

        Err(e) => log::error!("Unable to get total balance: {e:?}"),
    }
}
