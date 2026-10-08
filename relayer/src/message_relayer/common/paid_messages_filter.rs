use super::{MessageInBlock, PaidMessage};
use crate::message_relayer::gear_to_eth::storage::Storage;
use futures::{
    future::{self, Either},
    pin_mut,
};
use gclient::ext::sp_runtime::AccountId32;
use prometheus::IntGauge;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use utils_prometheus::{impl_metered_service, MeteredService};

pub struct PaidMessagesFilter {
    pending_messages: HashMap<[u8; 32], MessageInBlock>,
    pending_nonces: HashSet<[u8; 32]>,
    forwarded_nonces: HashSet<[u8; 32]>,
    excluded_from_fees: HashSet<AccountId32>,
    sender: UnboundedSender<MessageInBlock>,
    storage: Arc<dyn Storage>,

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
    pub fn new(
        excluded_from_fees: HashSet<AccountId32>,
        sender: UnboundedSender<MessageInBlock>,
        storage: Arc<dyn Storage>,
    ) -> Self {
        Self {
            pending_messages: HashMap::default(),
            pending_nonces: HashSet::new(),
            forwarded_nonces: HashSet::new(),
            excluded_from_fees,
            sender,
            storage,
            metrics: Metrics::new(),
        }
    }

    async fn retire_owned(&mut self) -> anyhow::Result<()> {
        let tracked: HashSet<_> = self
            .pending_messages
            .keys()
            .chain(self.pending_nonces.iter())
            .chain(self.forwarded_nonces.iter())
            .copied()
            .collect();
        for nonce in tracked {
            if self.storage.outbound_nonce_owned(nonce).await? {
                self.pending_messages.remove(&nonce);
                self.pending_nonces.remove(&nonce);
                self.forwarded_nonces.remove(&nonce);
            }
        }
        Ok(())
    }

    async fn forward_nonce(&mut self, nonce: [u8; 32]) -> anyhow::Result<()> {
        self.retire_owned().await?;
        if self.forwarded_nonces.contains(&nonce)
            || self.storage.outbound_nonce_owned(nonce).await?
        {
            self.pending_messages.remove(&nonce);
            self.pending_nonces.remove(&nonce);
            return Ok(());
        }
        let Some(message) = self.pending_messages.get(&nonce) else {
            return Ok(());
        };
        if !self.pending_nonces.contains(&nonce)
            && !self
                .excluded_from_fees
                .contains(&AccountId32::from(message.message.source))
        {
            return Ok(());
        }
        if !self.storage.message_is_eligible(message).await? {
            return Ok(());
        }
        self.sender.send(
            self.pending_messages
                .remove(&nonce)
                .expect("pending message was checked"),
        )?;
        self.pending_nonces.remove(&nonce);
        // Keep only the channel handoff gap; durable active/held/failed/completed ownership retires it.
        self.forwarded_nonces.insert(nonce);
        Ok(())
    }

    async fn receive_message(&mut self, message: MessageInBlock) -> anyhow::Result<()> {
        let nonce = message.message.nonce_be;
        if let Some(original) = self.pending_messages.get(&nonce) {
            anyhow::ensure!(
                original == &message,
                "HOLD: conflicting filtered outbound nonce"
            );
        } else {
            self.pending_messages.insert(nonce, message);
        }
        self.forward_nonce(nonce).await
    }

    async fn receive_paid(&mut self, nonce: [u8; 32]) -> anyhow::Result<()> {
        self.pending_nonces.insert(nonce);
        self.forward_nonce(nonce).await
    }

    pub fn spawn(
        mut self,
        mut messages: UnboundedReceiver<MessageInBlock>,
        mut paid_messages: UnboundedReceiver<PaidMessage>,
    ) {
        tokio::spawn(async move {
            match run_inner(&mut self, &mut messages, &mut paid_messages).await {
                Ok(_) => {}
                Err(e) => log::error!("Paid messages filter failed: {e}"),
            }
        });
    }
}

async fn run_inner(
    self_: &mut PaidMessagesFilter,
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

            Either::Left((Some(message), _)) => self_.receive_message(message).await?,
            Either::Right((Some(PaidMessage { nonce }), _)) => self_.receive_paid(nonce).await?,
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
    use primitive_types::H256;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn reconnect_replays_are_bounded_and_retire_only_durable_owned_nonces() {
        use crate::message_relayer::gear_to_eth::{
            storage::{JSONStorage, OutboundLaneIdentity},
            tx_manager::{Transaction, TransactionManager, TxStatus},
        };
        use primitive_types::H160;
        use uuid::Uuid;
        let path = std::env::temp_dir().join(format!("gear-filter-reconnect-{}", Uuid::new_v4()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_outbound_lane(OutboundLaneIdentity {
                destination_chain_id: 560048,
                destination_genesis_hash: H256::repeat_byte(1),
                message_queue_address: H160::repeat_byte(2),
                bridging_payment_address: Some(H256::repeat_byte(3)),
                fee_exempt_sources: Default::default(),
                sender_address: H160::repeat_byte(4),
            })
            .await
            .unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(5), 10)
            .await
            .unwrap();
        let message = MessageInBlock {
            message: Message {
                nonce_be: [7; 32],
                source: [1; 32],
                destination: [2; 20],
                payload: vec![3],
            },
            block: GearBlockNumber(10),
            block_hash: H256::repeat_byte(10),
            authority_set_id: AuthoritySetId(1),
        };
        let unseen = [9; 32];
        storage
            .record_queued_block(10, message.block_hash, &[message.clone()])
            .await
            .unwrap();
        storage
            .record_paid_block(10, message.block_hash, &[message.message.nonce_be, unseen])
            .await
            .unwrap();
        let first_fees = storage.paid_observations().await.unwrap();
        let (sender, mut received) = mpsc::unbounded_channel();
        let mut filter = PaidMessagesFilter::new(HashSet::new(), sender, storage.clone());
        for _ in 0..32 {
            filter.receive_paid(message.message.nonce_be).await.unwrap();
            filter.receive_paid(unseen).await.unwrap();
        }
        assert_eq!(
            filter.pending_nonces.len(),
            2,
            "fee-only replay must be keyed by nonce"
        );
        filter.receive_message(message.clone()).await.unwrap();
        assert_eq!(received.try_recv().unwrap(), message);
        for _ in 0..32 {
            filter.receive_message(message.clone()).await.unwrap();
            filter.receive_paid(message.message.nonce_be).await.unwrap();
            filter.receive_paid(unseen).await.unwrap();
        }
        assert!(
            received.is_empty(),
            "an unacknowledged channel handoff cannot be sent again"
        );
        assert!(filter.pending_messages.is_empty());
        assert_eq!(filter.pending_nonces, HashSet::from([unseen]));
        let manager = TransactionManager::new(storage.clone());
        let tx = Transaction::new(message.clone(), TxStatus::WaitForMerkleRoot);
        let uuid = tx.uuid;
        manager.add_transaction(tx).await;
        manager.update_storage().await.unwrap();
        let original = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
        filter.receive_paid(message.message.nonce_be).await.unwrap();
        assert!(
            filter.forwarded_nonces.is_empty(),
            "the durable UUID retires channel bookkeeping"
        );
        assert_eq!(filter.pending_nonces, HashSet::from([unseen]));
        assert_eq!(storage.paid_observations().await.unwrap(), first_fees);
        assert_eq!(
            tokio::fs::read(path.join(uuid.to_string())).await.unwrap(),
            original
        );
        assert_eq!(
            storage.pending_event_pairs().await.unwrap()[0].message,
            message
        );
        assert!(received.is_empty());
        tokio::fs::remove_dir_all(path).await.unwrap();
    }

    #[tokio::test]
    async fn fee_payer_filter() {
        let account0 = [0; 32];
        let account1 = [1; 32];
        let mut set = HashSet::new();

        set.insert(account0.into());

        let (filter_msg_sender, mut msg_receiver) = mpsc::unbounded_channel();
        let filter = PaidMessagesFilter::new(
            set,
            filter_msg_sender,
            Arc::new(crate::message_relayer::gear_to_eth::storage::NoStorage::new()),
        );

        let message0 = MessageInBlock {
            message: Message {
                destination: [0u8; 20],
                source: account0,
                nonce_be: [0u8; 32],
                payload: vec![1, 2, 3],
            },
            block: GearBlockNumber(0),
            block_hash: H256::default(),
            authority_set_id: AuthoritySetId(0),
        };

        let message1 = MessageInBlock {
            message: Message {
                destination: [0u8; 20],
                source: account1,
                nonce_be: [1u8; 32],
                payload: vec![4, 5, 6],
            },
            block: GearBlockNumber(0),
            block_hash: H256::default(),
            authority_set_id: AuthoritySetId(0),
        };

        let (msg_sender, filter_msg_receiver) = mpsc::unbounded_channel();
        let (paid_sender, paid_receiver) = mpsc::unbounded_channel();
        filter.spawn(filter_msg_receiver, paid_receiver);

        msg_sender.send(message0).unwrap();
        let res = msg_receiver.recv().await.unwrap();
        assert_eq!(res.message.nonce_be, [0u8; 32]);
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
        assert_eq!(res.message.nonce_be, [1u8; 32]);
        assert_eq!(res.message.source, account1);
        assert_eq!(res.message.payload, vec![4, 5, 6]);
        assert!(msg_receiver.is_empty(), "No more messages should be sent");

        drop(msg_sender);
        drop(paid_sender);
    }
}
