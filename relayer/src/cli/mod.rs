use clap::{Args, Parser, Subcommand};

mod common;

pub use common::{
    BeaconRpcArgs, EthereumArgs, GenesisConfigArgs, PrometheusArgs, ProofStorageArgs, VaraArgs,
};

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
pub struct RelayCheckpointsArgs {
    /// Specify ProgramId of the Checkpoint-light-client program
    #[arg(long, env = "CHECKPOINT_LIGHT_CLIENT_ADDRESS")]
    pub program_id: String,

    #[clap(flatten)]
    pub beacon_args: BeaconRpcArgs,

    #[clap(flatten)]
    pub vara_args: VaraArgs,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, env = "VARA_SURI")]
    pub vara_suri: String,

    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
}

#[derive(Args)]
pub struct RelayErc20Args {
    #[command(subcommand)]
    pub command: RelayErc20Commands,

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
    #[arg(long, env = "VARA_SURI")]
    pub vara_suri: String,

    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,

    #[clap(flatten)]
    pub beacon_rpc: BeaconRpcArgs,

    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
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
