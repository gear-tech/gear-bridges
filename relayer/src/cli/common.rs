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
pub struct EthereumArgs {
    /// Address of the ethereum endpoint
    #[arg(long = "ethereum-endpoint", env = "ETH_RPC")]
    pub eth_endpoint: String,
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
    #[arg(
        long = "ethereum-beacon-rpc-timeout",
        env = "ETH_BEACON_RPC_TIMEOUT",
        default_value = "10"
    )]
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
