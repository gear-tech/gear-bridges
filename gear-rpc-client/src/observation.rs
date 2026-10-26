//! Hash-pinned original-dispatch discovery. Callers own finalized interval and
//! retained-pin validation, envelope authentication, journal writes and retry policy.

use crate::GearApi;
use anyhow::Result;
use gsdk::{gear, GearConfig};
use std::ops::RangeInclusive;
use subxt::{blocks::ExtrinsicDetails, utils::H256, OnlineClient};

pub struct LocatedExtrinsic {
    pub number: u32,
    pub block_hash: H256,
    pub extrinsic: ExtrinsicDetails<GearConfig, OnlineClient<GearConfig>>,
}

/// Find the first exact extrinsic hash in a caller-qualified finalized interval.
/// Advance discovery only after an entire block has been inspected successfully.
pub async fn find_extrinsic(
    chain: &GearApi,
    range: RangeInclusive<u32>,
    hash: H256,
    mut scanned: impl FnMut(u32, H256),
) -> Result<Option<LocatedExtrinsic>> {
    for number in range {
        let block_hash = chain.block_number_to_hash(number).await?;
        let block = chain.api.blocks().at(block_hash).await?;
        let extrinsics = block.extrinsics().await?;
        if let Some(extrinsic) = extrinsics.iter().find(|extrinsic| extrinsic.hash() == hash) {
            return Ok(Some(LocatedExtrinsic {
                number,
                block_hash,
                extrinsic,
            }));
        }
        scanned(number, block_hash);
    }
    Ok(None)
}

pub struct LocatedReply {
    pub number: u32,
    pub block_hash: H256,
    pub event: gear::gear::events::UserMessageSent,
}

/// Discover a reply by its original message ID, never by payload or actor alone.
/// The caller must authenticate actor/recipient, code, retained bytes and pins.
pub async fn find_reply(
    chain: &GearApi,
    range: RangeInclusive<u32>,
    message_id: H256,
    mut scanned: impl FnMut(u32, H256),
) -> Result<Option<LocatedReply>> {
    for number in range {
        let block_hash = chain.block_number_to_hash(number).await?;
        for event in chain.metadata_events_at(block_hash).await?.iter() {
            let Some(event) = event?.as_event::<gear::gear::events::UserMessageSent>()? else {
                continue;
            };
            if event
                .message
                .details()
                .is_some_and(|details| details.to_message_id().as_ref() == message_id.as_bytes())
            {
                scanned(number, block_hash);
                return Ok(Some(LocatedReply {
                    number,
                    block_hash,
                    event,
                }));
            }
        }
        scanned(number, block_hash);
    }
    Ok(None)
}
