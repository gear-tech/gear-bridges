use futures::{
    future::{self, Either},
    pin_mut,
};
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
                match run_inner(&mut self, &sender, &mut messages, &mut paid_messages).await {
                    Ok(_) => break,
                    Err(e) => log::error!("Paid messages filter failed: {e}"),
                }
            }
        });

        receiver
    }
}

async fn run_inner(
    self_: &mut PaidMessagesFilter,
    sender: &UnboundedSender<MessageInBlock>,
    messages: &mut UnboundedReceiver<MessageInBlock>,
    paid_messages: &mut UnboundedReceiver<PaidMessage>,
) -> anyhow::Result<()> {
    loop {
        let recv_messages = messages.recv();
        pin_mut!(recv_messages);

        let recv_paid_messages = paid_messages.recv();
        pin_mut!(recv_paid_messages);

        match future::select(recv_messages, recv_paid_messages).await {
            Either::Left((None, _)) => {
                log::info!("Channel with messages closed. Exiting");
                return Ok(());
            }

            Either::Right((None, _)) => {
                log::info!("Channel with paid messages closed. Exiting");
                return Ok(());
            }

            Either::Left((Some(message), _)) => {
                if let Some(msg) = self_
                    .pending_messages
                    .insert(message.message.nonce_le, message)
                {
                    panic!(
                        "Received 2 messages with the same nonce: {}",
                        hex::encode(msg.message.nonce_le)
                    );
                }
            }

            Either::Right((Some(PaidMessage { nonce }), _)) => self_.pending_nonces.push(nonce),
        }

        for i in (0..self_.pending_nonces.len()).rev() {
            if let Some(message) = self_.pending_messages.remove(&self_.pending_nonces[i]) {
                sender.send(message)?;
                self_.pending_nonces.remove(i);
            }
        }

        self_
            .metrics
            .pending_messages_count
            .set(self_.pending_messages.len() as i64);
    }
}
