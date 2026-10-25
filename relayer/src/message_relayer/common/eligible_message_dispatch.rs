use super::{MessageInBlock, PaidMessage};
use crate::message_relayer::gear_to_eth::storage::Storage;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Dispatch only original messages admitted by the durable queued/paid journal.
pub struct EligibleMessageDispatch {
    handoffs: HashSet<[u8; 32]>,
    sender: UnboundedSender<MessageInBlock>,
    storage: Arc<dyn Storage>,
}

impl EligibleMessageDispatch {
    pub fn new(sender: UnboundedSender<MessageInBlock>, storage: Arc<dyn Storage>) -> Self {
        Self {
            handoffs: HashSet::new(),
            sender,
            storage,
        }
    }

    async fn dispatch(&mut self) -> anyhow::Result<()> {
        // One durable snapshot decides eligibility; only channel-gap ownership lives here.
        let eligible = self.storage.eligible_messages().await?;
        let mut previous = std::mem::take(&mut self.handoffs);
        for message in eligible {
            let nonce = message.message.nonce_be;
            if !previous.remove(&nonce) {
                self.sender.send(message)?;
            }
            self.handoffs.insert(nonce);
        }
        Ok(())
    }

    pub fn spawn(
        mut self,
        mut queued: UnboundedReceiver<MessageInBlock>,
        mut paid: UnboundedReceiver<PaidMessage>,
    ) {
        tokio::spawn(async move {
            let result: anyhow::Result<()> = async {
                // Replay the durable eligible originals even without a new subscription event.
                self.dispatch().await?;
                loop {
                    tokio::select! {
                        message = queued.recv() => if message.is_none() { return Ok(()); },
                        payment = paid.recv() => if payment.is_none() { return Ok(()); },
                    }
                    // Producer payloads are wakeups, not authority for fees or source bytes.
                    self.dispatch().await?;
                }
            }
            .await;
            if let Err(error) = result {
                log::error!("Eligible message dispatch failed: {error}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::{
        common::{AuthoritySetId, GearBlockNumber},
        gear_to_eth::{
            storage::{JSONStorage, OutboundLaneIdentity},
            tx_manager::{Transaction, TransactionManager, TxStatus},
        },
    };
    use gear_rpc_client::dto::Message;
    use primitive_types::{H160, H256};
    use uuid::Uuid;

    #[tokio::test]
    async fn durable_eligibility_replays_and_preserves_channel_gap_ownership() {
        let path = std::env::temp_dir().join(format!("gear-eligible-dispatch-{}", Uuid::new_v4()));
        let storage = Arc::new(JSONStorage::new(&path));
        storage
            .bind_outbound_lane(OutboundLaneIdentity {
                destination_chain_id: 31337,
                destination_genesis_hash: H256::repeat_byte(1),
                message_queue_address: H160::repeat_byte(2),
                bridging_payment_address: Some(H256::repeat_byte(3)),
                fee_exempt_sources: [[1; 32]].into(),
                sender_address: H160::repeat_byte(4),
            })
            .await
            .unwrap();
        storage
            .bind_event_chain(H256::repeat_byte(5), 10)
            .await
            .unwrap();
        let exempt = MessageInBlock {
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
        let mut paid_message = exempt.clone();
        paid_message.message.nonce_be = [8; 32];
        paid_message.message.source = [2; 32];
        storage
            .record_queued_block(
                10,
                exempt.block_hash,
                &[exempt.clone(), paid_message.clone()],
            )
            .await
            .unwrap();
        let (sender, mut received) = tokio::sync::mpsc::unbounded_channel();
        let mut dispatch = EligibleMessageDispatch::new(sender, storage.clone());
        dispatch.dispatch().await.unwrap();
        assert_eq!(received.try_recv().unwrap(), exempt);
        assert!(
            received.is_empty(),
            "a durable non-exempt message requires its fee"
        );
        storage
            .record_paid_block(
                10,
                exempt.block_hash,
                &[paid_message.message.nonce_be, [9; 32]],
            )
            .await
            .unwrap();
        for _ in 0..32 {
            dispatch.dispatch().await.unwrap();
        }
        assert_eq!(received.try_recv().unwrap(), paid_message);
        assert!(
            received.is_empty(),
            "reconnect cannot duplicate a live channel handoff"
        );
        assert_eq!(
            dispatch.handoffs.len(),
            2,
            "unseen fees need no in-memory join entry"
        );
        let manager = TransactionManager::new(storage.clone());
        manager
            .add_transaction(Transaction::new(
                exempt.clone(),
                TxStatus::WaitForMerkleRoot,
            ))
            .await;
        manager.update_storage().await.unwrap();
        dispatch.dispatch().await.unwrap();
        assert_eq!(
            dispatch.handoffs,
            HashSet::from([paid_message.message.nonce_be])
        );
        // Crash after send but before durable UUID: a fresh channel must replay that original.
        let restarted_storage = Arc::new(JSONStorage::new(&path));
        TransactionManager::new(restarted_storage.clone())
            .load_from_storage()
            .await
            .unwrap();
        let (sender, received) = tokio::sync::mpsc::unbounded_channel();
        drop(received);
        let mut failed_handoff = EligibleMessageDispatch::new(sender, restarted_storage.clone());
        assert!(failed_handoff.dispatch().await.is_err());
        assert!(
            failed_handoff.handoffs.is_empty(),
            "failed sends cannot take channel-gap ownership"
        );
        let (sender, mut received) = tokio::sync::mpsc::unbounded_channel();
        let restarted = EligibleMessageDispatch::new(sender, restarted_storage);
        let (queued, queued_rx) = tokio::sync::mpsc::unbounded_channel();
        let (paid, paid_rx) = tokio::sync::mpsc::unbounded_channel();
        restarted.spawn(queued_rx, paid_rx);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), received.recv())
                .await
                .unwrap()
                .unwrap(),
            paid_message,
            "startup replays eligible originals without waiting for a new live notification"
        );
        assert!(
            received.is_empty(),
            "durable active UUID suppresses the exempt original"
        );
        drop((queued, paid));
        assert_eq!(
            storage.pending_event_pairs().await.unwrap()[0].message,
            paid_message
        );
        tokio::fs::remove_dir_all(path).await.unwrap();
    }
}
