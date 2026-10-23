use crate::{
    inclusion::{authenticated_receipt_block, ReceiptBlockErrors},
    EthApi, FinalizedView,
};
use alloy::{
    consensus::{transaction::SignerRecoverable, Transaction, TxEnvelope},
    eips::{Decodable2718, Encodable2718},
    primitives::{keccak256, Address, Bytes, B256},
    providers::Provider,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A direct signed CREATE, qualified against canonical finalized transaction and receipt roots.
/// The RPC receipt is only a locator; its contractAddress is never an authority for the floor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueDeployment {
    pub chain_id: u64,
    pub genesis_hash: B256,
    pub queue: Address,
    pub transaction_hash: B256,
    pub block_number: u64,
    pub block_hash: B256,
    pub transaction_index: u64,
    pub raw_transaction: Bytes,
}

impl QueueDeployment {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.genesis_hash != B256::ZERO && self.block_hash != B256::ZERO,
            "HOLD: queue deployment lacks chain/inclusion identity"
        );
        validate_creation(
            &self.raw_transaction,
            self.transaction_hash,
            self.chain_id,
            self.queue,
        )?;
        Ok(())
    }
}

fn validate_creation(raw: &[u8], hash: B256, chain_id: u64, queue: Address) -> Result<()> {
    ensure!(
        keccak256(raw) == hash,
        "HOLD: queue creation transaction hash changed"
    );
    let mut input = raw;
    let tx = TxEnvelope::decode_2718(&mut input).context("HOLD: invalid signed queue creation")?;
    ensure!(
        input.is_empty() && tx.to().is_none() && tx.chain_id() == Some(chain_id),
        "HOLD: queue deployment must be a signed direct CREATE on the bound chain"
    );
    let sender = tx
        .recover_signer()
        .context("HOLD: invalid queue creation signature")?;
    ensure!(
        sender.create(tx.nonce()) == queue,
        "HOLD: signed CREATE does not deploy the bound queue"
    );
    Ok(())
}

impl EthApi {
    pub async fn authenticate_queue_deployment(
        &self,
        hash: B256,
        view: &mut FinalizedView,
    ) -> Result<QueueDeployment> {
        let (approved_chain, approved_genesis) = self.finality_identity()?;
        ensure!(
            view.checkpoint().chain_id() == approved_chain
                && view.genesis_hash() == approved_genesis,
            "HOLD: queue deployment view differs from configured identity"
        );
        let provider = self.raw_provider();
        let receipt = provider
            .get_transaction_receipt(hash)
            .await?
            .context("queue deployment receipt is not available")?;
        ensure!(
            receipt.transaction_hash == hash,
            "HOLD: queue deployment locator changed hash"
        );
        let number = receipt
            .block_number
            .context("HOLD: queue deployment receipt lacks inclusion number")?;
        let block_hash = receipt
            .block_hash
            .context("HOLD: queue deployment receipt lacks inclusion hash")?;
        let index = receipt
            .transaction_index
            .context("HOLD: queue deployment receipt lacks transaction index")?;
        let (_, transactions, envelopes) = authenticated_receipt_block(
            provider,
            number,
            block_hash,
            ReceiptBlockErrors {
                block: "queue creation full block unavailable",
                receipts: "queue creation receipt set unavailable",
                header: "HOLD: queue creation header differs from finalized inclusion",
                transactions: "HOLD: queue creation full transactions missing",
                transaction_root: Some(
                    "HOLD: queue creation transaction trie differs from authenticated header",
                ),
                count: "HOLD: incomplete queue creation receipt set",
                coordinates: "HOLD: queue creation receipt coordinates conflict",
                root: "HOLD: queue creation receipt trie differs from authenticated header",
            },
        )
        .await?;
        let index_usize = usize::try_from(index).context("HOLD: invalid queue creation index")?;
        let tx = transactions
            .get(index_usize)
            .context("HOLD: queue creation transaction absent")?;
        ensure!(
            envelopes[index_usize].1.status(),
            "HOLD: queue creation reverted"
        );
        let deployment = QueueDeployment {
            chain_id: approved_chain,
            genesis_hash: view.genesis_hash(),
            queue: Address::from(self.message_queue_address().0),
            transaction_hash: hash,
            block_number: number,
            block_hash,
            transaction_index: index,
            raw_transaction: tx.encoded_2718().into(),
        };
        deployment.validate()?;
        ensure!(
            deployment.chain_id == self.chain_id,
            "HOLD: queue deployment endpoint chain changed"
        );
        // Reject a wrong creation locator before walking history; no floor is accepted
        // until this header is connected to the independently qualified finalized view.
        view.verify_blocks(provider, &BTreeMap::from([(number, block_hash)]))
            .await?;
        Ok(deployment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{SignableTransaction, TxEip1559},
        network::TxSigner,
        primitives::TxKind,
        signers::local::PrivateKeySigner,
    };

    #[tokio::test]
    async fn creation_hint_requires_exact_signed_create_chain_address_and_bytes() -> Result<()> {
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(17))?;
        let mut tx = TxEip1559 {
            chain_id: 560048,
            nonce: 31,
            gas_limit: 100_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Create,
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut tx).await?;
        let raw = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        let hash = keccak256(&raw);
        let queue = signer.address().create(31);
        validate_creation(&raw, hash, 560048, queue)?;
        assert!(validate_creation(&raw, B256::ZERO, 560048, queue).is_err());
        assert!(validate_creation(&raw, hash, 1, queue).is_err());
        assert!(validate_creation(&raw, hash, 560048, signer.address().create(32)).is_err());
        let mut trailing = raw.clone();
        trailing.push(0);
        assert!(validate_creation(&trailing, keccak256(&trailing), 560048, queue).is_err());
        let mut call = TxEip1559 {
            chain_id: 560048,
            nonce: 31,
            to: TxKind::Call(queue),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut call).await?;
        let raw = TxEnvelope::Eip1559(call.into_signed(signature)).encoded_2718();
        assert!(validate_creation(&raw, keccak256(&raw), 560048, queue).is_err());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires explicit read-only Ethereum and Beacon endpoints, approved bootstrap/store, queue and creation locator"]
    async fn qualify_live_queue_creation_read_only() -> Result<()> {
        let config = crate::BeaconFinalityConfig::load(
            std::env::var("ETH_BEACON_BOOTSTRAP")?,
            std::env::var("ETH_BEACON_BOOTSTRAP_SHA256")?.parse()?,
            &std::env::var("ETH_BEACON_RPC")?,
        )?;
        let mut api = EthApi::new(
            &std::env::var("QUEUE_TEST_RPC")?,
            &std::env::var("QUEUE_TEST_ADDRESS")?,
            None,
            None,
            None,
        )
        .await?
        .with_beacon_finality(config, std::env::var("ETH_BEACON_STORE")?)
        .await?;
        if let Ok(witness) = std::env::var("ETH_WITNESS_RPC") {
            api = api
                .with_data_witness(
                    &witness,
                    std::env::var("ETH_CHAIN_ID")?.parse()?,
                    std::env::var("ETH_GENESIS_HASH")?.parse()?,
                )
                .await?;
        }
        let hash = std::env::var("QUEUE_TEST_CREATION")?.parse()?;
        let mut view = api.verified_finalized_view().await?;
        let deployment = api.authenticate_queue_deployment(hash, &mut view).await?;
        println!("{}", serde_json::to_string(&deployment)?);
        Ok(())
    }
}
