use clap::Args;

#[derive(Args, Clone, Debug)]
pub struct EthereumConnectionArgs {
    /// Address of the ethereum endpoint
    #[arg(long = "ethereum-endpoint", env = "ETH_RPC")]
    pub ethereum_endpoint: String,

    /// Number of retries for the ethereum endpoint
    #[arg(long = "eth-max-retries", env = "ETH_RPC_MAX_RETRIES")]
    pub max_retries: Option<u32>,

    // Interval in milliseconds between retries for the ethereum endpoint
    #[arg(long = "eth-retry-interval-ms", env = "ETH_RPC_RETRY_INTERVAL_MS")]
    pub retry_interval_ms: Option<u64>,
}

#[derive(Args, Clone, Debug)]
pub struct EthereumTxArgs {
    /// Max fee per gas (in wei) for transactions sent to Ethereum. Default is
    /// 2_000_000_000 (2 Gwei).
    #[arg(long = "max-fee-per-gas", env = "ETH_MAX_FEE_PER_GAS")]
    pub max_fee_per_gas: Option<u128>,

    /// Max priority fee per gas (in wei) for transactions sent to Ethereum. Default is
    /// 500_000_000 (0.5 Gwei).
    #[arg(
        long = "max-priority-fee-per-gas",
        env = "ETH_MAX_PRIORITY_FEE_PER_GAS"
    )]
    pub max_priority_fee_per_gas: Option<u128>,
}

#[derive(Args, Clone, Debug)]
pub struct BeaconConnectionArgs {
    /// Address of the ethereum beacon RPC endpoint
    #[arg(
        long = "ethereum-beacon-rpc",
        alias = "beacon-endpoint",
        env = "ETH_BEACON_RPC"
    )]
    pub beacon_endpoint: String,

    /// Timeout in seconds for requests to the ethereum beacon RPC
    #[arg(
        long = "ethereum-beacon-rpc-timeout",
        alias = "beacon-timeout",
        env = "ETH_BEACON_RPC_TIMEOUT",
        default_value = "10"
    )]
    pub timeout: Option<u64>,
}

#[derive(Args, Clone, Debug)]
pub struct EthereumSignerArgs {
    #[clap(flatten)]
    pub connection: EthereumConnectionArgs,

    #[clap(flatten)]
    pub tx: EthereumTxArgs,

    /// Ethereum address of message queue contract (Commonly used in bridge context)
    #[arg(long = "mq-address", env = "ETH_MESSAGE_QUEUE_ADDRESS")]
    pub mq_address: Option<String>,

    /// Private key for fee payer
    #[arg(long = "eth-fee-payer", env = "ETH_FEE_PAYER")]
    pub eth_fee_payer: String,
}
