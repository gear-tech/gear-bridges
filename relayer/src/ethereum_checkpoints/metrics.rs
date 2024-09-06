use prometheus::{IntCounter, IntGauge};
use utils_prometheus::impl_metered_service;

impl_metered_service! {
    pub struct Updates {
        pub fetched_sync_update_slot: IntGauge,
        pub total_fetched_finality_updates: IntCounter,
        pub processed_finality_updates: IntCounter,
        pub processed_committee_updates: IntCounter,
        pub account_total_balance: IntGauge,
    }
}

impl Updates {
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
            processed_finality_updates: IntCounter::new(
                "checkpoints_relayer_processed_finality_updates",
                "Amount of processed finality updates",
            )?,
            processed_committee_updates: IntCounter::new(
                "checkpoints_relayer_processed_committee_updates",
                "Amount of processed committee updates",
            )?,
            account_total_balance: IntGauge::new(
                "checkpoints_relayer_account_total_balance",
                "The total balance of the account used to send messages",
            )?,
        })
    }
}
