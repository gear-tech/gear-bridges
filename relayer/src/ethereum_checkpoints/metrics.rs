use super::*;
use prometheus::{IntCounter, IntGauge};
use utils_prometheus::impl_metered_service;

pub struct Message {
    pub slot: Slot,
    pub committee_update: bool,
    pub processed: bool,
}

impl_metered_service! {
    struct EventListenerMetrics {
        fetched_sync_update_slot: IntGauge,
        total_fetched_finality_updates: IntCounter,
        total_fetched_committee_updates: IntCounter,
        processed_finality_updates: IntCounter,
        processed_committee_updates: IntCounter,
    }
}

impl EventListenerMetrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            fetched_sync_update_slot: IntGauge::new(
                "checkpoints_relayer_fetched_sync_update_slot",
                "checkpoints_relayer_fetched_sync_update_slot",
            )?,
            total_fetched_finality_updates: IntCounter::new(
                "checkpoints_relayer_total_fetched_finality_updates",
                "checkpoints_relayer_total_fetched_finality_updates",
            )?,
            total_fetched_committee_updates: IntCounter::new(
                "checkpoints_relayer_total_fetched_committee_updates",
                "checkpoints_relayer_total_fetched_committee_updates",
            )?,
            processed_finality_updates: IntCounter::new(
                "checkpoints_relayer_processed_finality_updates",
                "checkpoints_relayer_processed_finality_updates",
            )?,
            processed_committee_updates: IntCounter::new(
                "checkpoints_relayer_processed_committee_updates",
                "checkpoints_relayer_processed_committee_updates",
            )?,
        })
    }
}

pub fn spawn(endpoint_prometheus: String) -> Sender<Message> {
    let (sender, mut receiver) = mpsc::channel::<Message>(100);

    tokio::spawn(async move {
        let service = EventListenerMetrics::new();
        MetricsBuilder::new()
            .register_service(&service)
            .build()
            .run(endpoint_prometheus)
            .await;

        loop {
            let Some(metric_message) = receiver.recv().await else {
                return;
            };

            service
                .fetched_sync_update_slot
                .set(i64::from_le_bytes(metric_message.slot.to_le_bytes()));
            if metric_message.committee_update {
                service.total_fetched_committee_updates.inc();
                if metric_message.processed {
                    service.processed_committee_updates.inc();
                }
            } else {
                service.total_fetched_finality_updates.inc();
                if metric_message.processed {
                    service.processed_finality_updates.inc();
                }
            }
        }
    });

    sender
}
