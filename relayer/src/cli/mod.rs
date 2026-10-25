use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

mod common;

pub use common::{
    BeaconRpcArgs, EthereumArgs, EthereumKillSwitchArgs, EthereumSignerArgs, GearArgs,
    GearSignerArgs, PrometheusArgs,
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
    /// Serve read-only, Beacon-authenticated Ethereum evidence over JSON-lines.
    EthereumEvidence(crate::ethereum_evidence::Args),

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

    /// Start queue cleaner
    QueueCleaner(QueueCleanerArgs),

    /// Fetch relayed merkle roots to Ethereum
    FetchMerkleRoots(FetchMerkleRootsArgs),
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
    pub lane: common::LaneApprovalArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,

    #[arg(long, help = format!("How many confirmations wait for relayed merkle roots on Ethereum. Default: {DEFAULT_COUNT_CONFIRMATIONS}"))]
    pub confirmations_merkle_root: Option<u64>,

    #[arg(long, help = format!("How many confirmations wait for message transaction on Ethereum. Default: {DEFAULT_COUNT_CONFIRMATIONS}"))]
    pub confirmations_status: Option<u64>,

    #[arg(
        long,
        help = format!("Specify which addresses will not be required to pay fees for bridging. Default: bridgeAdmin and bridgePauser from chain genesis config"),
        value_parser = parse_fee_payers,
    )]
    pub no_fee: Option<FeePayers>,

    #[arg(long = "storage-path", env = "GEAR_ETH_TX_STORAGE_PATH")]
    pub storage_path: String,

    #[arg(long = "governance-admin", env = "GEAR_GOVERNANCE_ADMIN")]
    pub governance_admin: String,
    #[arg(long = "governance-pauser", env = "GEAR_GOVERNANCE_PAUSER")]
    pub governance_pauser: String,

    /// Storage path for Ethereum blocks.
    #[arg(long = "ethereum-blocks", env = "ETHEREUM_BLOCKS")]
    pub ethereum_blocks: Option<String>,
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

        /// Authorization token for web-server
        #[arg(long, env)]
        web_server_token: String,

        /// Socket address for web-server
        #[arg(long, env, default_value = "127.0.0.1:8443")]
        web_server_address: String,
    },
}

#[derive(Args)]
pub struct EthGearTokensArgs {
    #[command(subcommand)]
    pub command: EthGearTokensCommands,

    #[arg(long = "vft-manager-address", env = "VFT_MANAGER_ADDRESS")]
    pub vft_manager_address: String,

    #[clap(flatten)]
    pub gear_args: GearSignerArgs,

    /// Address of the ethereum endpoint
    #[arg(long, env)]
    pub ethereum_rpc: String,

    #[clap(flatten)]
    pub finality: cli_utils::EthereumFinalityArgs,
    #[clap(flatten)]
    pub identity: cli_utils::EthereumChainIdentityArgs,

    #[clap(flatten)]
    pub beacon_rpc: BeaconRpcArgs,
    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,

    /// Storage path for transactions. Relayer will save transaction
    /// status in that directory.
    #[arg(long = "storage-path", env = "ETH_GEAR_TX_STORAGE_PATH")]
    pub storage_path: String,

    /// Dedicated durable Ethereum discovery journal; never share between lanes.
    #[arg(long = "ethereum-blocks", env = "ETHEREUM_BLOCKS")]
    pub ethereum_blocks: String,

    /// Immutable first Ethereum block to scan, normally the manager/payment deployment block.
    #[arg(long, env)]
    pub ethereum_start_block: u64,

    /// Immutable predecessor seal for a replacement inbound deployment.
    #[arg(long, requires = "successor_manager_code_id")]
    pub cutover_seal: Option<PathBuf>,

    /// Independently approved replacement CodeId (never inferred from the live receiver).
    #[arg(long, requires = "cutover_seal")]
    pub successor_manager_code_id: Option<String>,
    #[clap(flatten)]
    pub native_wrapper: common::NativeWrapperApprovalArgs,
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

        /// Authorization token for web-server
        #[arg(long, env)]
        web_server_token: String,

        /// Socket address for web-server
        #[arg(long, env, default_value = "127.0.0.1:8443")]
        web_server_address: String,
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

    /// Private durable directory for this one source message; reuse it on restart.
    #[arg(long)]
    pub storage: PathBuf,

    /// Ethereum block number to start listening for merkle roots from. If not specified equals to the latest finalized block
    #[arg(long = "from-eth-block")]
    pub from_eth_block: Option<u64>,

    #[clap(flatten)]
    pub gear_args: GearArgs,
    #[clap(flatten)]
    pub ethereum_args: EthereumSignerArgs,
    #[clap(flatten)]
    pub lane: common::LaneApprovalArgs,

    #[arg(long, help = format!("How many confirmations wait for message transaction on Ethereum. Default: {DEFAULT_COUNT_CONFIRMATIONS}"))]
    pub confirmations_status: Option<u64>,
    #[arg(long, help = "Governance admin address")]
    pub governance_admin: String,
    #[arg(long, help = "Governance pauser address")]
    pub governance_pauser: String,
}

#[derive(Args)]
pub struct EthGearManualArgs {
    /// Transaction hash of the target message
    #[arg(long, short = 't')]
    pub tx_hash: String,

    /// Private durable directory for this one original Ethereum transaction; reuse it on restart.
    #[arg(long)]
    pub storage: PathBuf,

    /// Immutable predecessor seal required when manually relaying to a successor.
    #[arg(long, requires = "successor_manager_code_id")]
    pub cutover_seal: Option<PathBuf>,
    #[arg(long, requires = "cutover_seal")]
    pub successor_manager_code_id: Option<String>,
    #[clap(flatten)]
    pub native_wrapper: common::NativeWrapperApprovalArgs,

    /// ProgramId of the checkpoint-light-client program
    #[arg(long = "checkpoint-light-client")]
    pub checkpoint_light_client: Option<String>,

    /// ProgramId of the historical-proxy program
    #[arg(long = "historical-proxy")]
    pub historical_proxy: Option<String>,

    /// ProgramId of the program that will receive target message
    #[arg(long = "receiver-program")]
    pub receiver_program: String,

    /// Route of the function that will be called on receiver-program. If not specified then `vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE` is used
    #[arg(long = "receiver-route")]
    pub receiver_route: Option<String>,

    #[clap(flatten)]
    pub gear_args: GearSignerArgs,

    /// Address of the ethereum endpoint
    #[arg(long, env)]
    pub ethereum_rpc: String,

    #[clap(flatten)]
    pub finality: cli_utils::EthereumFinalityArgs,
    #[clap(flatten)]
    pub identity: cli_utils::EthereumChainIdentityArgs,

    #[clap(flatten)]
    pub beacon_args: BeaconRpcArgs,
}

#[derive(Args)]
pub struct KillSwitchArgs {
    /// Eth block number to start kill switch relayer read events from. If not specified equals to the latest finalized block
    #[arg(long = "from-eth-block")]
    pub from_eth_block: Option<u64>,

    /// Durable finality history owned exclusively by this observer.
    #[arg(long)]
    pub ethereum_finality_dir: PathBuf,

    #[clap(flatten)]
    pub gear_args: GearArgs,

    #[clap(flatten)]
    pub ethereum_args: EthereumKillSwitchArgs,

    #[clap(flatten)]
    pub prometheus_args: PrometheusArgs,
}

#[derive(Args)]
pub struct QueueCleanerArgs {
    #[clap(flatten)]
    pub gear_args: GearArgs,

    #[arg(long = "suri", env = "GEAR_SURI")]
    pub suri: String,

    #[arg(
        long,
        help = "Delay in seconds before processing an overflowed queue event",
        default_value = "30"
    )]
    pub delay: u64,
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

#[derive(Debug, Clone)]
pub enum FeePayers {
    /// User explicitly requests for all fees to be paid.
    All,
    /// Accounts which are excluded from fee payment.
    ExcludedIds(Vec<String>),
}

fn parse_fee_payers(s: &str) -> anyhow::Result<FeePayers> {
    if s.trim().eq_ignore_ascii_case("none") {
        return Ok(FeePayers::All);
    }
    let ids: Vec<String> = s.split(',').map(String::from).collect();
    if ids.is_empty() {
        Err(anyhow::anyhow!("Fee payers cannot be empty"))
    } else {
        Ok(FeePayers::ExcludedIds(ids))
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    #[test]
    fn outbound_requires_approved_lane_and_beacon_authority() {
        let command = Cli::command();
        for name in ["gear-eth-tokens", "gear-eth-manual"] {
            let outbound = command.find_subcommand(name).unwrap();
            for flag in [
                "lane-approval",
                "lane-approval-sha256",
                "ethereum-finality-beacon-rpc",
                "ethereum-finality-bootstrap",
                "ethereum-finality-bootstrap-sha256",
                "ethereum-finality-store",
            ] {
                assert!(
                    outbound
                        .get_arguments()
                        .find(|arg| arg.get_long() == Some(flag))
                        .unwrap()
                        .is_required_set(),
                    "{name}: {flag} must be required"
                );
            }
            assert!(!outbound
                .get_arguments()
                .find(|arg| arg.get_long() == Some("ethereum-witness-rpc"))
                .unwrap()
                .is_required_set());
        }
        for name in ["eth-gear-tokens", "eth-gear-manual"] {
            assert!(!command
                .find_subcommand(name)
                .unwrap()
                .get_arguments()
                .any(|arg| arg.get_long() == Some("lane-approval")));
        }
    }

    #[test]
    fn evidence_child_has_no_signer_or_gear_arguments() {
        let command = Cli::command();
        let evidence = command.find_subcommand("ethereum-evidence").unwrap();
        let flags: Vec<_> = evidence
            .get_arguments()
            .filter_map(|arg| arg.get_long())
            .collect();
        assert_eq!(
            flags,
            [
                "ethereum-endpoint",
                "ethereum-finality-beacon-rpc",
                "ethereum-finality-bootstrap",
                "ethereum-finality-bootstrap-sha256",
                "ethereum-finality-store",
                "ethereum-finality-proof-source",
                "ethereum-evidence-proof-source",
            ]
        );
        for flag in [
            "ethereum-finality-proof-source",
            "ethereum-evidence-proof-source",
        ] {
            assert!(!evidence
                .get_arguments()
                .find(|arg| arg.get_long() == Some(flag))
                .unwrap()
                .is_required_set());
        }
    }
}
