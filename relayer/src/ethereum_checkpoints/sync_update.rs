use super::*;
use checkpoint_light_client_client::{traits::ServiceSyncUpdate as _, ServiceSyncUpdate};
use ethereum_beacon_client::{utils, BeaconClient};
use std::ops::ControlFlow::{self, *};

pub fn spawn_receiver(beacon_client: BeaconClient, sender: Sender<SyncUpdate>) {
    tokio::spawn(receive_updates(
        beacon_client,
        sender,
        Duration::from_secs(DELAY_SECS_UPDATE_REQUEST),
    ));
}

async fn receive_updates(
    beacon_client: BeaconClient,
    sender: Sender<SyncUpdate>,
    interval: Duration,
) {
    log::info!("Update receiver spawned");
    let mut failures = 0;
    loop {
        match receive(&beacon_client, &sender).await {
            Ok(Break(_)) => break,
            Ok(Continue(_)) => failures = 0,
            Err(e) => {
                log::error!("{e:?}");
                failures += 1;
                if failures >= COUNT_FAILURE {
                    break;
                }
            }
        }
        time::sleep(interval).await;
    }
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

    let full_slot = update.finalized_header.slot;
    let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
        &update.sync_aggregate.sync_committee_signature.0 .0[..],
    )
    .map_err(|e| anyhow!("Failed to deserialize point on G2: {e:?}"))?;
    let sync_aggregate_encoded = update.sync_aggregate.encode();
    let sync_update = utils::sync_update_from_update(signature, update);

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

    // Advance committee ownership before using a newer finality-only header.
    if finality_update.finalized_header.slot > full_slot {
        let signature = <G2 as ark_serialize::CanonicalDeserialize>::deserialize_compressed(
            &finality_update.sync_aggregate.sync_committee_signature.0 .0[..],
        )
        .map_err(|e| anyhow!("Failed to deserialize finality signature: {e:?}"))?;
        let sync_aggregate_encoded = finality_update.sync_aggregate.encode();
        let sync_update = utils::sync_update_from_finality(signature, finality_update);
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
    let mut service = ServiceSyncUpdate::new(remoting.clone());

    service
        .process(sync_update, sync_aggregate_encoded)
        .with_gas_limit(gas_limit)
        .send_recv(program_id.into())
        .await
        .map_err(|e| anyhow!("Failed to apply sync committee: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn successful_updates_reset_the_consecutive_failure_budget() {
        // Unoptimized committee deserialization exceeds the test harness's default stack.
        std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let updates: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/src/checkpoint_light_client/chain-data/sepolia-update-640.json"
        )).unwrap();
        let finality = updates[0].to_string();
        let updates = updates.to_string();
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let count = stream.read(&mut request).await.unwrap();
                let request = std::str::from_utf8(&request[..count]).unwrap();
                let (status, body) = if request.starts_with("GET /eth/v1/beacon/light_client/finality_update ") {
                    let attempt = observed.fetch_add(1, Ordering::SeqCst);
                    if matches!(attempt, 1 | 3) {
                        ("200 OK", finality.as_str())
                    } else {
                        ("503 Service Unavailable", "{}")
                    }
                } else {
                    assert!(request.starts_with("GET /eth/v1/beacon/light_client/updates?"));
                    ("200 OK", updates.as_str())
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = BeaconClient::new(endpoint, Some(Duration::from_secs(1))).await.unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        time::timeout(Duration::from_secs(5), receive_updates(client, sender, Duration::ZERO))
            .await.unwrap();
        server.abort();
        assert_eq!(requests.load(Ordering::SeqCst), 7);
        assert!(receiver.recv().await.is_some());
        assert!(receiver.recv().await.is_some());
        assert!(receiver.recv().await.is_none());
            });
        }).unwrap().join().unwrap();
    }

    #[test]
    fn newer_finality_does_not_discard_the_verified_following_committee_update() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let fixture: serde_json::Value = serde_json::from_str(include_str!(
                            "../../../tests/src/checkpoint_light_client/chain-data/synthetic-fulu-transitions.json"
                        ))
                        .unwrap();
                        let finality = fixture["updates"][4].to_string();
                        let updates = format!("[{}]", fixture["updates"][3]);
                        let expected_full = full_finalized_slot(&fixture, 3);
                        let expected_finality = full_finalized_slot(&fixture, 4);
                        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                            .await
                            .unwrap();
                        let endpoint = format!("http://{}", listener.local_addr().unwrap());
                        let server = tokio::spawn(async move {
                            for _ in 0..2 {
                                let (mut stream, _) = listener.accept().await.unwrap();
                                let mut request = [0; 4096];
                                let count = stream.read(&mut request).await.unwrap();
                                let request = std::str::from_utf8(&request[..count]).unwrap();
                                let body = if request.starts_with("GET /eth/v1/beacon/light_client/finality_update ") {
                                    &finality
                                } else {
                                    assert!(request.starts_with("GET /eth/v1/beacon/light_client/updates?start_period=201&count=1 "));
                                    &updates
                                };
                                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                            }
                        });
                        let client = BeaconClient::new(endpoint, Some(Duration::from_secs(1)))
                            .await
                            .unwrap();
                        let (sender, mut receiver) = mpsc::channel(4);
                        assert!(matches!(receive(&client, &sender).await.unwrap(), Continue(())));
                        drop(sender);
                        let full = receiver.recv().await.unwrap().sync_update;
                        let finality = receiver.recv().await.unwrap().sync_update;
                        assert_eq!(full.finalized_header.slot, expected_full);
                        assert!(full.sync_committee_next_pub_keys.is_some());
                        assert_eq!(finality.finalized_header.slot, expected_finality);
                        assert!(finality.sync_committee_next_pub_keys.is_none());
                        assert!(receiver.recv().await.is_none());
                        server.await.unwrap();
                    });
            })
            .unwrap()
            .join()
            .unwrap();
    }

    fn full_finalized_slot(fixture: &serde_json::Value, index: usize) -> u64 {
        fixture["updates"][index]["data"]["finalized_header"]["beacon"]["slot"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }
}
