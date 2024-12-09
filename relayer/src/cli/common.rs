use clap::Args;

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
    pub gear_args: VaraArgs,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, env = "VARA_SURI")]
    pub vara_suri: String,
}

#[derive(Args)]
pub struct VaraArgs {
    /// Domain of the VARA RPC endpoint
    #[arg(long, default_value = "ws://127.0.0.1", env = "VARA_DOMAIN")]
    pub vara_domain: String,

    /// Port of the VARA RPC endpoint
    #[arg(long, default_value = "9944", env = "VARA_PORT")]
    pub vara_port: u16,

    /// Set retries of the VARA RPC client
    #[arg(long, default_value = "3", env = "VARA_RPC_RETRIES")]
    pub vara_rpc_retries: u8,
}

#[derive(Args)]
pub struct EthereumArgs {
    /// Address of the ethereum endpoint
    #[arg(long = "ethereum-endpoint", env = "ETH_RPC")]
    pub eth_endpoint: String,
    /// Private key for fee payer
    #[arg(long = "eth-fee-payer", env = "ETH_FEE_PAYER")]
    pub fee_payer: Option<String>,
    /// Ethereum address of relayer contract
    #[arg(long = "relayer-address", env = "ETH_RELAYER_ADDRESS")]
    pub relayer_address: String,
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
    #[arg(long = "ethereum-beacon-rpc-timeout", env = "ETH_BEACON_RPC_TIMEOUT")]
    pub beacon_timeout: Option<u64>,
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
