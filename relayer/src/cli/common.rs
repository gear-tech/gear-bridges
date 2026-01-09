use clap::Args;
pub use cli_utils::{
    BeaconConnectionArgs, BlockStorageArgs, EthereumConnectionArgs, EthereumTxArgs,
    GearConnectionArgs, GearSignerArgs, GenesisConfigArgs, PrometheusArgs,
};
use std::path::PathBuf;

#[derive(Args)]
pub struct ProofStorageArgs {
    /// Gear fee payer. If not set, proofs are saved to file system
    #[arg(long = "gear-fee-payer", env = "GEAR_FEE_PAYER")]
    pub gear_fee_payer: Option<String>,
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

    /// Private key for admin role
    #[arg(long = "eth-admin-pk-path", env = "ETH_ADMIN_PK_PATH")]
    pub eth_admin_pk_path: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct EthereumArgs {
    #[clap(flatten)]
    pub connection: EthereumConnectionArgs,

    #[clap(flatten)]
    pub tx: EthereumTxArgs,

    /// Ethereum address of message queue contract
    #[arg(long = "mq-address", env = "ETH_MESSAGE_QUEUE_ADDRESS")]
    pub mq_address: String,
}

#[derive(Args)]
pub struct RelayerHttpArgs {
    /// URL of the relayer HTTP endpoint
    #[arg(long = "relayer-http-url", env = "RELAYER_HTTP_URL")]
    pub url: String,

    /// Access token for the relayer HTTP endpoint
    #[arg(long = "relayer-http-access-token", env = "RELAYER_HTTP_ACCESS_TOKEN")]
    pub access_token: String,

    /// Timeout in seconds for requests to the relayer HTTP endpoint, default is 1800 seconds (30 minutes)
    #[arg(
        long = "relayer-http-timeout-secs",
        default_value = "1800",
        env = "RELAYER_HTTP_TIMEOUT_SECS"
    )]
    pub timeout_secs: u64,
}

// Type aliases for backward compatibility or clarity if needed, though we use the structs directly above.
pub type GearArgs = GearConnectionArgs;
// BeaconRpcArgs was used in relayer, mapping to BeaconConnectionArgs
pub type BeaconRpcArgs = BeaconConnectionArgs;
