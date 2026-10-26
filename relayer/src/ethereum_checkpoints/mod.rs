use anyhow::{anyhow, ensure, Context, Result as AnyResult};
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
use sails_rs::calls::ActionIo;
use utils_prometheus::MeteredService;

mod metrics;
mod replay_back;
mod sync_update;

const SIZE_CHANNEL: usize = 100_000;
const COUNT_FAILURE: usize = 3;

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
                &self.gear_api,
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
                    match replay_back::execute(replay_back::Args {
                        beacon_client: &self.beacon_client,
                        gear_api: &self.gear_api,
                        program_id: self.program_id.0,
                        gas_limit,
                        replay_back,
                        checkpoint,
                        sync_update,
                        size_batch: self.size_batch,
                        sync_aggregate_encoded,
                    })
                    .await
                    {
                        Ok(()) => Ok(()),
                        Err(error) if error.is::<replay_back::ReplayStateChanged>() => {
                            Err(Error::StateChanged)
                        }
                        Err(error) => return Err(error),
                    }
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

async fn send_checkpoint_message(
    api: &GearApi,
    program_id: [u8; 32],
    payload: Vec<u8>,
    gas_limit: u64,
) -> AnyResult<Vec<u8>> {
    use gsdk::gear::{self, runtime_types::gear_common::event::MessageEntry};

    let signer: gsdk::signer::Signer = api.clone().into();
    let sender: &[u8] = signer.account_id().as_ref();
    let target = gear_core::ids::ActorId::from(program_id);
    let tx = signer
        .process(
            gear::tx()
                .gear()
                .send_message(target, payload, gas_limit, 0, false),
        )
        .await?;
    let original_hash = tx.extrinsic_hash();
    let result = async {
    let included_hash = tx.block_hash();
    let included_number = api.block_number_at(included_hash).await?;
    let mut queued = None;
    let mut success = false;
    for event in tx.fetch_events().await?.iter() {
        let event = event?;
        if let Some(error) = gear_rpc_client::extrinsic_failure(&event)? {
            anyhow::bail!("Original checkpoint {original_hash:#x} failed dispatch: {error}");
        }
        if event.pallet_name() == "System" && event.variant_name() == "ExtrinsicSuccess" {
            ensure!(!success, "Duplicate checkpoint dispatch success; HOLD");
            success = true;
        }
        if let Some(message) = event.as_event::<gear::gear::events::MessageQueued>()? {
            ensure!(
                queued.is_none()
                    && message.source.0.as_slice() == sender
                    && message.destination == target
                    && message.entry == MessageEntry::Handle,
                "Original checkpoint {original_hash:#x} queued identity changed; HOLD"
            );
            queued = Some(message.id);
        }
    }
    ensure!(success, "Original checkpoint {original_hash:#x} lacks successful dispatch; HOLD");
    let id = queued.ok_or_else(|| anyhow!("Original checkpoint {original_hash:#x} lacks queued message; HOLD"))?;
    log::info!("Checkpoint original {original_hash:#x} finalized at {included_hash:#x}, message {id:?}");


        let chain = gear_rpc_client::GearApi::from(signer.api().clone());
        let mut next = included_number;
        let mut previous = (included_number, included_hash);
        loop {
            let finalized_hash = api.last_block_hash().await?;
            let finalized_number = api.block_number_at(finalized_hash).await?;
            ensure!(
                finalized_number >= previous.0
                    && api.get_block_hash(previous.0).await? == previous.1
                    && api.get_block_hash(included_number).await? == included_hash,
                "Checkpoint finalized history changed; HOLD"
            );
            if let Some(found) = gear_rpc_client::observation::find_reply(
                &chain,
                next..=finalized_number,
                id.into_bytes().into(),
                |_, _| {},
            )
            .await?
            {
                let number = found.number;
                let block_hash = found.block_hash;
                let message = found.event.message;
                let details = message.details().expect("discovered reply has details");
                ensure!(
                    message.source() == target && message.destination().as_ref() == sender,
                    "Original checkpoint reply actor or recipient changed; HOLD"
                );
                ensure!(
                    api.get_block_hash(number).await? == block_hash
                        && api.get_block_hash(finalized_number).await? == finalized_hash,
                    "Original checkpoint reply finalized pin changed; HOLD"
                );
                ensure!(details.to_reply_code().is_success(),
                    "Original checkpoint terminal reply failed: {:?}, payload=0x{}; HOLD",
                    details.to_reply_code(), hex::encode(message.payload_bytes()));
                log::info!("Checkpoint original {original_hash:#x} reply finalized at {block_hash:#x}");
                return Ok(message.payload_bytes().to_vec());
            }
            ensure!(api.get_block_hash(finalized_number).await? == finalized_hash,
                "Checkpoint finality changed while waiting for original reply; HOLD");
            previous = (finalized_number, finalized_hash);
            next = finalized_number.checked_add(1).ok_or_else(|| anyhow!("Checkpoint source height overflow; HOLD"))?;
            time::sleep(Duration::from_secs(3)).await;
        }
    }.await;
    result.with_context(|| {
        format!("Original checkpoint {original_hash:#x}; HOLD/reconcile before retry")
    })
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
