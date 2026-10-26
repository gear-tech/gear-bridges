//! Independent authority for reading historical receipt originals; never fresh submission policy.
use anyhow::{ensure, Result};
use primitive_types::H256;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistoricalReceiptFraming {
    NetworkRlp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalReceiptApproval {
    pub manager_id: H256,
    pub manager_code_id: H256,
    pub endpoint_id: H256,
    pub endpoint_code_id: H256,
    pub framing: HistoricalReceiptFraming,
    pub minimum_slot: u64,
    pub maximum_slot: u64,
}
impl HistoricalReceiptApproval {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [
                self.manager_id,
                self.manager_code_id,
                self.endpoint_id,
                self.endpoint_code_id
            ]
            .iter()
            .all(|id| !id.is_zero())
                && self.minimum_slot <= self.maximum_slot,
            "HOLD: incomplete historical receipt authority or inverted slot range"
        );
        Ok(())
    }
    /// Inputs must be authenticated at the exact original finalized pin, including proxy selection.
    pub fn matches(
        &self,
        manager: H256,
        manager_code: H256,
        endpoint: H256,
        endpoint_code: H256,
        slot: u64,
    ) -> bool {
        self.manager_id == manager
            && self.manager_code_id == manager_code
            && self.endpoint_id == endpoint
            && self.endpoint_code_id == endpoint_code
            && (self.minimum_slot..=self.maximum_slot).contains(&slot)
    }
}

pub fn validate_historical_receipts(approvals: &[HistoricalReceiptApproval]) -> Result<()> {
    for (index, approval) in approvals.iter().enumerate() {
        approval.validate()?;
        ensure!(
            approvals[..index]
                .iter()
                .all(|other| other.manager_id != approval.manager_id
                    || other.maximum_slot < approval.minimum_slot
                    || approval.maximum_slot < other.minimum_slot),
            "HOLD: overlapping historical receipt authorities for one manager"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Reconstructed from HTML7fe1b032 VB-20. Original PoC bundle unavailable.
    #[test]
    fn reconstructed_html7fe1b032_framing_authority_is_exact_and_unambiguous() {
        let authority = HistoricalReceiptApproval {
            manager_id: H256::repeat_byte(1),
            manager_code_id: H256::repeat_byte(2),
            endpoint_id: H256::repeat_byte(3),
            endpoint_code_id: H256::repeat_byte(4),
            framing: HistoricalReceiptFraming::NetworkRlp,
            minimum_slot: 10,
            maximum_slot: 20,
        };
        assert!(authority.matches(
            authority.manager_id,
            authority.manager_code_id,
            authority.endpoint_id,
            authority.endpoint_code_id,
            10
        ));
        assert!(!authority.matches(
            authority.manager_id,
            H256::repeat_byte(5),
            authority.endpoint_id,
            authority.endpoint_code_id,
            10
        ));
        assert!(!authority.matches(
            authority.manager_id,
            authority.manager_code_id,
            authority.endpoint_id,
            authority.endpoint_code_id,
            21
        ));
        assert!(validate_historical_receipts(std::slice::from_ref(&authority)).is_ok());
        assert!(validate_historical_receipts(&[authority.clone(), authority]).is_err());
    }
}
