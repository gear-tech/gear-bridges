use prometheus::{IntCounter, IntGauge};
use utils_prometheus::impl_metered_service;

impl_metered_service! {
    pub struct Updates {
        pub fetched_sync_update_slot: IntGauge,
        pub total_fetched_finality_updates: IntCounter,
        pub total_fetched_committee_updates: IntCounter,
        pub processed_finality_updates: IntCounter,
        pub processed_committee_updates: IntCounter,
    }
}

impl Updates {
    pub fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            fetched_sync_update_slot: IntGauge::new(
                "checkpoints_relayer_fetched_sync_update_slot",
                "The slot of the last applied update",
            )?,
            total_fetched_finality_updates: IntCounter::new(
                "checkpoints_relayer_total_fetched_finality_updates",
                "Total amount of fetched finality updates",
            )?,
            total_fetched_committee_updates: IntCounter::new(
                "checkpoints_relayer_total_fetched_committee_updates",
                "Total amount of fetched committee updates",
            )?,
            processed_finality_updates: IntCounter::new(
                "checkpoints_relayer_processed_finality_updates",
                "Amount of processed finality updates",
            )?,
            processed_committee_updates: IntCounter::new(
                "checkpoints_relayer_processed_committee_updates",
                "Amount of processed committee updates",
            )?,
        })
    }
}
