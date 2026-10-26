use gear_common::api_provider::ApiProviderConnection;
use gear_rpc_client::{GearApi, GearHeader};
use parity_scale_codec::Decode;
use primitive_types::H256;
use sc_consensus_grandpa::FinalityProof;
use sp_consensus_grandpa::GrandpaJustification;
use std::time::Duration;

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(90);

/// A single owned loop: no detached submissions and no process-wide exit on RPC failure.
pub async fn queue_cleaner(
    mut api_provider: ApiProviderConnection,
    suri: String,
    delay: u64,
) -> anyhow::Result<()> {
    let base = Duration::from_secs(delay.max(1));
    let mut backoff = base;
    loop {
        tokio::time::sleep(backoff).await;
        match tokio::time::timeout(ATTEMPT_TIMEOUT, reset_once(&mut api_provider, &suri)).await {
            Ok(Ok(())) => backoff = base,
            failure => {
                // Timeout can mean a submitted reset: never reuse the old overflow or proof.
                // The next iteration observes finalized storage before doing any new work.
                log::warn!("Queue cleaner attempt pending/failed: {failure:?}");
                backoff = backoff
                    .saturating_mul(2)
                    .min(base.max(Duration::from_secs(60)));
                if let Ok(Err(error)) =
                    tokio::time::timeout(ATTEMPT_TIMEOUT, api_provider.reconnect()).await
                {
                    log::warn!("Queue cleaner reconnect unavailable: {error:#}");
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Overflow {
    since: u32,
    queue_id: u64,
    set_id: u64,
}

async fn finalized_overflow(api: &GearApi) -> anyhow::Result<(H256, Option<Overflow>)> {
    let hash = api.latest_finalized_block().await?;
    let address = gsdk::gear::storage()
        .gear_eth_bridge()
        .queue_overflowed_since();
    let since: u32 = match api.api.storage_fetch_at(&address, Some(hash)).await {
        Ok(since) => since,
        Err(gsdk::Error::StorageEntryNotFound) => return Ok((hash, None)),
        Err(error) => return Err(error.into()),
    };
    if since == 0 {
        return Ok((hash, None));
    }
    let number = api.block_hash_to_number(hash).await?;
    anyhow::ensure!(
        since <= number,
        "HOLD: overflow is beyond finalized source height"
    );
    let queue_id = crate::message_relayer::common::gear::queue_id_at(api, number, hash).await?;
    let set_id = api.authority_set_id(hash).await?;
    Ok((
        hash,
        Some(Overflow {
            since,
            queue_id,
            set_id,
        }),
    ))
}

fn proof_target(bytes: &[u8]) -> anyhow::Result<(H256, u32)> {
    let mut input = bytes;
    let proof = FinalityProof::<GearHeader>::decode(&mut input)?;
    anyhow::ensure!(input.is_empty(), "HOLD: trailing GRANDPA proof bytes");
    let mut input = proof.justification.as_slice();
    let justification = GrandpaJustification::<GearHeader>::decode(&mut input)?;
    anyhow::ensure!(
        input.is_empty() && justification.commit.target_hash == proof.block,
        "HOLD: malformed GRANDPA target/justification"
    );
    Ok((proof.block.0.into(), justification.commit.target_number))
}

async fn reset_once(api_provider: &mut ApiProviderConnection, suri: &str) -> anyhow::Result<()> {
    let api = api_provider.client();
    let (pin, Some(overflow)) = finalized_overflow(&api).await? else {
        return Ok(());
    };
    // Prove a CURRENT finalized target, not the old overflow block. With stable
    // keys a GRANDPA set change need not reset QueueId, but invalidates old proofs.
    let number = api.block_hash_to_number(pin).await?;
    let finality = api
        .prove_finality(number)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Current finalized GRANDPA proof unavailable"))?;
    let (target_hash, target_number) = proof_target(&finality)?;
    let latest = api.latest_finalized_block().await?;
    anyhow::ensure!(
        target_number >= number
            && target_number >= overflow.since
            && target_number <= api.block_hash_to_number(latest).await?
            && api.block_number_to_hash(target_number).await? == target_hash
            && api.signed_by_authority_set_id(target_hash).await? == overflow.set_id,
        "Current finalized GRANDPA target/set not yet available"
    );
    let (_, current) = finalized_overflow(&api).await?;
    if current != Some(overflow) {
        // Another cleaner/reset or set transition won. Re-observe; never submit stale work.
        return Ok(());
    }
    api_provider
        .gclient_client(suri)?
        .reset_overflowed_queue(finality)
        .await?;
    // Even an ambiguous response is handled by the same fresh-storage retry loop.
    let (_, remaining) = finalized_overflow(&api).await?;
    log::info!("Queue reset submitted for {overflow:?}; finalized observation: {remaining:?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    /// Reconstructed from supplied HTML VB-16 / audited 7fe1b032. This executes
    /// the actual pinned runtime verifier, never the magic-proof unit stub.
    #[tokio::test]
    #[ignore = "requires prepared owned loopback Gear overflow and stable-key GRANDPA transition; see docs/merkle-roots.md"]
    async fn real_runtime_overflow_survives_stable_key_set_transition() -> anyhow::Result<()> {
        let endpoint = std::env::var("QUEUE_CLEANER_TEST_RPC")?;
        let url = url::Url::parse(&endpoint)?;
        anyhow::ensure!(
            matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
            "Runtime mutation test is restricted to owned loopback nodes"
        );
        let expected_genesis: H256 = std::env::var("QUEUE_CLEANER_TEST_GENESIS")?.parse()?;
        let expected_code = std::env::var("QUEUE_CLEANER_TEST_CODE_SHA256")?;
        let suri = std::env::var("QUEUE_CLEANER_TEST_SURI")?;
        let old_proof = tokio::fs::read(std::env::var("QUEUE_CLEANER_TEST_OLD_PROOF")?).await?;
        let provider = gear_common::api_provider::ApiProvider::new(endpoint, 1).await?;
        let mut connection = provider.connection();
        provider.spawn();
        let api = connection.client();
        anyhow::ensure!(
            api.block_number_to_hash(0).await? == expected_genesis,
            "Wrong owned genesis"
        );
        let (pin, overflow) = finalized_overflow(&api).await?;
        let overflow =
            overflow.ok_or_else(|| anyhow::anyhow!("Fixture has no finalized overflow"))?;
        let code: Option<String> = api
            .api
            .rpc()
            .request(
                "state_getStorage",
                subxt_rpcs::rpc_params!["0x3a636f6465", pin],
            )
            .await?;
        let code = code.ok_or_else(|| anyhow::anyhow!("Missing runtime code"))?;
        let code = hex::decode(code.strip_prefix("0x").unwrap_or(&code))?;
        anyhow::ensure!(
            hex::encode(sha2::Sha256::digest(&code)) == expected_code,
            "Runtime is not independently pinned"
        );
        let (old_hash, old_number) = proof_target(&old_proof)?;
        anyhow::ensure!(
            old_number >= overflow.since && api.block_number_to_hash(old_number).await? == old_hash,
            "Old proof does not authenticate the overflowing queue"
        );
        let old_set = api.signed_by_authority_set_id(old_hash).await?;
        anyhow::ensure!(
            old_set < overflow.set_id,
            "Fixture lacks a GRANDPA set transition"
        );
        anyhow::ensure!(
            api.fetch_authority_set(old_set).await?
                == api.fetch_authority_set(overflow.set_id).await?,
            "Fixture changed keys, not just the GRANDPA set"
        );
        anyhow::ensure!(
            crate::message_relayer::common::gear::queue_id_at(&api, old_number, old_hash).await?
                == overflow.queue_id,
            "Fixture reset QueueId during set transition"
        );
        let signer = connection.gclient_client(&suri)?;
        anyhow::ensure!(
            signer.reset_overflowed_queue(vec![1, 2, 3]).await.is_err(),
            "Runtime accepted a magic/invalid proof"
        );
        anyhow::ensure!(
            signer.reset_overflowed_queue(old_proof).await.is_err(),
            "Runtime unexpectedly accepted old-set proof"
        );
        anyhow::ensure!(
            finalized_overflow(&api).await?.1 == Some(overflow),
            "Negative controls changed overflow"
        );
        tokio::time::timeout(ATTEMPT_TIMEOUT, reset_once(&mut connection, &suri)).await??;
        tokio::time::timeout(ATTEMPT_TIMEOUT, async {
            loop {
                let (pin, remaining) = finalized_overflow(&api).await?;
                if remaining.is_none() {
                    let number = api.block_hash_to_number(pin).await?;
                    anyhow::ensure!(
                        crate::message_relayer::common::gear::queue_id_at(&api, number, pin)
                            .await?
                            > overflow.queue_id,
                        "Reset did not advance QueueId"
                    );
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        })
        .await??;
        Ok(())
    }
}
