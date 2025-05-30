use clap::{Args, Parser, Subcommand};

mod common;

pub use common::{
    BeaconRpcArgs, EthereumArgs, EthereumSignerArgs, GearArgs, GearSignerArgs, GenesisConfigArgs,
    PrometheusArgs, ProofStorageArgs,
};

pub const DEFAULT_COUNT_CONFIRMATIONS: u64 = 8;

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
    /// Start core protocol gear to ethereum relayer
    GearEthCore(GearEthCoreArgs),
    /// Start core protocol ethereum to gear relayer
    EthGearCore(EthGearCoreArgs),

    /// Relay tokens from gear to ethereum
    GearEthTokens(GearEthTokensArgs),
    /// Relay tokens from ethereum to gear
    EthGearTokens(EthGearTokensArgs),

    /// Manually relay message from gear to ethereum
    GearEthManual(GearEthManualArgs),
    /// Manually relay message from ethereum to gear
    EthGearManual(EthGearManualArgs),

    /// Start kill switch relayer
    KillSwitch(KillSwitchArgs),

    /// Fetch relayed merkle roots to Ethereum
    FetchMerkleRoots(FetchMerkleRootsArgs),
}

#[derive(Args)]
pub struct GearEthCoreArgs {
    #[clap(flatten)]
    pub gear_args: GearArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumSignerArgs,
    #[clap(flatten)]
    pub genesis_config_args: GenesisConfigArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
    #[clap(flatten)]
    pub proof_storage_args: ProofStorageArgs,

    /// Authority set id to start relaying from. If not specified equals to one from the latest finalized block
    #[arg(long, env = "START_AUTHORITY_SET_ID")]
    pub start_authority_set_id: Option<u64>,
}

#[derive(Args)]
pub struct EthGearCoreArgs {
    /// ProgramId of the checkpoint-light-client program
    #[arg(long, env = "CHECKPOINT_LIGHT_CLIENT_ADDRESS")]
    pub program_id: String,
    #[clap(flatten)]
    pub beacon_args: BeaconRpcArgs,
    #[clap(flatten)]
    pub gear_args: GearSignerArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
    #[arg(
        long = "size-batch-multiplier",
        env = "ETH_SIZE_BATCH_MULTIPLIER",
        default_value = "30"
    )]
    pub size_batch_multiplier: u64,
}

#[derive(Args)]
pub struct GearEthTokensArgs {
    #[clap(subcommand)]
    pub command: GearEthTokensCommands,

    /// Block number to start relaying from. If not specified equals to the latest finalized block
    #[arg(long = "from-block")]
    pub from_block: Option<u32>,

    #[clap(flatten)]
    pub gear_args: GearArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumSignerArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,

    #[arg(long, help = format!("How many confirmations wait for relayed merkle roots on Ethereum. Default: {DEFAULT_COUNT_CONFIRMATIONS}"))]
    pub confirmations_merkle_root: Option<u64>,
}

#[derive(Subcommand)]
pub enum GearEthTokensCommands {
    /// Relay all the messages
    AllTokenTransfers,
    /// Relay only messages sent through bridging-payment
    PaidTokenTransfers {
        /// Address of the bridging-payment program
        #[arg(long = "bridging-payment-address", env = "BRIDGING_PAYMENT_ADDRESS")]
        bridging_payment_address: String,
    },
}

#[derive(Args)]
pub struct EthGearTokensArgs {
    #[command(subcommand)]
    pub command: EthGearTokensCommands,

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
    pub gear_args: GearSignerArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,
    #[clap(flatten)]
    pub beacon_rpc: BeaconRpcArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
}

#[derive(Subcommand)]
pub enum EthGearTokensCommands {
    /// Relay all the transactions
    AllTokenTransfers {
        /// Address of the ERC20Manager contract on ethereum
        #[arg(long = "erc20-manager-address", env = "ERC20_MANAGER_ADDRESS")]
        erc20_manager_address: String,
    },
    /// Relay only transactions sent to BridgingPayment
    PaidTokenTransfers {
        /// Address of the BridgingPayment contract on ethereum
        #[arg(long = "bridging-payment-address", env = "BRIDGING_PAYMENT_ADDRESS")]
        bridging_payment_address: String,
    },
}

#[derive(Args)]
pub struct GearEthManualArgs {
    /// Nonce of the target message
    #[arg(long = "message-nonce", short = 'n')]
    pub nonce: String,

    /// Block where target message was sent
    #[arg(long = "message-block", short = 'b')]
    pub block: u32,

    /// Ethereum block number to start listening for merkle roots from. If not specified equals to the latest finalized block
    #[arg(long = "from-eth-block")]
    pub from_eth_block: Option<u64>,

    #[clap(flatten)]
    pub gear_args: GearArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumSignerArgs,
}

#[derive(Args)]
pub struct EthGearManualArgs {
    /// Transaction hash of the target message
    #[arg(long = "tx-hash", short = 't')]
    pub tx_hash: String,

    /// Ethereum slot containing target message
    #[arg(long = "slot", short = 's')]
    pub slot: u64,

    /// ProgramId of the checkpoint-light-client program
    #[arg(long = "checkpoint-light-client")]
    pub checkpoint_light_client: String,

    /// ProgramId of the historical-proxy program
    #[arg(long = "historical-proxy")]
    pub historical_proxy: String,

    /// ProgramId of the program that will receive target message
    #[arg(long = "receiver-program")]
    pub receiver_program: String,

    /// Route of the function that will be called on receiver-program. If not specified then `vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE` is used
    #[arg(long = "receiver-route")]
    pub receiver_route: Option<String>,

    #[clap(flatten)]
    pub gear_args: GearSignerArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,
    #[clap(flatten)]
    pub beacon_args: BeaconRpcArgs,
}

#[derive(Args)]
pub struct KillSwitchArgs {
    /// Eth block number to start kill switch relayer read events from. If not specified equals to the latest finalized block
    #[arg(long = "from-eth-block")]
    pub from_eth_block: Option<u64>,

    #[clap(flatten)]
    pub gear_args: GearArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumSignerArgs,
    #[clap(flatten)]
    pub genesis_config_args: GenesisConfigArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
    #[clap(flatten)]
    pub proof_storage_args: ProofStorageArgs,
}

#[derive(Args)]
pub struct FetchMerkleRootsArgs {
    /// Ethereum block number to fetch merkle roots from
    #[arg(long)]
    pub from_eth_block: u64,

    #[clap(flatten)]
    pub ethereum_args: EthereumArgs,

    #[clap(flatten)]
    pub gear_args: GearArgs,
}
