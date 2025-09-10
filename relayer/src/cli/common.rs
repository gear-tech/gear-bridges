use std::path::PathBuf;

use clap::Args;

#[derive(Args)]
pub struct BlockStorageArgs {
    /// Path to the block storage. For merkle root relayer this should be
    /// a path to JSON file where unprocessed blocks will be saved. For
    /// other relayers, this should be a path to a directory where blocks
    /// are stored in individual files.
    #[arg(long = "block-storage", env = "GEAR_BLOCK_STORAGE")]
    pub block_storage_path: Option<PathBuf>,
}

#[derive(Args)]
pub struct ProofStorageArgs {
    /// Gear fee payer. If not set, proofs are saved to file system
    #[arg(long = "gear-fee-payer", env = "GEAR_FEE_PAYER")]
    pub gear_fee_payer: Option<String>,
}

#[derive(Args)]
pub struct GenesisConfigArgs {
    /// Authority set hash used in genesis config
    #[arg(long = "authority-set-hash", env = "GENESIS_CONFIG_AUTHORITY_SET_HASH")]
    pub authority_set_hash: String,
    /// Authority set id used in genesis config
    #[arg(long = "authority-set-id", env = "GENESIS_CONFIG_AUTHORITY_SET_ID")]
    pub authority_set_id: u64,
}

#[derive(Args)]
pub struct GearSignerArgs {
    #[clap(flatten)]
    pub common: GearArgs,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long = "gear-suri", env = "GEAR_SURI")]
    pub suri: String,
}

#[derive(Args)]
pub struct GearArgs {
    /// Domain of the Gear RPC endpoint
    #[arg(
        long = "gear-domain",
        default_value = "ws://127.0.0.1",
        env = "GEAR_DOMAIN"
    )]
    pub domain: String,

    /// Port of the Gear RPC endpoint
    #[arg(long = "gear-port", default_value = "9944", env = "GEAR_PORT")]
    pub port: u16,

    /// Retry count of the Gear RPC client
    #[arg(
        long = "gear-rpc-retries",
        default_value = "3",
        env = "GEAR_RPC_RETRIES"
    )]
    pub retries: u8,
}

#[derive(Args)]
pub struct EthereumSignerArgs {
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,

    /// Private key for fee payer
    #[arg(long = "eth-fee-payer", env = "ETH_FEE_PAYER")]
    pub fee_payer: String,
}

#[derive(Args)]
pub struct EthereumSignerPathArgs {
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,

    /// Private key for fee payer
    #[arg(long = "eth-fee-payer-path", env = "ETH_FEE_PAYER_PATH")]
    pub fee_payer_path: PathBuf,
}

#[derive(Args, Clone)]
pub struct EthereumArgs {
    /// Address of the ethereum endpoint
    #[arg(long = "ethereum-endpoint", env = "ETH_RPC")]
    pub eth_endpoint: String,

    /// Number of retries for the ethereum endpoint
    #[arg(long = "eth-max-retries", env = "ETH_RPC_MAX_RETRIES")]
    pub eth_max_retries: Option<u32>,

    // Interval in milliseconds between retries for the ethereum endpoint
    #[arg(long = "eth-retry-interval-ms", env = "ETH_RPC_RETRY_INTERVAL_MS")]
    pub eth_retry_interval_ms: Option<u64>,

    /// Ethereum address of message queue contract
    #[arg(long = "mq-address", env = "ETH_MESSAGE_QUEUE_ADDRESS")]
    pub mq_address: String,
}

#[derive(Args)]
pub struct BeaconRpcArgs {
    /// Address of the ethereum beacon RPC endpoint
    #[arg(long = "ethereum-beacon-rpc", env = "ETH_BEACON_RPC")]
    pub beacon_endpoint: String,

    /// Timeout in seconds for requests to the ethereum beacon RPC
    #[arg(
        long = "ethereum-beacon-rpc-timeout",
        env = "ETH_BEACON_RPC_TIMEOUT",
        default_value = "10"
    )]
    pub beacon_timeout: Option<u64>,
}

#[derive(Args)]
pub struct RelayerHttpArgs {
    /// URL of the relayer HTTP endpoint
    #[arg(long = "relayer-http-url", env = "RELAYER_HTTP_URL")]
    pub url: String,

    /// Access token for the relayer HTTP endpoint
    #[arg(long = "relayer-http-access-token", env = "RELAYER_HTTP_ACCESS_TOKEN")]
    pub access_token: String,

    /// Timeout in seconds for requests to the relayer HTTP endpoint
    #[arg(
        long = "relayer-http-timeout-secs",
        default_value = "30",
        env = "RELAYER_HTTP_TIMEOUT_SECS"
    )]
    pub timeout_secs: u64,
}

#[derive(Args)]
pub struct PrometheusArgs {
    /// Address of the prometheus endpoint
    #[arg(
        long = "prometheus-endpoint",
        default_value = "0.0.0.0:9090",
        env = "PROMETHEUS_ENDPOINT"
    )]
    pub endpoint: String,
}
