use super::*;
use anyhow::{anyhow, Result as AnyResult};
pub use checkpoint_light_client_io::sync_update::Error;
use std::ops::ControlFlow::{self, *};

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
            match receive(
                &client_http,
                &beacon_endpoint,
                &sender,
                delay,
                &mut failures,
            )
            .await
            {
                Break(_) => break,
                Continue(_) => (),
            }
        }
    });
}

async fn receive(
    client_http: &Client,
    beacon_endpoint: &str,
    sender: &Sender<SyncCommitteeUpdate>,
    delay: Duration,
    failures: &mut usize,
) -> ControlFlow<()> {
    // helper macro
    macro_rules! check_and_return {
        () => {
            *failures += 1;
            if *failures >= COUNT_FAILURE {
                return Break(());
            }

            time::sleep(delay).await;

            return Continue(());
        };
    }

    let finality_update = match utils::get_finality_update(client_http, beacon_endpoint).await {
        Ok(finality_update) => finality_update,

        Err(e) => {
            log::error!("Unable to fetch FinalityUpdate: {e:?}");
            check_and_return!();
        }
    };

    let period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = match utils::get_updates(client_http, beacon_endpoint, period, 1).await {
        Ok(updates) => updates,
        Err(e) => {
            log::error!("Unable to fetch Updates: {e:?}");
            check_and_return!();
        }
    };

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => {
            log::error!("Requested single update");
            check_and_return!();
        }
    };

    let reader_signature = if update.finalized_header.slot >= finality_update.finalized_header.slot
    {
        &update.sync_aggregate.sync_committee_signature.0 .0[..]
    } else {
        &finality_update.sync_aggregate.sync_committee_signature.0 .0[..]
    };

    let Ok(signature) =
        <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(reader_signature)
    else {
        log::error!("Failed to deserialize point on G2");
        check_and_return!();
    };

    let sync_update = if update.finalized_header.slot >= finality_update.finalized_header.slot {
        utils::sync_update_from_update(signature, update)
    } else {
        utils::sync_update_from_finality(signature, finality_update)
    };

    if sender.send(sync_update).await.is_err() {
        return Break(());
    }

    time::sleep(delay).await;

    Continue(())
}

pub async fn try_to_apply(
    client: &GearApi,
    listener: &mut EventListener,
    program_id: [u8; 32],
    sync_update: SyncCommitteeUpdate,
    gas_limit: u64,
) -> AnyResult<Result<(), Error>> {
    let payload = Handle::SyncUpdate(sync_update);
    let (message_id, _) = client
        .send_message(program_id.into(), payload, gas_limit, 0)
        .await
        .map_err(|e| anyhow!("Failed to send message: {e:?}"))?;

    let (_message_id, payload, _value) = listener
        .reply_bytes_on(message_id)
        .await
        .map_err(|e| anyhow!("Failed to get reply: {e:?}"))?;
    let payload =
        payload.map_err(|e| anyhow!("Failed to get replay payload to SyncUpdate: {e:?}"))?;
    let result_decoded = HandleResult::decode(&mut &payload[..])
        .map_err(|e| anyhow!("Failed to decode HandleResult of SyncUpdate: {e:?}"))?;

    log::debug!("try_to_apply; result_decoded = {result_decoded:?}");

    match result_decoded {
        HandleResult::SyncUpdate(result) => Ok(result),
        _ => Err(anyhow!("Wrong response type")),
    }
}
