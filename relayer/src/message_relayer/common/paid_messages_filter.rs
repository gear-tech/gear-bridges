use std::{
    collections::HashMap,
    sync::mpsc::{channel, Receiver, Sender},
};

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
        pending_messages_count: IntGauge
    }
}

impl Metrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            pending_messages_count: IntGauge::new(
                "paid_messages_filter_pending_messages_count",
                "Amount of discovered but not paid messages",
            )?,
        })
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

    pub fn run(
        mut self,
        messages: Receiver<MessageInBlock>,
        paid_messages: Receiver<PaidMessage>,
    ) -> Receiver<MessageInBlock> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &messages, &paid_messages);
                if let Err(err) = res {
                    log::error!("Paid messages filter failed: {}", err);
                }
            }
        });

        receiver
    }

    fn run_inner(
        &mut self,
        sender: &Sender<MessageInBlock>,
        messages: &Receiver<MessageInBlock>,
        paid_messages: &Receiver<PaidMessage>,
    ) -> anyhow::Result<()> {
        loop {
            for message in messages.try_iter() {
                self.pending_messages
                    .insert(message.message.nonce_le, message)
                    .expect("Received 2 messages with the same nonce");
            }

            for PaidMessage { nonce } in paid_messages.try_iter() {
                self.pending_nonces.push(nonce);
            }

            for i in (0..self.pending_nonces.len()).rev() {
                if let Some(message) = self.pending_messages.remove(&self.pending_nonces[i]) {
                    sender.send(message)?;
                    self.pending_nonces.remove(i);
                }
            }

            if !self.pending_nonces.is_empty() {
                log::warn!("Discovered message that was paid but it's contents haven't discovered");
            }

            self.metrics
                .pending_messages_count
                .set(self.pending_messages.len() as i64);
        }
    }
}
