use crate::message_relayer::common::RelayedMerkleRoot;
use crate::{
    common::{self, BASE_RETRY_DELAY, MAX_RETRIES},
    message_relayer::{
        common::{GearBlockNumber, MessageInBlock},
        eth_to_gear::api_provider::ApiProviderConnection,
    },
};
use ethereum_client::{Error, EthApi, TxHash, TxStatus};
use futures::{
    future::{self, Either},
    pin_mut,
};
use gear_rpc_client::{dto::Message, GearApi};
use keccak_hash::keccak_256;
use primitive_types::H256;
use prometheus::{Gauge, IntCounter, IntGauge};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::{self, Duration},
};
use utils_prometheus::{impl_metered_service, MeteredService};

type Status = (TxHash, Result<TxStatus, Error>);

pub struct MessageSender {
    eth_api: EthApi,
    api_provider: ApiProviderConnection,

    metrics: Metrics,
}

impl MeteredService for MessageSender {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl_metered_service! {
    struct Metrics {
        pending_tx_count: IntGauge = IntGauge::new(
            "ethereum_message_sender_pending_tx_count",
            "Amount of txs pending finalization on ethereum",
        ),
        fee_payer_balance: Gauge = Gauge::new(
            "ethereum_message_sender_fee_payer_balance",
            "Transaction fee payer balance",
        ),
        total_failed_txs: IntCounter = IntCounter::new(
            "ethereum_message_sender_total_failed_txs",
            "Total amount of txs sent to ethereum and failed",
        ),
    }
}

impl MessageSender {
    pub fn new(eth_api: EthApi, api_provider: ApiProviderConnection) -> Self {
        Self {
            eth_api,
            api_provider,

            metrics: Metrics::new(),
        }
    }

    pub async fn run(
        mut self,
        mut messages: UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)>,
    ) {
        tokio::task::spawn(async move {
            let mut attempts = 0;

            let (tx_sender, mut tx_receiver) = mpsc::unbounded_channel();
            loop {
                match run_inner(&mut self, &mut messages, &mut tx_receiver, &tx_sender).await {
                    Ok(_) => break,
                    Err(e) => {
                        attempts += 1;
                        let delay = BASE_RETRY_DELAY * 2u32.pow(attempts - 1);
                        log::error!(
                        "Ethereum message sender failed (attempt: {attempts}/{MAX_RETRIES}): {e}. Retrying in {delay:?}",
                    );
                        if attempts >= MAX_RETRIES {
                            log::error!("Maximum attempts reached, exiting...");
                            break;
                        }

                        tokio::time::sleep(delay).await;

                        match self.api_provider.reconnect().await {
                            Ok(()) => {
                                log::info!("Ethereum message sender reconnected");
                            }

                            Err(err) => {
                                log::error!("Ethereum message sender unable to reconnect: {err}");
                                return;
                            }
                        }

                        if common::is_transport_error_recoverable(&e) {
                            match self.eth_api.reconnect().inspect_err(|e| {
                                log::error!("Failed to reconnect to Ethereum: {e}");
                            }) {
                                Ok(eth_api) => self.eth_api = eth_api,
                                Err(_) => {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

async fn run_inner(
    this: &mut MessageSender,
    messages: &mut UnboundedReceiver<(MessageInBlock, RelayedMerkleRoot)>,
    tx_receiver: &mut UnboundedReceiver<Status>,
    tx_sender: &UnboundedSender<Status>,
) -> anyhow::Result<()> {
    let gear_api = this.api_provider.client();
    loop {
        let fee_payer_balance = this.eth_api.get_approx_balance().await?;
        this.metrics.fee_payer_balance.set(fee_payer_balance);

        let recv_messages = messages.recv();
        pin_mut!(recv_messages);

        let recv_tx_statuses = tx_receiver.recv();
        pin_mut!(recv_tx_statuses);

        match future::select(recv_messages, recv_tx_statuses).await {
            Either::Left((None, _)) => {
                log::info!("Channel with messages closed. Exiting");
                return Ok(());
            }

            Either::Right((None, _)) => {
                log::info!("Channel with transactions statuses closed. Exiting");
                return Ok(());
            }

            Either::Left((Some((message, merkle_root)), _)) => {
                let tx_hash = submit_message(
                    &gear_api,
                    &this.eth_api,
                    &message.message,
                    merkle_root.block,
                    merkle_root.block_hash,
                )
                .await?;
                this.metrics.pending_tx_count.inc();

                tokio::spawn(get_tx_status(
                    this.eth_api.clone(),
                    tx_hash,
                    tx_sender.clone(),
                ));
            }

            Either::Right((Some(status), _)) => {
                check_tx_status(this, status);
            }
        }
    }
}

fn check_tx_status(this: &mut MessageSender, status: Status) {
    let (tx_hash, status) = status;
    match status {
        Ok(TxStatus::Pending) => {
            log::error!("Transaction {tx_hash} is still pending");
        }

        Ok(TxStatus::Finalized) => {
            this.metrics.pending_tx_count.dec();

            log::info!("Transaction {tx_hash} has been finalized");
        }

        Ok(TxStatus::Failed) => {
            this.metrics.total_failed_txs.inc();

            log::error!("Failed to finalize transaction {tx_hash}");
        }

        Err(e) => {
            log::warn!("Unable to get status of the transaction {tx_hash:?}: {e:?}")
        }
    }
}

async fn get_tx_status(eth_api: EthApi, tx_hash: TxHash, tx_sender: UnboundedSender<Status>) {
    // wait for 18 minutes for the first time and for 5 minutes in the next three attempts
    let mut iter = [18, 5, 5, 5].iter().peekable();
    while let Some(minutes) = iter.next() {
        time::sleep(Duration::from_secs(minutes * 60)).await;

        let status = eth_api.get_tx_status(tx_hash).await;
        match status {
            Ok(TxStatus::Pending) if iter.peek().is_some() => {}

            status => {
                let result = tx_sender.send((tx_hash, status));
                if result.is_err() {
                    log::error!("Failed to notify about transaction status: tx_hash = {tx_hash}, error = {result:?}");
                }

                break;
            }
        }
    }
}

async fn submit_message(
    gear_api: &GearApi,
    eth_api: &EthApi,
    message: &Message,
    merkle_root_block: GearBlockNumber,
    merkle_root_block_hash: H256,
) -> anyhow::Result<TxHash> {
    let message_hash = message_hash(message);

    log::info!(
        "Relaying message with hash {} and nonce {}",
        hex::encode(message_hash),
        hex::encode(message.nonce_le)
    );

    let proof = gear_api
        .fetch_message_inclusion_merkle_proof(merkle_root_block_hash, message_hash.into())
        .await?;

    let tx_hash = eth_api
        .provide_content_message(
            merkle_root_block.0,
            proof.num_leaves as u32,
            proof.leaf_index as u32,
            message.nonce_le,
            message.source,
            message.destination,
            message.payload.to_vec(),
            proof.proof,
        )
        .await?;

    log::info!(
        "Message with nonce {} relaying started: tx_hash = {tx_hash}",
        hex::encode(message.nonce_le)
    );

    Ok(tx_hash)
}

fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_le.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_256(&data, &mut hash);

    hash
}
