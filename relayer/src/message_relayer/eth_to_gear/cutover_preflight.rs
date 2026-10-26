//! Independently approved cutover coverage domain. Not inferred from the discovery cursor.
use super::{fetch_covered_block, CoveredBlock};
use alloy::{
    consensus::{transaction::SignerRecoverable, Transaction},
    eips::eip2718::{Decodable2718, Encodable2718},
};
use anyhow::{ensure, Context, Result};
use primitive_types::{H160, H256};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoveragePolicy {
    pub ethereum_chain_id: u64,
    pub ethereum_genesis_hash: H256,
    pub erc20_manager: H160,
    pub deployment_block: u64,
    pub deployment_hash: H256,
    pub deployment_transaction: H256,
    pub deployment_transaction_index: u64,
    #[serde(rename = "historicalReceipts", default)]
    pub historical_receipts: Vec<gear_common::receipt_framing::HistoricalReceiptApproval>,
    /// Sorted, unique historically approved payment contracts; registration alone is not approval.
    pub fee_emitters: Vec<H160>,
    pub max_redirect_bytes: u32,
    pub max_gas: u64,
}
impl CoveragePolicy {
    pub fn validate(&self) -> Result<()> {
        gear_common::receipt_framing::validate_historical_receipts(&self.historical_receipts)?;
        ensure!(
            self.ethereum_chain_id != 0
                && !self.ethereum_genesis_hash.is_zero()
                && !self.erc20_manager.is_zero()
                && !self.deployment_hash.is_zero()
                && !self.deployment_transaction.is_zero()
                && self.max_redirect_bytes > 0
                && self.max_gas > 0,
            "HOLD: incomplete approved coverage policy"
        );
        ensure!(
            self.fee_emitters.iter().all(|id| !id.is_zero())
                && self.fee_emitters.windows(2).all(|pair| pair[0] < pair[1]),
            "HOLD: historical fee emitters must be sorted and unique"
        );
        Ok(())
    }
    pub fn validate_deployment(&self, block: &CoveredBlock) -> Result<()> {
        self.validate()?;
        super::validate_covered_block(block)?;
        ensure!(
            block.number == self.deployment_block && block.hash == self.deployment_hash,
            "HOLD: deployment header differs from independent approval"
        );
        let index = usize::try_from(self.deployment_transaction_index)?;
        ensure!(
            block.transactions.get(index).map(|hash| hash.0) == Some(self.deployment_transaction.0),
            "HOLD: deployment transaction is not in authenticated inventory"
        );
        let raw = block
            .raw_transactions
            .get(index)
            .context("HOLD: deployment original missing")?;
        let tx = alloy::consensus::TxEnvelope::decode_2718(&mut raw.as_slice())?;
        // CREATE provenance is derivable from the trie-bound original, unlike RPC contractAddress.
        // Factory/CREATE2 deployments require their separately reviewed provenance verifier.
        ensure!(tx.to().is_none() && tx.recover_signer()?.create(tx.nonce()).as_slice() == self.erc20_manager.as_bytes(),
            "HOLD: manager deployment lacks authenticated CREATE provenance; factory deployment unsupported");
        let receipt = block
            .receipts
            .iter()
            .find(|receipt| receipt.transaction_index == Some(self.deployment_transaction_index))
            .context("HOLD: deployment receipt missing")?;
        ensure!(
            explicit_success(receipt),
            "HOLD: deployment receipt has no explicit successful status"
        );
        Ok(())
    }
    pub async fn authenticate(
        &self,
        eth: &ethereum_client::PollingEthApi,
        view: &mut ethereum_client::FinalizedView,
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            eth.finality_identity()?
                == (self.ethereum_chain_id, self.ethereum_genesis_hash.0.into()),
            "HOLD: coverage policy chain differs"
        );
        view.verify_blocks(
            &**eth,
            &std::collections::BTreeMap::from([(
                self.deployment_block,
                self.deployment_hash.0.into(),
            )]),
        )
        .await?;
        let block =
            fetch_covered_block(eth, self.deployment_block, self.deployment_hash.0.into()).await?;
        self.validate_deployment(&block)?;
        view.recheck(&**eth).await?;
        Ok(())
    }
}

pub fn explicit_success(receipt: &alloy::rpc::types::TransactionReceipt) -> bool {
    ethereum_common::utils::map_receipt_envelope(receipt.as_ref())
        .as_receipt()
        .is_some_and(|receipt| {
            matches!(receipt.status, alloy::consensus::Eip658Value::Eip658(true))
        })
}

/// Structural equality only. Historical callers MUST additionally authenticate the retained
/// original's manager and proxy-selected endpoint CodeIds using MessageSender.
pub fn receipt_bytes_match(
    raw: &[u8],
    receipt: &alloy::rpc::types::TransactionReceipt,
    policy: Option<&CoveragePolicy>,
    manager: H256,
    slot: u64,
) -> bool {
    let envelope = ethereum_common::utils::map_receipt_envelope(receipt.as_ref());
    raw == envelope.encoded_2718()
        || policy.is_some_and(|policy| {
            policy.historical_receipts.iter().any(|approval| {
                approval.manager_id == manager
                    && (approval.minimum_slot..=approval.maximum_slot).contains(&slot)
            }) && raw == alloy_rlp::encode(&envelope)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Reconstructed from HTML7fe1b032 VB-23/24; no original PoC bundle available.
    #[test]
    fn reconstructed_html7fe1b032_sorted_historical_fee_policy() {
        let mut policy = CoveragePolicy {
            ethereum_chain_id: 1,
            ethereum_genesis_hash: H256::repeat_byte(1),
            erc20_manager: H160::repeat_byte(2),
            deployment_block: 10,
            deployment_hash: H256::repeat_byte(3),
            historical_receipts: vec![],
            deployment_transaction: H256::repeat_byte(4),
            deployment_transaction_index: 0,
            fee_emitters: vec![H160::repeat_byte(5), H160::repeat_byte(6)],
            max_redirect_bytes: 1024,
            max_gas: 100,
        };
        assert!(policy.validate().is_ok());
        policy.fee_emitters.reverse();
        assert!(policy.validate().is_err());
        policy.fee_emitters = vec![H160::repeat_byte(5); 2];
        assert!(policy.validate().is_err());
    }
}
