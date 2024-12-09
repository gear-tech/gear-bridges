use clap::{Args, Parser, Subcommand};

const DEFAULT_ETH_BEACON_RPC: &str = "http://localhost:50000";
const DEFAULT_ETH_RPC: &str = "http://localhost:8545";
const DEFAULT_PROMETHEUS_ENDPOINT: &str = "0.0.0.0:9090";
const DEFAULT_VARA_SURI: &str = "//Alice";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommands,
}

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
pub enum CliCommands {
    /// Start service constantly relaying messages to ethereum
    #[clap(visible_alias("rr"))]
    RelayMerkleRoots(RelayMerkleRootsArgs),
    /// Relay message to ethereum
    #[clap(visible_alias("rm"))]
    RelayMessages(RelayMessagesArgs),
    /// Start service constantly relaying Ethereum checkpoints to the Vara program
    RelayCheckpoints(RelayCheckpointsArgs),
    /// Relay the ERC20 tokens to the Vara network
    RelayErc20(RelayErc20Args),
    /// Kill switch relayer
    #[clap(visible_alias("ks"))]
    KillSwitch(KillSwitchArgs),
}

#[derive(Args)]
pub struct RelayMessagesArgs {
    #[clap(flatten)]
    pub vara_args: VaraArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
    /// Block number to start relaying from. If not specified equals to the latest finalized block
    #[arg(long = "from-block")]
    pub from_block: Option<u32>,
    /// Address of bridging payment program (if not specified, relayer will relay all messages)
    #[arg(long = "bridging-payment-address", env = "BRIDGING_PAYMENT_ADDRESS")]
    pub bridging_payment_address: Option<String>,
}

#[derive(Args)]
pub struct RelayMerkleRootsArgs {
    #[clap(flatten)]
    pub vara_args: VaraArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,
    #[clap(flatten)]
    pub genesis_config_args: GenesisConfigArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
    #[clap(flatten)]
    pub proof_storage_args: ProofStorageArgs,
}

#[derive(Args)]
pub struct KillSwitchArgs {
    #[clap(flatten)]
    pub vara_args: VaraArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,
    #[clap(flatten)]
    pub genesis_config_args: GenesisConfigArgs,
    /// Eth block number to start kill switch relayer read events from. If not specified equals to the latest finalized block
    #[arg(long = "from-eth-block")]
    pub from_eth_block: Option<u64>,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
    #[clap(flatten)]
    pub proof_storage_args: ProofStorageArgs,
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
    #[arg(
        long = "ethereum-endpoint",
        default_value = DEFAULT_ETH_RPC,
        env = "ETH_RPC"
    )]
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
    #[arg(
        long = "ethereum-beacon-rpc",
        default_value = DEFAULT_ETH_BEACON_RPC,
        env = "ETH_BEACON_RPC"
    )]
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
        default_value = DEFAULT_PROMETHEUS_ENDPOINT,
        env = "PROMETHEUS_ENDPOINT"
    )]
    pub endpoint: String,
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
pub struct RelayCheckpointsArgs {
    /// Specify ProgramId of the Checkpoint-light-client program
    #[arg(long, env = "CHECKPOINT_LIGHT_CLIENT_ADDRESS")]
    pub program_id: String,

    /// Specify the endpoint providing Beacon API
    #[arg(long, env = "BEACON_ENDPOINT")]
    pub beacon_endpoint: String,

    /// Specify the timeout in seconds for requests to the Beacon API endpoint
    #[arg(long, default_value = "120", env = "BEACON_TIMEOUT")]
    pub beacon_timeout: u64,

    #[clap(flatten)]
    pub vara_args: VaraArgs,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = DEFAULT_VARA_SURI, env = "VARA_SURI")]
    pub vara_suri: String,

    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
}

#[derive(Args)]
pub struct RelayErc20Args {
    #[clap(flatten)]
    pub common: RelayErc20ArgsCommon,

    #[command(subcommand)]
    pub command: RelayErc20Commands,
}

#[derive(Subcommand)]
pub enum RelayErc20Commands {
    /// Relay all the transactions
    AllTokenTransfers {
        /// Address of the ERC20Treasury contract on ethereum
        #[arg(long = "erc20-treasury-address", env = "ERC20_TREASURY_ADDRESS")]
        erc20_treasury_address: String,
    },
    /// Relay only transactions sent to BridgingPayment
    PaidTokenTransfers {
        /// Address of the BridgingPayment contract on ethereum
        #[arg(long = "bridging-payment-address", env = "BRIDGING_PAYMENT_ADDRESS")]
        bridging_payment_address: String,
    },
}

#[derive(Args)]
pub struct RelayErc20ArgsCommon {
    /// Address of the checkpoint-light-client program on gear
    #[arg(
        long = "checkpoint-light-client-address",
        env = "CHECKPOINT_LIGHT_CLIENT_ADDRESS"
    )]
    pub checkpoint_light_client_address: String,

    #[arg(long = "historical-proxy-address", env = "HISTORICAL_PROXY_ADDRESS")]
    pub historical_proxy_address: String,
    #[arg(long = "vft-manager-address", env = "VFT_MANAGER_ADDRESS")]
    pub vft_manager_address: String,
    #[clap(flatten)]
    pub vara_args: VaraArgs,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = DEFAULT_VARA_SURI, env = "VARA_SURI")]
    pub vara_suri: String,

    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,

    #[clap(flatten)]
    pub beacon_rpc: BeaconRpcArgs,

    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
}
