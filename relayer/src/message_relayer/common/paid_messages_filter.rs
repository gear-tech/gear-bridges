use std::collections::HashMap;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use super::{MessageInBlock, PaidMessage};

pub struct PaidMessagesFilter {
    pending_messages: HashMap<[u8; 32], MessageInBlock>,
    pending_nonces: Vec<[u8; 32]>,

    metrics: Metrics,
}

impl MeteredService for PaidMessagesFilter {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        pending_messages_count: IntGauge = IntGauge::new(
            "paid_messages_filter_pending_messages_count",
            "Amount of discovered but not paid messages",
        )
    }
}

impl PaidMessagesFilter {
    pub fn new() -> Self {
        Self {
            pending_messages: HashMap::default(),
            pending_nonces: vec![],

            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut messages: UnboundedReceiver<MessageInBlock>,
        mut paid_messages: UnboundedReceiver<PaidMessage>,
    ) -> UnboundedReceiver<MessageInBlock> {
        let (sender, receiver) = unbounded_channel();

        tokio::spawn(async move {
            loop {
                let res = self
                    .run_inner(&sender, &mut messages, &mut paid_messages)
                    .await;
                if let Err(err) = res {
                    log::error!("Paid messages filter failed: {}", err);
                }
            }
        });

        receiver
    }

    async fn run_inner(
        &mut self,
        sender: &UnboundedSender<MessageInBlock>,
        messages: &mut UnboundedReceiver<MessageInBlock>,
        paid_messages: &mut UnboundedReceiver<PaidMessage>,
    ) -> anyhow::Result<()> {
        loop {
            while let Some(message) = messages.try_recv().ok() {
                if let Some(msg) = self
                    .pending_messages
                    .insert(message.message.nonce_le, message)
                {
                    panic!(
                        "Received 2 messages with the same nonce: {}",
                        hex::encode(msg.message.nonce_le)
                    );
                }
            }

            while let Some(PaidMessage { nonce }) = paid_messages.try_recv().ok() {
                self.pending_nonces.push(nonce);
            }

            for i in (0..self.pending_nonces.len()).rev() {
                if let Some(message) = self.pending_messages.remove(&self.pending_nonces[i]) {
                    sender.send(message)?;
                    self.pending_nonces.remove(i);
                }
            }

            self.metrics
                .pending_messages_count
                .set(self.pending_messages.len() as i64);
        }
    }
}
