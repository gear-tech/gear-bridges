use anyhow::anyhow;
use gclient::Result;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vft_client::traits::*;
use vft_manager_client::{traits::*, Order};

/// Asynchronously migrates balances from an old VFT contract to a new one.
///
/// This function performs a batched migration of token balances between two VFT contracts.
/// It ensures:
/// 1. Verifying the new contract has zero supply before migration
/// 2. Fetching balances in configurable batches
/// 3. Minting equivalent amounts in the new contract
///
/// # Parameters
/// - `remoting`: remoting interface for contract interactions
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
    remoting: GClientRemoting,
    gas_limit: u64,
    size_batch: u32,
    vft: ActorId,
    vft_new: ActorId,
) -> Result<()> {
    let service_vft = vft_client::Vft::new(remoting.clone());
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

    let service_extension = vft_client::VftExtension::new(remoting.clone());
    let mut service_admin = vft_client::VftAdmin::new(remoting);
    let mut cursor = 0;
    loop {
        let balances = service_extension
            .balances(cursor, size_batch)
            .with_gas_limit(gas_limit)
            .recv(vft)
            .await
            .map_err(|e| anyhow!("{e:?}"))?;

        cursor += size_batch;

        let len = balances.len();
        for (account, balance) in balances {
            service_admin
                .mint(account, balance)
                .with_gas_limit(gas_limit)
                .send_recv(vft_new)
                .await
                .map_err(|e| anyhow!("{e:?}"))?;
        }

        if (len as u32) < size_batch {
            break Ok(());
        }
    }
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
