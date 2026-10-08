use super::*;
use checkpoint_light_client_client::traits::ServiceReplayBack as _;
use checkpoint_light_client_io::ReplayBackStatus;
use ethereum_beacon_client::{self, BeaconClient};
use futures::{stream, Stream, StreamExt};

pub struct Args<'a> {
    pub beacon_client: &'a BeaconClient,
    pub remoting: &'a GClientRemoting,
    pub program_id: [u8; 32],
    pub gas_limit: u64,
    pub replay_back: Option<ReplayBack>,
    pub checkpoint: (Slot, Hash256),
    pub sync_update: SyncCommitteeUpdate,
    pub size_batch: u64,
    pub sync_aggregate_encoded: Vec<u8>,
}

pub async fn execute(args: Args<'_>) -> AnyResult<()> {
    let Args {
        beacon_client,
        remoting,
        program_id,
        gas_limit,
        replay_back,
        checkpoint,
        sync_update,
        size_batch,
        sync_aggregate_encoded,
    } = args;

    log::info!("Replaying back started");

    let (mut slot_start, _) = checkpoint;
    if let Some(ReplayBack {
        finalized_header,
        last_header: slot_end,
    }) = replay_back
    {
        let slots_batch_iter = SlotsBatchIter::new(slot_start, slot_end, size_batch)
            .ok_or(anyhow!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_end = {slot_end}."))?;

        replay_back_slots(
            beacon_client,
            remoting,
            program_id,
            gas_limit,
            ReplayBackStatus::InProcess,
            slots_batch_iter,
        )
        .await?;

        slot_start = finalized_header;
    }

    if slot_start >= sync_update.finalized_header.slot {
        return Ok(());
    }

    let period_start = 1 + eth_utils::calculate_period(slot_start);
    let updates = if period_start <= eth_utils::calculate_period(sync_update.finalized_header.slot)
    {
        beacon_client
            .get_updates(period_start, MAX_REQUEST_LIGHT_CLIENT_UPDATES)
            .await
            .map_err(|e| anyhow!("Failed to get updates for period {period_start}: {e:?}"))?
    } else {
        Vec::new()
    };

    let slot_last = sync_update.finalized_header.slot;
    for update in updates {
        let slot_end = update.data.finalized_header.slot;
        let mut slots_batch_iter = SlotsBatchIter::new(slot_start, slot_end, size_batch)
            .ok_or(anyhow!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_end = {slot_end}."))?;

        let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
            &update.data.sync_aggregate.sync_committee_signature.0 .0[..],
        )
        .map_err(|e| anyhow!("Failed to deserialize point on G2 (replay back): {e:?}"))?;

        let sync_aggregate_encoded = update.data.sync_aggregate.encode();
        let sync_update =
            ethereum_beacon_client::utils::sync_update_from_update(signature, update.data);
        let status = replay_back_slots_start(
            beacon_client,
            remoting,
            program_id,
            gas_limit,
            slots_batch_iter.next(),
            sync_update,
            sync_aggregate_encoded,
        )
        .await?;

        replay_back_slots(
            beacon_client,
            remoting,
            program_id,
            gas_limit,
            status,
            slots_batch_iter,
        )
        .await?;

        slot_start = slot_end;
        if slot_end >= slot_last {
            return Ok(());
        }
    }

    let mut slots_batch_iter = SlotsBatchIter::new(slot_start, slot_last, size_batch)
        .ok_or(anyhow!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_last = {slot_last}."))?;

    let status = replay_back_slots_start(
        beacon_client,
        remoting,
        program_id,
        gas_limit,
        slots_batch_iter.next(),
        sync_update,
        sync_aggregate_encoded,
    )
    .await?;

    replay_back_slots(
        beacon_client,
        remoting,
        program_id,
        gas_limit,
        status,
        slots_batch_iter,
    )
    .await?;

    log::info!("Replaying back finished");

    Ok(())
}

async fn replay_back_slots(
    beacon_client: &BeaconClient,
    remoting: &GClientRemoting,
    program_id: [u8; 32],
    gas_limit: u64,
    status: ReplayBackStatus,
    slots_batch_iter: SlotsBatchIter,
) -> AnyResult<()> {
    finish_replay(
        status,
        stream::iter(slots_batch_iter).then(|(slot_start, slot_end)| {
            log::debug!("slot_start = {slot_start}, slot_end = {slot_end}");
            replay_back_slots_inner(
                beacon_client,
                remoting,
                program_id,
                slot_start,
                slot_end,
                gas_limit,
            )
        }),
    )
    .await
}

async fn finish_replay(
    mut status: ReplayBackStatus,
    statuses: impl Stream<Item = AnyResult<ReplayBackStatus>>,
) -> AnyResult<()> {
    futures::pin_mut!(statuses);
    while !matches!(status, ReplayBackStatus::Finished) {
        status = statuses.next().await
            .ok_or_else(|| anyhow!("Replay remains InProcess after available history; retain its original base and retry"))??;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn replay_back_slots_inner(
    beacon_client: &BeaconClient,
    remoting: &GClientRemoting,
    program_id: [u8; 32],
    slot_start: Slot,
    slot_end: Slot,
    gas_limit: u64,
) -> AnyResult<ReplayBackStatus> {
    let mut service = checkpoint_light_client_client::ServiceReplayBack::new(remoting.clone());

    service
        .process(beacon_client.request_headers(slot_start, slot_end).await?)
        .with_gas_limit(gas_limit)
        .send_recv(program_id.into())
        .await
        .map_err(|e| anyhow!("Failed to send ReplayBack message: {e:?}"))?
        .map_err(|e| anyhow!("Backreplay failed: {e:?}"))
}

#[allow(clippy::too_many_arguments)]
async fn replay_back_slots_start(
    beacon_client: &BeaconClient,
    remoting: &GClientRemoting,
    program_id: [u8; 32],
    gas_limit: u64,
    slots: Option<(Slot, Slot)>,
    sync_update: SyncCommitteeUpdate,
    sync_aggregate_encoded: Vec<u8>,
) -> AnyResult<ReplayBackStatus> {
    let Some((slot_start, slot_end)) = slots else {
        return Err(anyhow!("Cannot start replay without a header batch"));
    };
    let mut service = checkpoint_light_client_client::ServiceReplayBack::new(remoting.clone());

    service
        .start(
            sync_update,
            sync_aggregate_encoded,
            beacon_client.request_headers(slot_start, slot_end).await?,
        )
        .with_gas_limit(gas_limit)
        .send_recv(program_id.into())
        .await
        .map_err(|e| anyhow!("Failed to send ReplayBack start message: {e:?}"))?
        .map_err(|e| anyhow!("Failed to start ReplayBack failed: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exhausted_incomplete_replay_cannot_report_success() {
        for remaining in [None, Some(ReplayBackStatus::InProcess)] {
            let statuses = stream::iter(remaining.into_iter().map(Ok));
            assert!(
                finish_replay(ReplayBackStatus::InProcess, statuses)
                    .await
                    .is_err(),
                "missing history must leave replay unfinished",
            );
        }
    }

    #[tokio::test]
    async fn finished_replay_never_polls_another_submission() {
        for (initial, expected_polls) in [
            (ReplayBackStatus::Finished, 0),
            (ReplayBackStatus::InProcess, 1),
        ] {
            let polls = std::cell::Cell::new(0);
            let statuses = stream::iter([ReplayBackStatus::Finished, ReplayBackStatus::InProcess])
                .map(|status| {
                    polls.set(polls.get() + 1);
                    assert_eq!(polls.get(), 1, "submitted another batch after Finished");
                    Ok(status)
                });
            finish_replay(initial, statuses).await.unwrap();
            assert_eq!(polls.get(), expected_polls);
        }
    }
}
