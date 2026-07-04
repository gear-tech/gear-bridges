use crate::{
    hex_utils,
    message_relayer::common::{web_request::EthTransaction, EthereumSlotNumber, TxHashWithSlot},
};
use alloy::{network::TransactionResponse, providers::Provider};
use anyhow::Context;
use ethereum_client::PollingEthApi;
use ethereum_common::SECONDS_PER_SLOT;
use tokio::sync::mpsc::{UnboundedReceiver, WeakUnboundedSender};

/// Receives transaction relay requests from an external source (e.g. HTTP server),
/// resolves the Ethereum slot number, and forwards the transaction to the tx manager.
pub struct TransactionDataExtractor {
    eth_api: PollingEthApi,
    genesis_time: u64,
    sender: WeakUnboundedSender<TxHashWithSlot>,
    receiver: UnboundedReceiver<EthTransaction>,
}

impl TransactionDataExtractor {
    pub fn new(
        eth_api: PollingEthApi,
        genesis_time: u64,
        sender: WeakUnboundedSender<TxHashWithSlot>,
        receiver: UnboundedReceiver<EthTransaction>,
    ) -> Self {
        Self {
            eth_api,
            genesis_time,
            sender,
            receiver,
        }
    }

    pub fn spawn(self) {
        tokio::task::spawn(task(self));
    }
}

async fn task(mut this: TransactionDataExtractor) {
    loop {
        let result = run_inner(&mut this).await;
        let Err(e) = result else {
            log::trace!("Transaction data extractor exiting...");
            return;
        };

        log::error!("Transaction data extractor failed: {e}");
        if let Err(e) = this.eth_api.reconnect().await {
            log::error!(r#"Unable to reconnect: "{e}""#);
            return;
        }

        log::debug!("EthApi reconnected");
    }
}

async fn run_inner(this: &mut TransactionDataExtractor) -> anyhow::Result<()> {
    loop {
        let Some(request) = this.receiver.recv().await else {
            return Ok(());
        };

        log::trace!(r#"Processing transaction: "{request:?}""#);

        let tx_hash = hex_utils::decode_h256(&request.tx_hash)
            .context("Failed to decode transaction hash")?
            .0
            .into();

        let tx = this
            .eth_api
            .get_transaction_by_hash(tx_hash)
            .await?
            .context("Transaction not found")?;
        let block_number = tx.block_number().context("Block number is None")?;
        let block_timestamp = this.eth_api.get_block(block_number).await?.header.timestamp;
        let slot_number = EthereumSlotNumber(
            block_timestamp.saturating_sub(this.genesis_time) / SECONDS_PER_SLOT,
        );

        let Some(sender) = this.sender.upgrade() else {
            log::info!("Unable to upgrade sender channel.");
            return Ok(());
        };

        log::info!("Relaying transaction {tx_hash} from slot #{slot_number} via HTTP request",);

        sender.send(TxHashWithSlot {
            slot_number,
            tx_hash,
        })?;
    }
}
