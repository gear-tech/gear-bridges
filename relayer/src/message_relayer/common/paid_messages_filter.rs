use futures::{
    future::{self, Either},
    pin_mut,
};
use gclient::ext::sp_runtime::AccountId32;

use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use super::{MessageInBlock, PaidMessage};

pub struct PaidMessagesFilter {
    pending_messages: HashMap<[u8; 32], MessageInBlock>,
    pending_nonces: Vec<[u8; 32]>,
    excluded_from_fees: HashSet<AccountId32>,

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
    pub fn new(excluded_from_fees: HashSet<AccountId32>) -> Self {
        Self {
            pending_messages: HashMap::default(),
            pending_nonces: vec![],
            excluded_from_fees,
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
                if self_
                    .excluded_from_fees
                    .contains(&AccountId32::from(message.message.source))
                {
                    log::debug!(
                        "Account {} is excluded from paying fees, automatically sending message {}",
                        AccountId32::from(message.message.source),
                        hex::encode(message.message.nonce_le)
                    );
                    sender.send(message)?;
                    continue;
                }
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

#[cfg(test)]
mod tests {

    use super::*;
    use crate::message_relayer::common::{AuthoritySetId, GearBlockNumber};
    use gear_rpc_client::dto::Message;

    #[tokio::test]
    async fn fee_payer_filter() {
        let account0 = [0; 32];
        let account1 = [1; 32];
        let mut set = HashSet::new();

        set.insert(account0.into());

        let filter = PaidMessagesFilter::new(set);

        let message0 = MessageInBlock {
            message: Message {
                destination: [0u8; 20],
                source: account0.into(),
                nonce_le: [0u8; 32],
                payload: vec![1, 2, 3],
            },
            block: GearBlockNumber(0),
            block_hash: H256::default(),
            authority_set_id: AuthoritySetId(0),
        };

        let message1 = MessageInBlock {
            message: Message {
                destination: [0u8; 20],
                source: account1.into(),
                nonce_le: [1u8; 32],
                payload: vec![4, 5, 6],
            },
            block: GearBlockNumber(0),
            block_hash: H256::default(),
            authority_set_id: AuthoritySetId(0),
        };

        let (msg_sender, msg_receiver) = unbounded_channel();
        let (paid_sender, paid_receiver) = unbounded_channel();
        let mut msg_receiver = filter.run(msg_receiver, paid_receiver).await;

        msg_sender.send(message0).unwrap();
        let res = msg_receiver.recv().await.unwrap();
        assert_eq!(res.message.nonce_le, [0u8; 32]);
        assert_eq!(res.message.source, account0);
        assert_eq!(res.message.payload, vec![1, 2, 3]);

        msg_sender.send(message1).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            msg_receiver.is_empty(),
            "Message from account1 should not be sent"
        );
        paid_sender.send(PaidMessage { nonce: [1u8; 32] }).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let res = msg_receiver.recv().await.unwrap();
        assert_eq!(res.message.nonce_le, [1u8; 32]);
        assert_eq!(res.message.source, account1);
        assert_eq!(res.message.payload, vec![4, 5, 6]);
        assert!(msg_receiver.is_empty(), "No more messages should be sent");

        drop(msg_sender);
        drop(paid_sender);
    }
}
