use alloy::{
    contract::Error as ContractError,
    network::Ethereum,
    primitives::Address,
    providers::{DynProvider, Provider, ProviderBuilder},
    sol,
};
use clap::Parser;
use gprimitives::ActorId;
use serde::{Deserialize, Serialize};
use std::fs;

mod cli;
mod error;

use cli::*;
use error::*;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Deployment {
    governance_admin: Address,
    governance_pauser: Address,
    wrapped_vara: Address,
    message_queue: Address,
    #[serde(rename = "ERC20Manager")]
    erc20_manager: Address,
}

sol! {
    #[sol(rpc)]
    interface IGovernance {
        function governance() external view returns (bytes32);
    }
}

async fn query_governance(
    governance: Address,
    provider: &DynProvider<Ethereum>,
) -> Result<ActorId, ContractError> {
    Ok(IGovernance::new(governance, provider)
        .governance()
        .call()
        .await?
        .0
        .into())
}

#[derive(Debug)]
struct GovernanceInfo {
    governance_admin: ActorId,
    governance_pauser: ActorId,
}

#[derive(Debug, Clone, Copy)]
enum GovernanceType {
    Admin,
    Pauser,
}

#[derive(Debug)]
struct Message {
    source: ActorId,
    destination: Address,
    payload: Vec<u8>,
}

fn change_governance(
    governance_info: GovernanceInfo,
    deployment: Deployment,
    governance_type: GovernanceType,
    new_governance: ActorId,
) -> Message {
    const CHANGE_GOVERNANCE: u8 = 0x00;

    let (source, destination) = match governance_type {
        GovernanceType::Admin => (
            governance_info.governance_admin,
            deployment.governance_admin,
        ),
        GovernanceType::Pauser => (
            governance_info.governance_pauser,
            deployment.governance_pauser,
        ),
    };
    let payload = [&[CHANGE_GOVERNANCE], new_governance.as_ref()].concat();

    Message {
        source,
        destination,
        payload,
    }
}

fn pause_proxy(
    governance_info: GovernanceInfo,
    deployment: Deployment,
    governance_type: GovernanceType,
    proxy: ProxyType,
) -> Message {
    const PAUSE_PROXY: u8 = 0x01;

    let (source, destination) = match governance_type {
        GovernanceType::Admin => (
            governance_info.governance_admin,
            deployment.governance_admin,
        ),
        GovernanceType::Pauser => (
            governance_info.governance_pauser,
            deployment.governance_pauser,
        ),
    };
    let proxy_address = match proxy {
        ProxyType::WrappedVara => deployment.wrapped_vara,
        ProxyType::MessageQueue => deployment.message_queue,
        ProxyType::ERC20Manager => deployment.erc20_manager,
    };
    let payload = [&[PAUSE_PROXY], proxy_address.as_slice()].concat();

    Message {
        source,
        destination,
        payload,
    }
}

fn unpause_proxy(
    governance_info: GovernanceInfo,
    deployment: Deployment,
    governance_type: GovernanceType,
    proxy: ProxyType,
) -> Message {
    const UNPAUSE_PROXY: u8 = 0x02;

    let (source, destination) = match governance_type {
        GovernanceType::Admin => (
            governance_info.governance_admin,
            deployment.governance_admin,
        ),
        GovernanceType::Pauser => (
            governance_info.governance_pauser,
            deployment.governance_pauser,
        ),
    };
    let proxy_address = match proxy {
        ProxyType::WrappedVara => deployment.wrapped_vara,
        ProxyType::MessageQueue => deployment.message_queue,
        ProxyType::ERC20Manager => deployment.erc20_manager,
    };
    let payload = [&[UNPAUSE_PROXY], proxy_address.as_slice()].concat();

    Message {
        source,
        destination,
        payload,
    }
}

fn upgrade_proxy(
    governance_info: GovernanceInfo,
    deployment: Deployment,
    proxy: ProxyType,
    new_implementation: Address,
    data: Data,
) -> Message {
    const UPGRADE_PROXY: u8 = 0x03;

    let (source, destination) = (
        governance_info.governance_admin,
        deployment.governance_admin,
    );
    let proxy_address = match proxy {
        ProxyType::WrappedVara => deployment.wrapped_vara,
        ProxyType::MessageQueue => deployment.message_queue,
        ProxyType::ERC20Manager => deployment.erc20_manager,
    };
    let payload = [
        &[UPGRADE_PROXY],
        proxy_address.as_slice(),
        new_implementation.as_slice(),
        &data,
    ]
    .concat();

    Message {
        source,
        destination,
        payload,
    }
}

fn add_vft_manager(
    governance_info: GovernanceInfo,
    deployment: Deployment,
    vft_manager: ActorId,
) -> Message {
    const ADD_VFT_MANAGER: u8 = 0x00;

    let (source, destination) = (governance_info.governance_admin, deployment.erc20_manager);
    let payload = [&[ADD_VFT_MANAGER], vft_manager.as_ref()].concat();

    Message {
        source,
        destination,
        payload,
    }
}

fn register_ethereum_token(
    governance_info: GovernanceInfo,
    deployment: Deployment,
    token: Address,
) -> Message {
    const REGISTER_ETHEREUM_TOKEN: u8 = 0x01;

    let (source, destination) = (governance_info.governance_admin, deployment.erc20_manager);
    let payload = [&[REGISTER_ETHEREUM_TOKEN], token.as_slice()].concat();

    Message {
        source,
        destination,
        payload,
    }
}

fn register_gear_token(
    governance_info: GovernanceInfo,
    deployment: Deployment,
    token_name: LimitedString,
    token_symbol: LimitedString,
    token_decimals: u8,
) -> Message {
    const REGISTER_GEAR_TOKEN: u8 = 0x02;

    let (source, destination) = (governance_info.governance_admin, deployment.erc20_manager);

    let mut token_name_raw = [0; 32];
    token_name_raw[0] = token_name.len() as u8;
    token_name_raw[1..(1 + token_name.len())].copy_from_slice(token_name.as_bytes());

    let mut token_symbol_raw = [0; 32];
    token_symbol_raw[0] = token_symbol.len() as u8;
    token_symbol_raw[1..(1 + token_symbol.len())].copy_from_slice(token_symbol.as_bytes());

    let payload = [
        &[REGISTER_GEAR_TOKEN],
        &token_name_raw[..],
        &token_symbol_raw[..],
        &[token_decimals],
    ]
    .concat();

    Message {
        source,
        destination,
        payload,
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let Cli { rpc_url, command } = Cli::parse();

    let deployment: Deployment = toml::from_str(&fs::read_to_string("deployment.toml")?)?;

    let provider: DynProvider<Ethereum> =
        ProviderBuilder::default().connect(&rpc_url).await?.erased();

    let governance_admin = query_governance(deployment.governance_admin, &provider).await?;
    let governance_pauser = query_governance(deployment.governance_pauser, &provider).await?;

    let governance_info = GovernanceInfo {
        governance_admin,
        governance_pauser,
    };

    let Message {
        source,
        destination,
        payload,
    } = match command {
        DestinationCommand::GovernanceAdmin { command } => match command {
            GovernanceAdminCommand::ChangeGovernance { new_governance } => change_governance(
                governance_info,
                deployment,
                GovernanceType::Admin,
                new_governance,
            ),
            GovernanceAdminCommand::PauseProxy { proxy } => {
                pause_proxy(governance_info, deployment, GovernanceType::Admin, proxy)
            }
            GovernanceAdminCommand::UnpauseProxy { proxy } => {
                unpause_proxy(governance_info, deployment, GovernanceType::Admin, proxy)
            }
            GovernanceAdminCommand::UpgradeProxy {
                proxy,
                new_implementation,
                data,
            } => upgrade_proxy(governance_info, deployment, proxy, new_implementation, data),
        },
        DestinationCommand::GovernancePauser { command } => match command {
            GovernancePauserCommand::ChangeGovernance { new_governance } => change_governance(
                governance_info,
                deployment,
                GovernanceType::Pauser,
                new_governance,
            ),
            GovernancePauserCommand::PauseProxy { proxy } => {
                pause_proxy(governance_info, deployment, GovernanceType::Pauser, proxy)
            }
            GovernancePauserCommand::UnpauseProxy { proxy } => {
                unpause_proxy(governance_info, deployment, GovernanceType::Pauser, proxy)
            }
        },
        DestinationCommand::ERC20Manager { command } => match command {
            ERC20ManagerCommand::AddVftManager { vft_manager } => {
                add_vft_manager(governance_info, deployment, vft_manager)
            }
            ERC20ManagerCommand::RegisterEthereumToken { token } => {
                register_ethereum_token(governance_info, deployment, token)
            }
            ERC20ManagerCommand::RegisterGearToken {
                token_name,
                token_symbol,
                token_decimals,
            } => register_gear_token(
                governance_info,
                deployment,
                token_name,
                token_symbol,
                token_decimals,
            ),
        },
    };

    println!("source: {source}");
    println!("destination: {destination}");
    println!("payload: 0x{}", hex::encode(payload));

    Ok(())
}
