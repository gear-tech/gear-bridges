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

/// Beacon approval is finality authority; an optional EL witness only corroborates data.
#[derive(Args, Clone, Debug)]
pub struct EthereumFinalityArgs {
    #[arg(long = "ethereum-witness-rpc", env = "ETH_WITNESS_RPC")]
    pub ethereum_witness_rpc: Option<String>,

    #[arg(long = "ethereum-finality-beacon-rpc", env = "ETH_FINALITY_BEACON_RPC")]
    pub ethereum_finality_beacon_rpc: String,

    #[arg(long = "ethereum-finality-bootstrap", env = "ETH_FINALITY_BOOTSTRAP")]
    pub ethereum_finality_bootstrap: std::path::PathBuf,

    #[arg(
        long = "ethereum-finality-bootstrap-sha256",
        env = "ETH_FINALITY_BOOTSTRAP_SHA256"
    )]
    pub ethereum_finality_bootstrap_sha256: String,

    /// Exclusive archive directory for this process; clones reuse its owner lock.
    #[arg(long = "ethereum-finality-store", env = "ETH_FINALITY_STORE")]
    pub ethereum_finality_store: std::path::PathBuf,

    /// Read-only peer archives supplying retained immutable Beacon proof records.
    #[arg(long = "ethereum-finality-proof-source")]
    pub ethereum_finality_proof_sources: Vec<std::path::PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub struct EthereumChainIdentityArgs {
    #[arg(long = "ethereum-chain-id", env = "ETH_CHAIN_ID")]
    pub ethereum_chain_id: u64,
    #[arg(long = "ethereum-genesis-hash", env = "ETH_GENESIS_HASH")]
    pub ethereum_genesis_hash: String,
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
