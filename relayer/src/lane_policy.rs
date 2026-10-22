//! Independently approved outbound effect requirements, fixed before signing.
use alloy::primitives::{Address, B256};
use anyhow::{ensure, Context, Result};
use ethereum_client::{EthApi, PreparedContentMessage};
use gear_rpc_client::dto::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LanePolicy {
    pub approval_sha256: B256,
    pub chain_id: u64,
    pub genesis_hash: B256,
    pub queue: Address,
    pub source_manager: B256,
    pub destination: Address,
    pub evm_manager: Option<Address>,
    pub token_delivery_required: bool,
    pub original_hash: Option<B256>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ApprovedLane {
    chain_id: u64,
    genesis_hash: B256,
    queue: Address,
    source_manager: B256,
    destination: Address,
    evm_manager: Option<Address>,
    token_delivery_required: bool,
    original_hash: Option<B256>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ApprovalFile {
    schema_version: u8,
    lanes: Vec<ApprovedLane>,
}

#[derive(Clone, Debug, Default)]
pub struct LaneApprovals {
    policies: Vec<LanePolicy>,
}

impl LaneApprovals {
    pub fn validate_saved(&self, policy: &LanePolicy, api: &EthApi, message: &Message, signed: &PreparedContentMessage) -> Result<()> {
        ensure!(self.policies.contains(policy), "HOLD: retained policy differs from independently approved lane");
        policy.validate_for_api(api, message)?;
        policy.validate(message, signed)
    }

    #[cfg(test)]
    pub(crate) fn from_policy(policy: LanePolicy) -> Self {
        Self { policies: vec![policy] }
    }

    pub fn load(path: &Path, expected_sha256: B256) -> Result<Self> {
        let bytes = std::fs::read(path).context("reading outbound lane approval")?;
        ensure!(expected_sha256 != B256::ZERO && B256::from_slice(&Sha256::digest(&bytes)) == expected_sha256,
            "HOLD: outbound lane approval digest differs from independent approval");
        let approval: ApprovalFile = serde_json::from_slice(&bytes)?;
        ensure!(approval.schema_version == 1 && !approval.lanes.is_empty(), "HOLD: invalid outbound lane approval schema or empty lanes");
        let policies = approval.lanes.into_iter().map(|lane| LanePolicy {
            approval_sha256: expected_sha256,
            chain_id: lane.chain_id,
            genesis_hash: lane.genesis_hash,
            queue: lane.queue,
            source_manager: lane.source_manager,
            destination: lane.destination,
            evm_manager: lane.evm_manager,
            token_delivery_required: lane.token_delivery_required,
            original_hash: lane.original_hash,
        }).collect::<Vec<_>>();
        for policy in &policies { policy.validate_shape()?; }
        Ok(Self { policies })
    }

    pub fn qualify(&self, api: &EthApi, message: &Message, original: Option<&PreparedContentMessage>) -> Result<LanePolicy> {
        let identity = api.finality_identity()?;
        let queue = Address::from_slice(api.message_queue_address().as_bytes());
        let mut matches = self.policies.iter().filter(|policy| {
            (policy.chain_id, policy.genesis_hash) == identity
                && policy.queue == queue
                && policy.source_manager == B256::from(message.source)
                && policy.destination == Address::from(message.destination)
                && policy.original_hash == original.map(|signed| signed.hash)
        });
        let policy = matches.next().context("HOLD: no independently approved exact outbound lane/original policy")?;
        ensure!(matches.next().is_none(), "HOLD: ambiguous outbound lane approval");
        policy.validate_for_api(api, message)?;
        if let Some(signed) = original { policy.validate(message, signed)?; }
        Ok(policy.clone())
    }
}

impl LanePolicy {
    fn validate_shape(&self) -> Result<()> {
        ensure!(self.approval_sha256 != B256::ZERO && self.chain_id != 0 && self.genesis_hash != B256::ZERO
            && self.queue != Address::ZERO && self.source_manager != B256::ZERO && self.destination != Address::ZERO,
            "HOLD: incomplete outbound lane identity");
        ensure!(if self.token_delivery_required { self.evm_manager == Some(self.destination) } else { self.evm_manager.is_none() },
            "HOLD: token lane requires exact EVM manager; generic lane must be explicitly separate");
        ensure!(self.original_hash != Some(B256::ZERO), "HOLD: zero historical original hash");
        Ok(())
    }

    pub fn validate_for_api(&self, api: &EthApi, message: &Message) -> Result<()> {
        self.validate_shape()?;
        ensure!((self.chain_id, self.genesis_hash) == api.finality_identity()?
            && self.queue.as_slice() == api.message_queue_address().as_bytes(), "HOLD: outbound lane network/queue differs");
        self.validate_message(message)
    }

    fn validate_message(&self, message: &Message) -> Result<()> {
        ensure!(self.source_manager == B256::from(message.source) && self.destination == Address::from(message.destination),
            "HOLD: outbound lane source/destination differs");
        ensure!(!self.token_delivery_required || message.payload.len() == 104, "HOLD: malformed approved token payload");
        Ok(())
    }

    pub fn validate(&self, message: &Message, signed: &PreparedContentMessage) -> Result<()> {
        self.validate_shape()?;
        self.validate_message(message)?;
        ensure!(self.chain_id == signed.chain_id && self.queue == signed.contract
            && self.original_hash.is_none_or(|hash| hash == signed.hash), "HOLD: outbound lane differs from immutable signed original");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> LanePolicy {
        LanePolicy { approval_sha256: B256::repeat_byte(1), chain_id: 31337, genesis_hash: B256::repeat_byte(2),
            queue: Address::repeat_byte(3), source_manager: B256::repeat_byte(4), destination: Address::repeat_byte(5),
            evm_manager: Some(Address::repeat_byte(5)), token_delivery_required: true, original_hash: None }
    }
    #[test]
    fn token_policy_cannot_be_downgraded_or_rebound() {
        let mut policy = policy();
        assert!(policy.validate_shape().is_ok());
        policy.token_delivery_required = false;
        assert!(policy.validate_shape().is_err());
        policy.token_delivery_required = true;
        policy.evm_manager = Some(Address::repeat_byte(6));
        assert!(policy.validate_shape().is_err());
        policy.evm_manager = None;
        assert!(policy.validate_shape().is_err());
        policy.token_delivery_required = false;
        assert!(policy.validate_shape().is_ok());
    }
    #[test]
    fn approval_rejects_wrong_digest_and_unknown_fields() {
        let path = std::env::temp_dir().join(format!("outbound-approval-{}.json", uuid::Uuid::new_v4()));
        let bytes = br#"{"schemaVersion":1,"lanes":[],"allowUnknown":true}"#;
        std::fs::write(&path, bytes).unwrap();
        assert!(LaneApprovals::load(&path, B256::repeat_byte(1)).is_err());
        assert!(LaneApprovals::load(&path, B256::from_slice(&Sha256::digest(bytes))).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
