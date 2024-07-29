use super::*;
use checkpoint_light_client_io::{replay_back::{Status, StatusStart}, BeaconBlockHeader};
use utils::ErrorNotFound;

pub async fn execute(
   client_http: &Client,
   beacon_endpoint: &str,
   client: &GearApi,
   listener: &mut EventListener,
   program_id: [u8; 32],
   replayed_slot: Option<Slot>,
   checkpoint: (Slot, Hash256),
   sync_update: SyncCommitteeUpdate,
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

       log::info!("The ongoing replaying back finished");

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

   let slot_last = sync_update.finalized_header.slot;
   for update in updates {
       let slot_end = update.data.finalized_header.slot;
       let Some(mut slots_batch_iter) = SlotsBatchIter::new(slot_start, slot_end, SIZE_BATCH) else {
           log::error!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_end (update) = {slot_end}.");

           return;
       };

       slot_start = slot_end;

       let Ok(signature) = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
            &update.data.sync_aggregate.sync_committee_signature.0.0[..],
        ) else {
            log::error!("Failed to deserialize point on G2 (replay back)");
            return;
        };

       let sync_update = utils::sync_update_from_update(signature, update.data);
       if replay_back_slots_start(client_http, beacon_endpoint, client, listener, program_id, slots_batch_iter.next(), sync_update).await.is_none() {
           return;
       }

       if replay_back_slots(client_http, beacon_endpoint, client, listener, program_id, slots_batch_iter).await.is_none() {
           return;
       }

       if slot_end == slot_last {
           // the provided sync_update is a sync committee update
           return;
       }
   }

   let Some(mut slots_batch_iter) = SlotsBatchIter::new(slot_start, slot_last, SIZE_BATCH) else {
       log::error!("Failed to create slots_batch::Iter with slot_start = {slot_start}, slot_last = {slot_last}.");

       return;
   };

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
   let payload = Handle::ReplayBack(request_headers(client_http, beacon_endpoint, slot_start, slot_end).await?);

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
       HandleResult::ReplayBack(Some(Status::InProcess | Status::Finished))
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

   let payload = Handle::ReplayBackStart {
       sync_update,
       headers: request_headers(client_http, beacon_endpoint, slot_start, slot_end).await?,
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
       HandleResult::ReplayBackStart(Ok(StatusStart::InProgress | StatusStart::Finished))
   ).then_some(())
}

pub async fn request_headers(
    client_http: &Client,
    beacon_endpoint: &str,
    slot_start: Slot,
    slot_end: Slot,
) -> Option<Vec<BeaconBlockHeader>> {
    let batch_size = (slot_end - slot_start) as usize;
    let mut requests_headers = Vec::with_capacity(batch_size);
    for i in slot_start..slot_end {
        requests_headers.push(utils::get_block_header(&client_http, &beacon_endpoint, i));
    }

    futures::future::join_all(requests_headers)
        .await
        .into_iter()
        .filter(|maybe_header| {
            match maybe_header {
                Err(e) if e.downcast_ref::<ErrorNotFound>().is_some() => false,
                _ => true,
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| log::error!("Failed to fetch block headers ([{slot_start}; {slot_end})): {e:?}"))
        .ok()
}
