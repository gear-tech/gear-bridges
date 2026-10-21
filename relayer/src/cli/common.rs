use clap::Args;
pub use cli_utils::{
    BeaconConnectionArgs, EthereumChainIdentityArgs, EthereumConnectionArgs,
    EthereumFinalityArgs, EthereumTxArgs, GearConnectionArgs, GearSignerArgs, PrometheusArgs,
};
use std::path::PathBuf;

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
