use crate::message_relayer::{common::GearBlock, eth_to_gear::api_provider::ApiProviderConnection};
use futures::StreamExt;
use gsdk::Event;

pub async fn queue_cleaner(
    mut api_provider: ApiProviderConnection,
    suri: String,
    delay: u64,
) -> anyhow::Result<()> {
    let client = api_provider.client();

    reset_overflowed_queue_from_storage(&mut api_provider, &suri).await?;

    let mut blocks = client.api.subscribe_finalized_blocks().await?;

    loop {
        match blocks.next().await {
            Some(Ok(block)) => {
                let gear_block = GearBlock::from_subxt_block(&client, block).await?;
                if !queue_overflowed(&gear_block) {
                    continue;
                }

                log::warn!(
                    "Queue overflowed event found at block #{}",
                    gear_block.number()
                );
                let start = std::time::Instant::now();
                if let Err(err) =
                    reset_overflowed_queue(&api_provider, &gear_block, &suri, delay).await
                {
                    log::error!(
                        "Failed to reset overflowed queue at block #{}: {err}",
                        gear_block.number()
                    );
                    return Err(err);
                } else {
                    log::info!(
                        "Overflowed queue reset at block #{} in {:.4} seconds",
                        gear_block.number(),
                        start.elapsed().as_secs_f64()
                    );
                }
            }

            Some(Err(err)) => {
                log::error!("Queue cleaner block subscription error: {err}");
                // Try to reconnect after a short delay
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                match api_provider.reconnect().await {
                    Ok(()) => {
                        log::info!("Queue cleaner reconnected");
                        blocks = api_provider
                            .client()
                            .api
                            .subscribe_finalized_blocks()
                            .await?;
                    }
                    Err(err) => {
                        log::error!("Queue cleaner unable to reconnect: {err}");
                        return Err(err);
                    }
                };

                reset_overflowed_queue_from_storage(&mut api_provider, &suri).await?;
            }

            None => {
                log::error!("Queue cleaner block subscription ended unexpectedly");
                return Err(anyhow::anyhow!("Block subscription ended"));
            }
        }
    }
}

async fn reset_overflowed_queue_from_storage(
    api_provider: &mut ApiProviderConnection,
    suri: &str,
) -> anyhow::Result<()> {
    let client = api_provider.client();
    let block = match client.fetch_queue_overflowed_since().await {
        Ok(Some(block)) => {
            log::info!("Last overflowed queue reset at block #{block}",);
            if block == 0 {
                // Nothing to do
                return Ok(());
            }

            block
        }
        Ok(None) => {
            log::info!("No overflowed queue reset found in storage");
            return Ok(());
        }
        Err(err) => {
            log::error!("Failed to fetch queue overflowed since: {err}");
            return Ok(());
        }
    };
    log::info!("Found unprocessed overflowed queue event at block #{block}",);
    let block_hash = client.block_number_to_hash(block).await?;
    let block = client.get_block_at(block_hash).await?;
    let block = GearBlock::from_subxt_block(&client, block).await?;
    reset_overflowed_queue(api_provider, &block, suri, 15).await
}

fn queue_overflowed(block: &GearBlock) -> bool {
    block.events().iter().any(|event| {
        matches!(
            event,
            Event::GearEthBridge(gsdk::gear::gear_eth_bridge::Event::QueueOverflowed)
        )
    })
}

async fn reset_overflowed_queue(
    api_provider: &ApiProviderConnection,
    block: &GearBlock,
    suri: &str,
    delay: u64,
) -> anyhow::Result<()> {
    let mut api_provider = api_provider.clone();
    let suri = suri.to_string();
    let block_number = block.number();
    // spawn tokio task to avoid blocking the main loop
    tokio::task::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        match reset_overflowed_queue_impl(&mut api_provider, block_number, &suri).await {
            Ok(()) => (),
            Err(err) => {
                // If anything goes wrong, we exit the process and just wait
                // for the orchestrator to restart us.
                log::error!("Failed to reset overflowed queue at block #{block_number}: {err}",);
                std::process::exit(1);
            }
        }
    });
    Ok(())
}

async fn reset_overflowed_queue_impl(
    api_provider: &mut ApiProviderConnection,
    block_number: u32,
    suri: &str,
) -> anyhow::Result<()> {
    let client = api_provider.client();
    let Some(finality) = client.prove_finality(block_number).await? else {
        log::error!("No finality proof for block {block_number}");
        return Err(anyhow::anyhow!("No finality proof"));
    };

    let gclient = api_provider.gclient_client(suri)?;
    gclient.reset_overflowed_queue(finality).await?;
    Ok(())
}
