pub mod api_provider;

use anyhow::anyhow;
use api_provider::ApiProviderConnection;
use gclient::Result;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vft_client::traits::*;
use vft_manager_client::{traits::*, Order};

// The constant is intentionally duplicated since vara-runtime is too heavy dependency.
pub const UNITS: u128 = 1_000_000_000_000;

/// Asynchronously migrates balances from an old VFT contract to a new one.
///
/// This function performs a batched migration of token balances between two VFT contracts.
/// It ensures:
/// 1. Verifying the new contract has zero supply before migration
/// 2. Fetching balances in configurable batches
/// 3. Minting equivalent amounts in the new contract
///
/// # Parameters
/// - `connection`: remoting interface for contract interactions
/// - `gas_limit`: Gas limit for each contract call
/// - `size_batch`: Number of balances to process per batch
/// - `vft`: ActorId of the source VFT contract (old)
/// - `vft_new`: ActorId of the destination VFT contract (new). Provided `remoting` should
///   have account with mint-permission.
///
/// # Returns
/// - `Ok(())` if migration completes successfully
/// - `Err(anyhow::Error)` on failure
pub async fn migrate_balances(
    mut connection: ApiProviderConnection,
    gear_suri: String,
    size_batch: u32,
    vft: ActorId,
    vft_new: ActorId,
) -> Result<()> {
    let gclient = connection.gclient();
    let gas_limit = gclient.block_gas_limit()?;
    let service_vft = vft_client::Vft::new(GClientRemoting::new(gclient));
    let supply = service_vft
        .total_supply()
        .with_gas_limit(gas_limit)
        .recv(vft_new)
        .await
        .map_err(|e| anyhow!("{e:?}"))?;
    if !supply.is_zero() {
        Err(anyhow!(
            "New VFT program should have zero supply but it is {supply}"
        ))?;
    }

    let mut cursor = 0;
    loop {
        let balances = get_balances(&mut connection, cursor, size_batch, gas_limit, vft).await?;
        let len = balances.len();
        log::debug!("migrate_balances: read {len} item(s) from {cursor}");

        cursor += size_batch;

        mint(&mut connection, &gear_suri, balances, gas_limit, vft_new).await?;
        log::debug!("migrate_balances: successfully send {len} item(s)");

        if (len as u32) < size_batch {
            break Ok(());
        }
    }
}

async fn get_balances(
    connection: &mut ApiProviderConnection,
    cursor: u32,
    size_batch: u32,
    gas_limit: u64,
    vft: ActorId,
) -> Result<Vec<(ActorId, U256)>> {
    loop {
        let service_extension =
            vft_client::VftExtension::new(GClientRemoting::new(connection.gclient()));
        match service_extension
            .balances(cursor, size_batch)
            .with_gas_limit(gas_limit)
            .recv(vft)
            .await
        {
            Ok(balances) => break Ok(balances),
            Err(e) => log::error!(
                r#"Failed to get balances (cursor = {cursor}, size_batch = {size_batch}): "{e:?}"#
            ),
        }

        connection.reconnect().await?;
    }
}

async fn mint(
    connection: &mut ApiProviderConnection,
    gear_suri: &str,
    balances: Vec<(ActorId, U256)>,
    gas_limit: u64,
    vft_new: ActorId,
) -> Result<()> {
    loop {
        let mut service_admin =
            vft_client::VftAdmin::new(GClientRemoting::new(connection.gclient_client(gear_suri)?));
        match service_admin
            .mint_list(balances.clone())
            .with_gas_limit(gas_limit)
            .send_recv(vft_new)
            .await
        {
            Ok(_) => break,
            Err(e) => log::error!(r#"Failed to mint "{balances:?}": "{e:?}""#),
        }

        connection.reconnect().await?;
    }

    Ok(())
}

pub async fn migrate_transactions(
    gas_limit: u64,
    size_batch: u32,
    remoting: GClientRemoting,
    vft_manager: ActorId,
    remoting_new: GClientRemoting,
    vft_manager_new: ActorId,
) -> Result<()> {
    let service = vft_manager_client::VftManager::new(remoting);
    let mut service_new = vft_manager_client::VftManager::new(remoting_new);
    let mut cursor = 0;
    loop {
        let transactions = service
            .transactions(Order::Direct, cursor, size_batch)
            .with_gas_limit(gas_limit)
            .recv(vft_manager)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;
        let len = transactions.len();

        log::debug!("migrate_transactions: read {len} item(s) from {cursor}");

        cursor += size_batch;

        service_new
            .insert_transactions(transactions)
            .with_gas_limit(gas_limit)
            .send_recv(vft_manager_new)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;

        log::debug!("migrate_transactions: successfully send {len} item(s)");

        if (len as u32) < size_batch {
            break Ok(());
        }
    }
}
