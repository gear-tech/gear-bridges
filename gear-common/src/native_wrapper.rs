//! Explicit, independently approved immutable wrapper capabilities. RPC errors never select an ABI.
use anyhow::{ensure, Context, Result};
use gclient::GearApi;
use primitive_types::{H160, H256};
use sails_rs::{calls::*, gclient::calls::*, Decode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use vft_manager_client::traits::VftManager as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeWrapperMode {
    EscrowV1,
    HeldEscrowV2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeWrapperBinding {
    pub actor_id: H256,
    pub ethereum_token: H160,
    pub code_id: H256,
    pub mode: NativeWrapperMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagerWrapperAbi {
    ActorOnly,
    CapabilityBound,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagerProgramApproval {
    pub actor_id: H256,
    pub code_id: H256,
    pub wrapper_abi: ManagerWrapperAbi,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeWrapperApproval {
    pub schema_version: u32,
    pub gear_genesis_hash: H256,
    pub source_manager: ManagerProgramApproval,
    pub replacement_manager: ManagerProgramApproval,
    pub wrapper: NativeWrapperBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedNativeWrapper {
    pub approval_digest: H256,
    pub approval: NativeWrapperApproval,
}

impl NativeWrapperApproval {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && !self.gear_genesis_hash.is_zero(),
            "HOLD: unsupported wrapper approval or zero genesis"
        );
        for manager in [&self.source_manager, &self.replacement_manager] {
            ensure!(
                !manager.actor_id.is_zero() && !manager.code_id.is_zero(),
                "HOLD: zero approved manager identity"
            );
            ensure!(
                manager.wrapper_abi != ManagerWrapperAbi::ActorOnly
                    || self.wrapper.mode == NativeWrapperMode::EscrowV1,
                "HOLD: actor-only predecessor supports only explicitly approved escrow-v1"
            );
        }
        ensure!(
            self.source_manager.actor_id != self.replacement_manager.actor_id
                || self.source_manager == self.replacement_manager,
            "HOLD: same manager has conflicting approved program bindings"
        );
        ensure!(
            !self.wrapper.actor_id.is_zero()
                && !self.wrapper.code_id.is_zero()
                && !self.wrapper.ethereum_token.is_zero(),
            "HOLD: zero approved native wrapper identity"
        );
        Ok(())
    }
}

pub fn load_native_wrapper_approval(
    path: &Path,
    expected_digest: H256,
) -> Result<ApprovedNativeWrapper> {
    ensure!(
        !expected_digest.is_zero(),
        "HOLD: independent wrapper approval digest is required"
    );
    let bytes = std::fs::read(path).context("reading independently approved wrapper policy")?;
    let digest = H256::from_slice(&Sha256::digest(&bytes));
    ensure!(
        digest == expected_digest,
        "HOLD: native wrapper approval SHA-256 differs"
    );
    let approval: NativeWrapperApproval = serde_json::from_slice(&bytes)?;
    approval.validate()?;
    Ok(ApprovedNativeWrapper {
        approval_digest: digest,
        approval,
    })
}

pub async fn native_wrapper_at(
    api: &GearApi,
    manager: H256,
    at: H256,
    approval: Option<&ApprovedNativeWrapper>,
) -> Result<Option<NativeWrapperBinding>> {
    let number = api.block_number_at(at).await?;
    let finalized = api.last_block_hash().await?;
    ensure!(
        number <= api.block_number_at(finalized).await? && api.get_block_hash(number).await? == at,
        "HOLD: wrapper capability pin is not canonical finalized history"
    );
    let approved = if let Some(approved) = approval {
        approved.approval.validate()?;
        ensure!(
            !approved.approval_digest.is_zero(),
            "HOLD: missing approval digest"
        );
        let policy = &approved.approval;
        let signer: gsdk::signer::Signer = api.clone().into();
        ensure!(
            signer.api().genesis_hash().as_ref() == policy.gear_genesis_hash.as_bytes()
                && api.get_block_hash(0).await?.as_ref() == policy.gear_genesis_hash.as_bytes(),
            "HOLD: approved Gear genesis differs"
        );
        let identity = if manager == policy.source_manager.actor_id {
            &policy.source_manager
        } else {
            ensure!(
                manager == policy.replacement_manager.actor_id,
                "HOLD: manager absent from wrapper approval"
            );
            &policy.replacement_manager
        };
        let program = api.program_at(manager.0.into(), Some(at)).await?;
        ensure!(
            program.state == gear_core::program::ProgramState::Initialized
                && program.code_id.into_bytes() == identity.code_id.0,
            "HOLD: approved manager CodeId differs"
        );
        Some((policy, identity))
    } else {
        None
    };
    if approved.is_some_and(|(_, identity)| identity.wrapper_abi == ManagerWrapperAbi::ActorOnly) {
        // Immutable predecessor route and SCALE Option<ActorId>, never the replacement reply ABI.
        use vft_manager_client::vft_manager::io::NativeWrapper;
        let reply = api
            .calculate_reply_for_handle_at(
                None,
                manager.0.into(),
                NativeWrapper::encode_call(),
                api.block_gas_limit()?,
                0,
                Some(at),
            )
            .await?;
        ensure!(
            reply.code.is_success(),
            "HOLD: approved actor-only NativeWrapper query failed"
        );
        let mut body = reply
            .payload
            .strip_prefix(NativeWrapper::ROUTE)
            .context("HOLD: predecessor NativeWrapper route differs")?;
        let actor = Option::<sails_rs::ActorId>::decode(&mut body)?;
        ensure!(
            body.is_empty(),
            "HOLD: predecessor NativeWrapper trailing bytes"
        );
        let (policy, _) = approved.expect("explicit actor-only approval");
        ensure!(
            actor == Some(policy.wrapper.actor_id.0.into()),
            "HOLD: approved predecessor wrapper differs"
        );
    } else {
        let actual = vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()))
            .native_wrapper()
            .at_block(at.0.into())
            .recv(manager.0.into())
            .await?;
        let Some((policy, _)) = approved else {
            ensure!(
                actual.is_none(),
                "HOLD: native wrapper requires independent CodeId-bound capability approval"
            );
            return Ok(None);
        };
        let actual = actual.context("HOLD: approved native wrapper is not configured")?;
        let mode = match actual.mode {
            vft_manager_client::NativeWrapperMode::EscrowV1 => NativeWrapperMode::EscrowV1,
            vft_manager_client::NativeWrapperMode::HeldEscrowV2 => NativeWrapperMode::HeldEscrowV2,
        };
        ensure!(
            actual.actor_id == policy.wrapper.actor_id.0.into()
                && actual.ethereum_token.0 == policy.wrapper.ethereum_token.0
                && actual.code_id.0 == policy.wrapper.code_id.0
                && mode == policy.wrapper.mode,
            "HOLD: manager native capability differs from approval"
        );
    }
    let binding = &approved.expect("Some wrapper requires approval").0.wrapper;
    let program = api.program_at(binding.actor_id.0.into(), Some(at)).await?;
    ensure!(
        program.state == gear_core::program::ProgramState::Initialized
            && program.code_id.into_bytes() == binding.code_id.0,
        "HOLD: actual wrapper CodeId differs from approved capability"
    );
    Ok(Some(binding.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approval_identity_and_abi_are_explicit() {
        let manager = ManagerProgramApproval {
            actor_id: H256::repeat_byte(1),
            code_id: H256::repeat_byte(2),
            wrapper_abi: ManagerWrapperAbi::ActorOnly,
        };
        let mut approval = NativeWrapperApproval {
            schema_version: 1,
            gear_genesis_hash: H256::repeat_byte(3),
            source_manager: manager.clone(),
            replacement_manager: manager,
            wrapper: NativeWrapperBinding {
                actor_id: H256::repeat_byte(4),
                ethereum_token: H160::repeat_byte(5),
                code_id: H256::repeat_byte(6),
                mode: NativeWrapperMode::EscrowV1,
            },
        };
        assert!(approval.validate().is_ok());
        approval.wrapper.mode = NativeWrapperMode::HeldEscrowV2;
        assert!(approval.validate().is_err());
        approval.wrapper.mode = NativeWrapperMode::EscrowV1;
        approval.replacement_manager.code_id = H256::repeat_byte(7);
        assert!(approval.validate().is_err());
        approval.replacement_manager = approval.source_manager.clone();
        approval.wrapper.code_id = H256::zero();
        assert!(approval.validate().is_err());
        approval.wrapper.code_id = H256::repeat_byte(6);
        let bytes = serde_json::to_vec(&approval).unwrap();
        let digest = H256::from_slice(&Sha256::digest(&bytes));
        let path = std::env::temp_dir().join(format!(
            "native-wrapper-approval-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            load_native_wrapper_approval(&path, digest)
                .unwrap()
                .approval,
            approval
        );
        assert!(load_native_wrapper_approval(&path, H256::repeat_byte(9)).is_err());
        let mut unknown = serde_json::to_value(&approval).unwrap();
        unknown["allow_unsafe_fallback"] = serde_json::json!(true);
        let unknown = serde_json::to_vec(&unknown).unwrap();
        std::fs::write(&path, &unknown).unwrap();
        assert!(
            load_native_wrapper_approval(&path, H256::from_slice(&Sha256::digest(&unknown)))
                .is_err()
        );
        std::fs::remove_file(path).unwrap();
    }
}
