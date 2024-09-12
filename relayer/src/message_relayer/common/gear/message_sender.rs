use std::sync::mpsc::Receiver;

use futures::executor::block_on;
use gclient::GearApi;
use gear_core::ids::MessageId;
use prometheus::IntGauge;
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::message_relayer::common::{ERC20DepositTx, EthereumSlotNumber};

pub struct MessageSender {
    gear_api: GearApi,

    metrics: Metrics,
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        messages_waiting_checkpoint: IntGauge = IntGauge::new(
            "gear_message_sender_messages_waiting_checkpoint",
            "Amount of messages waiting for corresponding checkpoint",
        ),
        messages_waiting_finality: IntGauge = IntGauge::new(
            "gear_message_sender_messages_waiting_finality",
            "Amount of messages waiting for finality on gear",
        ),
        fee_payer_balance: IntGauge = IntGauge::new(
            "gear_message_sender_fee_payer_balance",
            "Transaction fee payer balance",
        )
    }
}

impl MessageSender {
    pub fn new(gear_api: GearApi) -> Self {
        Self {
            gear_api,

            metrics: Metrics::new(),
        }
    }

    pub fn run(
        self,
        messages: Receiver<ERC20DepositTx>,
        checkpoints: Receiver<EthereumSlotNumber>,
    ) {
        tokio::task::spawn_blocking(move || loop {
            let res = block_on(self.run_inner(&messages, &checkpoints));
            if let Err(err) = res {
                log::error!("Ethereum message sender failed: {}", err);
            }
        });
    }

    async fn run_inner(
        &self,
        messages: &Receiver<ERC20DepositTx>,
        checkpoints: &Receiver<EthereumSlotNumber>,
    ) -> anyhow::Result<()> {
        let mut waiting_checkpoint: Vec<ERC20DepositTx> = vec![];
        let mut waiting_finality: Vec<(ERC20DepositTx, MessageId)> = vec![];

        let mut latest_checkpoint_slot = None;

        loop {
            self.update_balance_metric().await?;

            for checkpoint in checkpoints.try_iter() {
                if latest_checkpoint_slot.unwrap_or_default() < checkpoint {
                    latest_checkpoint_slot = Some(checkpoint);
                } else {
                    log::error!(
                        "Received checkpoints not in sequential order. \
                        Previously found checkpoint: {:?} and new checkpoint is {}",
                        latest_checkpoint_slot,
                        checkpoint
                    );
                }
            }

            for message in messages.try_iter() {
                waiting_checkpoint.push(message);
            }

            for i in (0..waiting_checkpoint.len()).rev() {
                if waiting_checkpoint[i].slot_number <= latest_checkpoint_slot.unwrap_or_default() {
                    let message_id = self.submit_message(&waiting_checkpoint[i]).await?;

                    let message = waiting_checkpoint.remove(i);
                    waiting_finality.push((message, message_id));
                }
            }

            self.metrics
                .messages_waiting_checkpoint
                .set(waiting_checkpoint.len() as i64);

            for i in (0..waiting_finality.len()).rev() {
                // TODO: check status of tx. If it's finalized - remove from vec.
            }

            self.metrics
                .messages_waiting_finality
                .set(waiting_finality.len() as i64);
        }
    }

    async fn submit_message(&self, message: &ERC20DepositTx) -> anyhow::Result<MessageId> {
        // TODO: submit message to gear.

        todo!()
    }

    async fn update_balance_metric(&self) -> anyhow::Result<()> {
        let balance = self
            .gear_api
            .total_balance(self.gear_api.account_id())
            .await?;

        let balance = balance / 1_000_000_000_000;
        let balance: i64 = balance.try_into().unwrap_or(i64::MAX);

        self.metrics.fee_payer_balance.set(balance);

        Ok(())
    }
}
