use bridging_payment_client::bridging_payment::events::BridgingPaymentEvents;
use primitive_types::H256;
use prometheus::IntCounter;
use sails_rs::events::EventIo;
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::{
    broadcast::{error::RecvError, Receiver},
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::{
    common::{
        gear::message_queued_event_extractor::{
            fetch_finalized_gear_block, finalized_replay_range, latest_finalized_block_number,
        },
        GearBlock, PaidMessage,
    },
    gear_to_eth::storage::{GearEventStream, Storage},
};
use gear_common::api_provider::ApiProviderConnection;

pub struct MessagePaidEventExtractor {
    api_provider: ApiProviderConnection,
    bridging_payment_address: H256,
    storage: Arc<dyn Storage>,
    forwarded_nonces: HashSet<[u8; 32]>,
    metrics: Metrics,
}

impl MeteredService for MessagePaidEventExtractor {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        total_messages_found: IntCounter = IntCounter::new(
            "message_paid_event_extractor_total_messages_found",
            "Total amount of paid messages discovered",
        ),
    }
}

impl MessagePaidEventExtractor {
    pub fn new(
        api_provider: ApiProviderConnection,
        bridging_payment_address: H256,
        storage: Arc<dyn Storage>,
    ) -> Self {
        Self {
            api_provider,
            bridging_payment_address,
            storage,
            forwarded_nonces: HashSet::new(),
            metrics: Metrics::new(),
        }
    }

    pub async fn run(self, mut blocks: Receiver<GearBlock>) -> UnboundedReceiver<PaidMessage> {
        let (sender, receiver) = unbounded_channel();
        tokio::task::spawn(async move {
            let mut this = self;
            loop {
                match this.run_inner(&sender, &mut blocks).await {
                    Ok(()) => return,
                    Err(err) => {
                        log::error!("Message paid event extractor failed: {err}");
                        if blocks.is_closed() || sender.is_closed() {
                            return;
                        }
                        loop {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            match this.api_provider.reconnect().await {
                                Ok(()) => break,
                                Err(err) => log::error!(
                                    "Message paid event extractor unable to reconnect: {err}; retrying"
                                ),
                            }
                        }
                    }
                }
            }
        });
        receiver
    }

    async fn run_inner(
        &mut self,
        sender: &UnboundedSender<PaidMessage>,
        blocks: &mut Receiver<GearBlock>,
    ) -> anyhow::Result<()> {
        self.replay_pending(sender).await?;
        self.catch_up_to_finalized(sender).await?;
        loop {
            match blocks.recv().await {
                Ok(block) => self.process_subscribed_block(block, sender).await?,
                Err(RecvError::Closed) => {
                    log::warn!("Message paid event extractor channel closed, exiting");
                    return Ok(());
                }
                Err(RecvError::Lagged(skipped)) => {
                    log::warn!("Message paid event extractor lagged by {skipped} blocks; backfilling finalized range");
                    self.catch_up_to_finalized(sender).await?;
                }
            }
        }
    }

    async fn replay_pending(
        &mut self,
        sender: &UnboundedSender<PaidMessage>,
    ) -> anyhow::Result<()> {
        let count =
            forward_paid_observations(self.storage.as_ref(), sender, &mut self.forwarded_nonces)
                .await?;
        self.metrics.total_messages_found.inc_by(count);
        Ok(())
    }

    async fn catch_up_to_finalized(
        &mut self,
        sender: &UnboundedSender<PaidMessage>,
    ) -> anyhow::Result<()> {
        let latest = latest_finalized_block_number(&mut self.api_provider).await?;
        self.catch_up_through(latest, sender).await
    }

    async fn catch_up_through(
        &mut self,
        finalized_block: u32,
        sender: &UnboundedSender<PaidMessage>,
    ) -> anyhow::Result<()> {
        let start =
            self.storage.event_start_block().await?.ok_or_else(|| {
                anyhow::anyhow!("Gear event storage has not been bound to a chain")
            })?;
        let from = self
            .storage
            .replay_from_block(start, GearEventStream::Paid)
            .await?;
        let Some(range) = finalized_replay_range(from, finalized_block) else {
            return Ok(());
        };
        for block_number in range {
            let cursor = self.storage.event_cursor(GearEventStream::Paid).await?;
            if let Some(cursor) = cursor {
                if block_number < cursor.block {
                    continue;
                }
                if block_number == cursor.block {
                    let block =
                        fetch_finalized_gear_block(&mut self.api_provider, block_number).await?;
                    if block.hash() != cursor.hash {
                        anyhow::bail!("Finalized paid-event block #{block_number} changed hash");
                    }
                    continue;
                }
            }
            let block = fetch_finalized_gear_block(&mut self.api_provider, block_number).await?;
            self.process_block_events(block, sender).await?;
        }
        Ok(())
    }

    async fn process_subscribed_block(
        &mut self,
        block: GearBlock,
        sender: &UnboundedSender<PaidMessage>,
    ) -> anyhow::Result<()> {
        let block_number = block.number();
        let block_hash = block.hash();
        let start =
            self.storage.event_start_block().await?.ok_or_else(|| {
                anyhow::anyhow!("Gear event storage has not been bound to a chain")
            })?;
        let cursor = self.storage.event_cursor(GearEventStream::Paid).await?;
        let next = match cursor {
            Some(cursor) if block_number <= cursor.block => {
                if block_number == cursor.block && block_hash != cursor.hash {
                    anyhow::bail!("Finalized paid-event block #{block_number} changed hash");
                }
                return Ok(());
            }
            Some(cursor) => cursor.block.checked_add(1),
            None => Some(start),
        };
        let Some(next) = next else {
            return Ok(());
        };
        if block_number < next {
            return Ok(());
        }
        if block_number > next {
            self.catch_up_through(block_number - 1, sender).await?;
        }

        let next = self
            .storage
            .event_cursor(GearEventStream::Paid)
            .await?
            .map(|cursor| cursor.block.saturating_add(1))
            .unwrap_or(start);
        if block_number != next {
            anyhow::bail!(
                "Paid-event cursor is at #{}, cannot advance to #{block_number}",
                next.saturating_sub(1)
            );
        }
        self.process_block_events(block, sender).await
    }

    async fn process_block_events(
        &mut self,
        block: GearBlock,
        sender: &UnboundedSender<PaidMessage>,
    ) -> anyhow::Result<()> {
        let block_hash = block.hash();
        let nonces = record_paid_events(
            self.storage.as_ref(),
            block.number(),
            block_hash,
            block.user_message_sent_events(self.bridging_payment_address, H256::zero()),
        )
        .await?;
        forward_paid_observations(self.storage.as_ref(), sender, &mut self.forwarded_nonces)
            .await?;

        let total = nonces.len() as u64;
        if total > 0 {
            log::info!(
                "Found {total} paid messages in block #{} ({block_hash})",
                block.number()
            );
            self.metrics.total_messages_found.inc_by(total);
        }
        Ok(())
    }
}

async fn forward_paid_observations(
    storage: &dyn Storage,
    sender: &UnboundedSender<PaidMessage>,
    forwarded: &mut HashSet<[u8; 32]>,
) -> anyhow::Result<u64> {
    let mut pending = HashSet::new();
    for observation in storage.paid_observations().await? {
        if !storage.outbound_nonce_owned(observation.nonce).await? {
            pending.insert(observation.nonce);
        }
    }
    // This cache belongs to the live channel, not to completed or handed-off UUIDs.
    forwarded.retain(|nonce| pending.contains(nonce));
    let mut count = 0;
    for nonce in pending {
        if !forwarded.contains(&nonce) {
            sender.send(PaidMessage { nonce })?;
            forwarded.insert(nonce);
            count += 1;
        }
    }
    Ok(count)
}

async fn record_paid_events<'a>(
    storage: &dyn Storage,
    block: u32,
    block_hash: H256,
    events: impl Iterator<Item = &'a [u8]>,
) -> anyhow::Result<Vec<[u8; 32]>> {
    let nonces = events
        .filter_map(|event| {
            BridgingPaymentEvents::decode_event(event)
                .ok()
                .map(|event| match event {
                    BridgingPaymentEvents::BridgingPaid { nonce }
                    | BridgingPaymentEvents::PriorityBridgingPaid { nonce, .. } => nonce,
                })
        })
        .map(|nonce| {
            let mut bytes = [0; 32];
            nonce.to_big_endian(&mut bytes);
            bytes
        })
        .collect::<Vec<_>>();
    storage.record_paid_block(block, block_hash, &nonces).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::{
        common::{AuthoritySetId, GearBlockNumber, MessageInBlock},
        gear_to_eth::storage::{
            tests::{completed_tx, lane_identity},
            GearEventCursor, JSONStorage, OutboundLaneIdentity, PaidObservation,
        },
    };
    use parity_scale_codec::Encode;
    use primitive_types::{H160, U256};
    use uuid::Uuid;

    fn fee_event(nonce: U256, priority: bool) -> Vec<u8> {
        let nonce = nonce.to_little_endian();
        if priority {
            [
                BridgingPaymentEvents::ROUTE,
                &"PriorityBridgingPaid".encode(),
                &[0x42u8; 32],
                &nonce,
            ]
            .concat()
        } else {
            [
                BridgingPaymentEvents::ROUTE,
                &"BridgingPaid".encode(),
                &nonce,
            ]
            .concat()
        }
    }

    #[tokio::test]
    async fn independent_paid_extractor_retries_do_not_resend_or_keep_owned_bookkeeping() {
        use crate::message_relayer::gear_to_eth::tx_manager::{
            Transaction, TransactionManager, TxStatus,
        };
        for completed in [false, true] {
            let path = std::env::temp_dir().join(format!("gear-paid-retry-{}", Uuid::new_v4()));
            let storage = Arc::new(JSONStorage::new(&path));
            storage.bind_outbound_lane(lane_identity()).await.unwrap();
            storage
                .bind_event_chain(H256::repeat_byte(5), 10)
                .await
                .unwrap();
            let nonce = U256::from(7u64);
            let unseen = U256::from(999u64);
            let message = MessageInBlock {
                message: gear_rpc_client::dto::Message {
                    nonce_be: nonce.to_big_endian(),
                    source: [1; 32],
                    destination: [2; 20],
                    payload: vec![3],
                },
                block: GearBlockNumber(10),
                block_hash: H256::repeat_byte(10),
                authority_set_id: AuthoritySetId(1),
            };
            storage
                .record_queued_block(10, message.block_hash, std::slice::from_ref(&message))
                .await
                .unwrap();
            let first = fee_event(nonce, false);
            let paid_first = fee_event(unseen, true);
            record_paid_events(
                storage.as_ref(),
                10,
                message.block_hash,
                [first.as_slice(), paid_first.as_slice()].into_iter(),
            )
            .await
            .unwrap();
            let (sender, mut received) = unbounded_channel();
            let mut forwarded = HashSet::new();
            assert_eq!(
                forward_paid_observations(storage.as_ref(), &sender, &mut forwarded)
                    .await
                    .unwrap(),
                2
            );
            let delivered: HashSet<_> = [
                received.try_recv().unwrap().nonce,
                received.try_recv().unwrap().nonce,
            ]
            .into_iter()
            .collect();
            assert_eq!(
                delivered,
                HashSet::from([nonce.to_big_endian(), unseen.to_big_endian()])
            );
            for _ in 0..32 {
                assert_eq!(
                    forward_paid_observations(storage.as_ref(), &sender, &mut forwarded)
                        .await
                        .unwrap(),
                    0
                );
            }
            let manager = TransactionManager::new(storage.clone());
            let tx = if completed {
                completed_tx(message.clone()).await
            } else {
                Transaction::new(message.clone(), TxStatus::WaitForMerkleRoot)
            };
            let uuid = tx.uuid;
            manager.add_transaction(tx).await;
            manager.update_storage().await.unwrap();
            if completed {
                storage
                    .ack_event_pair(message.message.nonce_be)
                    .await
                    .unwrap();
            }
            let original = tokio::fs::read(path.join(uuid.to_string())).await.unwrap();
            let original_events = tokio::fs::read(path.join("gear_events.json"))
                .await
                .unwrap();
            for _ in 0..32 {
                assert_eq!(
                    forward_paid_observations(storage.as_ref(), &sender, &mut forwarded)
                        .await
                        .unwrap(),
                    0
                );
            }
            assert_eq!(
                forwarded,
                HashSet::from([unseen.to_big_endian()]),
                "only the genuinely unseen paid-first nonce belongs to this channel cache"
            );
            assert!(received.is_empty());
            assert_eq!(
                tokio::fs::read(path.join(uuid.to_string())).await.unwrap(),
                original
            );
            assert_eq!(
                tokio::fs::read(path.join("gear_events.json"))
                    .await
                    .unwrap(),
                original_events
            );
            let remaining = storage.paid_observations().await.unwrap();
            assert!(remaining.contains(&PaidObservation {
                nonce: unseen.to_big_endian(),
                block: 10,
                block_hash: message.block_hash
            }));
            if !completed {
                assert!(remaining.contains(&PaidObservation {
                    nonce: message.message.nonce_be,
                    block: 10,
                    block_hash: message.block_hash
                }));
            }
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn completed_nonce_later_normal_and_priority_fees_do_not_recreate_paid_backlog() {
        use crate::message_relayer::gear_to_eth::tx_manager::TransactionManager;
        use std::sync::Arc;

        for (first_priority, duplicate_priority) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let path = std::env::temp_dir()
                .join(format!("gear-completed-duplicate-fees-{}", Uuid::new_v4()));
            let storage = Arc::new(JSONStorage::new(&path));
            storage.bind_outbound_lane(lane_identity()).await.unwrap();
            storage
                .bind_event_chain(H256::repeat_byte(5), 10)
                .await
                .unwrap();
            let nonce = U256::from(7u64);
            let first_hash = H256::repeat_byte(10);
            let queued = MessageInBlock {
                message: gear_rpc_client::dto::Message {
                    nonce_be: nonce.to_big_endian(),
                    source: [1; 32],
                    destination: [2; 20],
                    payload: vec![3],
                },
                block: GearBlockNumber(10),
                block_hash: first_hash,
                authority_set_id: AuthoritySetId(1),
            };
            storage
                .record_queued_block(10, first_hash, std::slice::from_ref(&queued))
                .await
                .unwrap();
            let initial = fee_event(nonce, first_priority);
            record_paid_events(
                storage.as_ref(),
                10,
                first_hash,
                [initial.as_slice()].into_iter(),
            )
            .await
            .unwrap();
            let manager = TransactionManager::new(storage.clone());
            let tx = completed_tx(queued.clone()).await;
            let uuid = tx.uuid;
            manager.add_transaction(tx).await;
            manager.update_storage().await.unwrap();
            storage.ack_event_pair(nonce.to_big_endian()).await.unwrap();

            let restored = Arc::new(JSONStorage::new(&path));
            let recovered = TransactionManager::new(restored.clone());
            recovered.load_from_storage().await.unwrap();
            assert_eq!(recovered.completed.read().await[&uuid].message, queued);
            let late_fee = fee_event(nonce, duplicate_priority);
            let unseen = U256::from(999u64);
            let unseen_fee = fee_event(unseen, false);
            let second_hash = H256::repeat_byte(11);
            let discovered = record_paid_events(
                restored.as_ref(),
                11,
                second_hash,
                [late_fee.as_slice(), unseen_fee.as_slice()].into_iter(),
            )
            .await
            .unwrap();
            assert_eq!(discovered, vec![unseen.to_big_endian()]);
            assert_eq!(
                restored.paid_observations().await.unwrap(),
                vec![PaidObservation {
                    nonce: unseen.to_big_endian(),
                    block: 11,
                    block_hash: second_hash,
                }],
                "a later fee for a completed nonce must not recreate paid-only backlog"
            );
            assert!(restored.pending_event_pairs().await.unwrap().is_empty());
            assert_eq!(
                restored.event_cursor(GearEventStream::Paid).await.unwrap(),
                Some(GearEventCursor {
                    block: 11,
                    hash: second_hash
                })
            );
            let restarted = Arc::new(JSONStorage::new(&path));
            let restarted_manager = TransactionManager::new(restarted.clone());
            restarted_manager.load_from_storage().await.unwrap();
            let other_late_fee = fee_event(nonce, !duplicate_priority);
            let unseen_repeat = fee_event(unseen, true);
            assert!(record_paid_events(
                restarted.as_ref(),
                12,
                H256::repeat_byte(12),
                [other_late_fee.as_slice(), unseen_repeat.as_slice()].into_iter()
            )
            .await
            .unwrap()
            .is_empty());
            assert_eq!(
                restarted.paid_observations().await.unwrap(),
                vec![PaidObservation {
                    nonce: unseen.to_big_endian(),
                    block: 11,
                    block_hash: second_hash,
                }]
            );
            assert_eq!(
                restarted_manager.completed.read().await[&uuid].message,
                queued
            );
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }

    #[tokio::test]
    async fn canonical_duplicate_fees_keep_first_provenance_and_do_not_wedge_unpaired_nonces() {
        for (first_priority, repeated_priority) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let path = std::env::temp_dir().join(format!("gear-duplicate-fees-{}", Uuid::new_v4()));
            let storage = JSONStorage::new(&path);
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
            let unpaired = U256::from(999u64);
            let later = U256::from(7u64);
            let first_hash = H256::repeat_byte(10);
            let second_hash = H256::repeat_byte(11);
            let third_hash = H256::repeat_byte(12);
            let initial = fee_event(unpaired, first_priority);
            record_paid_events(&storage, 10, first_hash, [initial.as_slice()].into_iter())
                .await
                .unwrap();
            let queued = MessageInBlock {
                message: gear_rpc_client::dto::Message {
                    nonce_be: later.to_big_endian(),
                    source: [1; 32],
                    destination: [2; 20],
                    payload: vec![3],
                },
                block: GearBlockNumber(10),
                block_hash: first_hash,
                authority_set_id: AuthoritySetId(1),
            };
            storage
                .record_queued_block(10, first_hash, std::slice::from_ref(&queued))
                .await
                .unwrap();
            assert!(storage.pending_event_pairs().await.unwrap().is_empty());
            let duplicate = fee_event(unpaired, repeated_priority);
            let distinct = fee_event(later, false);
            let discovered = record_paid_events(
                &storage,
                11,
                second_hash,
                [duplicate.as_slice(), distinct.as_slice()].into_iter(),
            )
            .await
            .unwrap();
            assert_eq!(discovered, vec![later.to_big_endian()]);
            let escalate = fee_event(later, true);
            record_paid_events(&storage, 12, third_hash, [escalate.as_slice()].into_iter())
                .await
                .unwrap();
            let restored = JSONStorage::new(&path);
            let expected = [
                PaidObservation {
                    nonce: later.to_big_endian(),
                    block: 11,
                    block_hash: second_hash,
                },
                PaidObservation {
                    nonce: unpaired.to_big_endian(),
                    block: 10,
                    block_hash: first_hash,
                },
            ];
            assert_eq!(restored.paid_observations().await.unwrap(), expected);
            let pairs = restored.pending_event_pairs().await.unwrap();
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].message, queued);
            assert_eq!(pairs[0].paid_block, 11);
            assert_eq!(pairs[0].paid_block_hash, second_hash);
            assert_eq!(
                restored
                    .replay_from_block(10, GearEventStream::Paid)
                    .await
                    .unwrap(),
                13
            );
            for (block, hash) in [
                (12, H256::repeat_byte(99)),
                (14, H256::repeat_byte(14)),
                (11, second_hash),
            ] {
                assert!(record_paid_events(
                    &restored,
                    block,
                    hash,
                    [duplicate.as_slice()].into_iter()
                )
                .await
                .is_err());
            }
            assert_eq!(
                restored.event_cursor(GearEventStream::Paid).await.unwrap(),
                Some(GearEventCursor {
                    block: 12,
                    hash: third_hash
                })
            );
            assert_eq!(restored.paid_observations().await.unwrap(), expected);
            tokio::fs::remove_dir_all(path).await.unwrap();
        }
    }
}
