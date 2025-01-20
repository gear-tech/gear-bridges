use super::*;
use checkpoint_light_client_client::traits::SyncUpdate as _;
use ethereum_beacon_client::{utils, BeaconClient};
use std::ops::ControlFlow::{self, *};

pub fn spawn_receiver(beacon_client: BeaconClient, sender: Sender<SyncUpdate>) {
    tokio::spawn(async move {
        log::info!("Update receiver spawned");

        let mut failures = 0;
        loop {
            match receive(&beacon_client, &sender).await {
                Ok(Break(_)) => break,
                Ok(Continue(_)) => (),
                Err(e) => {
                    log::error!("{e:?}");

                    failures += 1;
                    if failures >= COUNT_FAILURE {
                        break;
                    }
                }
            }

            time::sleep(Duration::from_secs(DELAY_SECS_UPDATE_REQUEST)).await;
        }
    });
}

async fn receive(
    beacon_client: &BeaconClient,
    sender: &Sender<SyncUpdate>,
) -> AnyResult<ControlFlow<()>> {
    let finality_update = beacon_client
        .get_finality_update()
        .await
        .map_err(|e| anyhow!("Unable to fetch FinalityUpdate: {e:?}"))?;

    let period = eth_utils::calculate_period(finality_update.finalized_header.slot);
    let mut updates = beacon_client
        .get_updates(period, 1)
        .await
        .map_err(|e| anyhow!("Unable to fetch Updates: {e:?}"))?;

    let update = match updates.pop() {
        Some(update) if updates.is_empty() => update.data,
        _ => return Err(anyhow!("Requested single update")),
    };

    let reader_signature = if update.finalized_header.slot >= finality_update.finalized_header.slot
    {
        &update.sync_aggregate.sync_committee_signature.0 .0[..]
    } else {
        &finality_update.sync_aggregate.sync_committee_signature.0 .0[..]
    };

    let signature =
        <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(reader_signature)
            .map_err(|e| anyhow!("Failed to deserialize point on G2: {e:?}"))?;

    let (sync_aggregate_encoded, sync_update) =
        if update.finalized_header.slot >= finality_update.finalized_header.slot {
            (
                update.sync_aggregate.encode(),
                utils::sync_update_from_update(signature, update),
            )
        } else {
            (
                finality_update.sync_aggregate.encode(),
                utils::sync_update_from_finality(signature, finality_update),
            )
        };

    if sender
        .send(SyncUpdate {
            sync_update,
            sync_aggregate_encoded,
        })
        .await
        .is_err()
    {
        return Ok(Break(()));
    }

    Ok(Continue(()))
}

pub async fn try_to_apply(
    remoting: &GClientRemoting,
    program_id: [u8; 32],
    sync_update: SyncCommitteeUpdate,
    sync_aggregate_encoded: Vec<u8>,
    gas_limit: u64,
) -> AnyResult<Result<(), Error>> {
    let mut service = checkpoint_light_client_client::SyncUpdate::new(remoting.clone());

    service
        .process(sync_update, sync_aggregate_encoded)
        .with_gas_limit(gas_limit)
        .send_recv(program_id.into())
        .await
        .map_err(|e| anyhow!("Failed to apply sync committee: {e:?}"))
}
