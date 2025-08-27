use alloy::primitives::Address;
use clap::{Parser, Subcommand, ValueEnum};
use gprimitives::ActorId;
use std::{ops::Deref, str::FromStr};
use thiserror::Error;

/// Governance tools (`deployment.toml` file is required)
#[derive(Debug, Parser)]
pub struct Cli {
    /// Ethereum RPC URL
    #[arg(short, long)]
    pub rpc_url: String,

    /// Destination
    #[command(subcommand)]
    pub command: DestinationCommand,
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "verbatim")]
pub enum DestinationCommand {
    /// GovernanceAdmin actions
    GovernanceAdmin {
        #[command(subcommand)]
        command: GovernanceAdminCommand,
    },
    /// GovernancePauser actions
    GovernancePauser {
        #[command(subcommand)]
        command: GovernancePauserCommand,
    },
    /// ERC20Manager actions
    ERC20Manager {
        #[command(subcommand)]
        command: ERC20ManagerCommand,
    },
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "verbatim")]
pub enum GovernanceAdminCommand {
    /// Change governance address to `new_governance`
    ChangeGovernance {
        /// New governance address in 0x-prefixed hex or SS58 format (Vara Network)
        new_governance: ActorId,
    },
    /// Pause `proxy`
    PauseProxy {
        /// Which proxy to pause
        proxy: ProxyType,
    },
    /// Unpause `proxy`
    UnpauseProxy {
        /// Which proxy to unpause
        proxy: ProxyType,
    },
    /// Upgrade `proxy` to `new_implementation` and call `data` on it
    UpgradeProxy {
        /// Which proxy to upgrade
        proxy: ProxyType,
        /// New implementation address in 0x-prefixed hex format (Ethereum)
        new_implementation: Address,
        /// Hex-encoded call data with 0x prefix to call on new implementation after upgrade
        /// (use `0x` for no call)
        data: Data,
    },
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "verbatim")]
pub enum GovernancePauserCommand {
    /// Change governance address to `new_governance`
    ChangeGovernance {
        /// New governance address in 0x-prefixed hex or SS58 format (Vara Network)
        new_governance: ActorId,
    },
    /// Pause `proxy`
    PauseProxy {
        /// Which proxy to pause
        proxy: ProxyType,
    },
    /// Unpause `proxy`
    UnpauseProxy {
        /// Which proxy to unpause
        proxy: ProxyType,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "verbatim")]
pub enum ProxyType {
    /// MessageQueue
    MessageQueue,
    /// ERC20Manager
    ERC20Manager,
}

#[derive(Debug, Clone)]
pub struct Data(Vec<u8>);

impl Deref for Data {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Error, Debug)]
#[error("Invalid hex string")]
pub struct InvalidHexString;

impl FromStr for Data {
    type Err = InvalidHexString;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let data = s
            .strip_prefix("0x")
            .and_then(|s| hex::decode(s).ok())
            .ok_or(InvalidHexString)?;
        Ok(Self(data))
    }
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "verbatim")]
pub enum ERC20ManagerCommand {
    /// Add new VFT manager
    AddVftManager {
        /// VFT manager address in 0x-prefixed hex or SS58 format (Vara Network)
        vft_manager: ActorId,
    },
    /// Register Ethereum token
    RegisterEthereumToken {
        /// Token address in 0x-prefixed hex format (Ethereum)
        token: Address,
    },
    /// Register Gear token
    RegisterGearToken {
        /// Token name (1-31 characters)
        token_name: LimitedString,
        /// Token symbol (1-31 characters)
        token_symbol: LimitedString,
        /// Token decimals (0-255)
        token_decimals: u8,
    },
}

#[derive(Debug, Clone)]
pub struct LimitedString(String);

impl Deref for LimitedString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Error, Debug)]
#[error("String length must be between 1 and 31 characters")]
pub struct InvalidStringLength;

impl FromStr for LimitedString {
    type Err = InvalidStringLength;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        (1..=31)
            .contains(&s.len())
            .then(|| Self(s.into()))
            .ok_or(InvalidStringLength)
    }
}
