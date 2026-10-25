use clap::Args;
pub use cli_utils::{
    BeaconConnectionArgs, EthereumChainIdentityArgs, EthereumConnectionArgs, EthereumFinalityArgs,
    EthereumTxArgs, GearConnectionArgs, GearSignerArgs, PrometheusArgs,
};
use std::path::PathBuf;

#[derive(Args, Clone)]
pub struct NativeWrapperApprovalArgs {
    /// Exact independently approved capability file; required for any native wrapper.
    #[arg(long, requires = "native_wrapper_approval_sha256")]
    pub native_wrapper_approval: Option<PathBuf>,
    #[arg(long, requires = "native_wrapper_approval")]
    pub native_wrapper_approval_sha256: Option<primitive_types::H256>,
}

impl NativeWrapperApprovalArgs {
    pub fn load(
        &self,
    ) -> anyhow::Result<Option<std::sync::Arc<gear_common::native_wrapper::ApprovedNativeWrapper>>>
    {
        match (
            &self.native_wrapper_approval,
            self.native_wrapper_approval_sha256,
        ) {
            (None, None) => Ok(None),
            (Some(path), Some(digest)) => Ok(Some(std::sync::Arc::new(
                gear_common::native_wrapper::load_native_wrapper_approval(path, digest)?,
            ))),
            _ => anyhow::bail!(
                "HOLD: wrapper approval file and independent SHA-256 are required together"
            ),
        }
    }
}

#[derive(Args)]
pub struct LaneApprovalArgs {
    /// Independently approved outbound lane identities and immutable delivery policies.
    #[arg(long, env = "GEAR_ETH_LANE_APPROVAL")]
    pub lane_approval: PathBuf,
    /// Independently pinned SHA-256 of the exact lane approval file.
    #[arg(long, env = "GEAR_ETH_LANE_APPROVAL_SHA256")]
    pub lane_approval_sha256: alloy::primitives::B256,
}

impl LaneApprovalArgs {
    pub fn load(&self) -> anyhow::Result<crate::lane_policy::LaneApprovals> {
        crate::lane_policy::LaneApprovals::load(&self.lane_approval, self.lane_approval_sha256)
    }
}

#[derive(Args)]
pub struct EthereumSignerArgs {
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,

    /// Private key for fee payer
    #[arg(long = "eth-fee-payer", env = "ETH_FEE_PAYER")]
    pub eth_fee_payer: String,
}

#[derive(Args)]
pub struct EthereumKillSwitchArgs {
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,

    /// Private key for observer role
    #[arg(long = "eth-observer-pk-path", env = "ETH_OBSERVER_PK_PATH")]
    pub eth_observer_pk_path: PathBuf,
}

#[derive(Args, Clone)]
pub struct EthereumArgs {
    #[clap(flatten)]
    pub connection: EthereumConnectionArgs,

    #[clap(flatten)]
    pub finality: EthereumFinalityArgs,
    #[clap(flatten)]
    pub identity: EthereumChainIdentityArgs,

    #[clap(flatten)]
    pub tx: EthereumTxArgs,

    /// Ethereum address of message queue contract
    #[arg(long = "mq-address", env = "ETH_MESSAGE_QUEUE_ADDRESS")]
    pub mq_address: String,

    /// Direct CREATE locator; finalized transaction/receipt tries and signature authenticate queue birth
    #[arg(long, env = "QUEUE_DEPLOYMENT_TRANSACTION")]
    pub queue_deployment_transaction: Option<alloy::primitives::B256>,
}

// Type aliases for backward compatibility or clarity if needed, though we use the structs directly above.
pub type GearArgs = GearConnectionArgs;
// BeaconRpcArgs was used in relayer, mapping to BeaconConnectionArgs
pub type BeaconRpcArgs = BeaconConnectionArgs;
