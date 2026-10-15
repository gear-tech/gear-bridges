use crate::{
    hex_utils,
    message_relayer::common::{web_request::EthTransaction, EthereumSlotNumber, TxHashWithSlot},
};
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
    minimum_receipt_slot: u64,
    last_receipt_slot: Option<u64>,
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
            minimum_receipt_slot: 0,
            last_receipt_slot: None,
        }
    }

    pub fn with_receipt_domain(mut self, minimum: u64, last: Option<u64>) -> Self {
        self.minimum_receipt_slot = minimum;
        self.last_receipt_slot = last;
        self
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

        let block_number = this
            .eth_api
            .get_transaction_block_number(tx_hash)
            .await?
            .context("Transaction not found")?;
        let block_timestamp = this.eth_api.get_block(block_number).await?.header.timestamp;
        let elapsed = block_timestamp
            .checked_sub(this.genesis_time)
            .context("HOLD: Ethereum timestamp precedes Beacon genesis")?;
        anyhow::ensure!(
            elapsed % SECONDS_PER_SLOT == 0,
            "HOLD: Ethereum timestamp does not identify a Beacon slot"
        );
        let slot_number = EthereumSlotNumber(elapsed / SECONDS_PER_SLOT);
        if slot_number.0 < this.minimum_receipt_slot
            || this
                .last_receipt_slot
                .is_some_and(|last| slot_number.0 > last)
        {
            log::warn!("HOLD: HTTP receipt {tx_hash} is outside this receiver deployment domain");
            continue;
        }

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
