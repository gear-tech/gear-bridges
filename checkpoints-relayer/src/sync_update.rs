use super::*;
use checkpoint_light_client_io::sync_update::Error;

pub fn spawn_receiver(
    client_http: Client,
    beacon_endpoint: String,
    sender: Sender<SyncCommitteeUpdate>,
    delay: Duration,
) {
    tokio::spawn(async move {
        log::info!("Update receiver spawned");

        let mut failures = 0;

        loop {
            let finality_update = match utils::get_finality_update(&client_http, &beacon_endpoint).await {
                Ok(finality_update) => finality_update,

                Err(e) => {
                    log::error!("Unable to fetch FinalityUpdate: {e:?}");

                    failures += 1;
                    if failures >= COUNT_FAILURE {
                        return;
                    }

                    time::sleep(delay).await;
                    continue;
                }
            };

            let period = eth_utils::calculate_period(finality_update.finalized_header.slot);
            let mut updates = match utils::get_updates(&client_http, &beacon_endpoint, period, 1).await {
                Ok(updates) => updates,
                Err(e) => {
                    log::error!("Unable to fetch Updates: {e:?}");

                    failures += 1;
                    if failures >= COUNT_FAILURE {
                        return;
                    }

                    time::sleep(delay).await;
                    continue;
                }
            };

            let update = match updates.pop() {
                Some(update) if updates.is_empty() => update.data,
                _ => {
                    log::error!("Requested single update");

                    failures += 1;
                    if failures >= COUNT_FAILURE {
                        return;
                    }

                    time::sleep(delay).await;
                    continue;
                }
            };

            let sync_update = if update.finalized_header.slot >= finality_update.finalized_header.slot {
                utils::sync_update_from_update(update)
            } else {
                let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
                    &finality_update.sync_aggregate.sync_committee_signature.0 .0[..],
                );
            
                let Ok(signature) = signature else {
                    log::error!("Failed to deserialize point on G2");

                    failures += 1;
                    if failures >= COUNT_FAILURE {
                        return;
                    }

                    time::sleep(delay).await;
                    continue;
                };
            
                utils::sync_update_from_finality(signature, finality_update)
            };

            if sender.send(sync_update).await.is_err() {
                return;
            }

            time::sleep(delay).await;
        }
    });
}

pub async fn try_to_apply(
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    sync_update: SyncCommitteeUpdate,
) -> Status {
    let payload = Handle::SyncUpdate(sync_update);
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
        HandleResult::SyncUpdate(Err(Error::NotActual)) => Status::NotActual,
        HandleResult::SyncUpdate(Err(Error::ReplayBackRequired {
            replayed_slot,
            checkpoint
        })) => Status::ReplayBackRequired { replayed_slot, checkpoint },
        _ => Status::Error,
    }
 }
