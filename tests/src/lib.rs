use std::{
    collections::{hash_map::Entry, HashMap},
    sync::LazyLock,
};

use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::prelude::*;
use sp_core::crypto::DEV_PHRASE;
use tokio::sync::Mutex;
#[cfg(test)]
mod historical_proxy;
#[cfg(test)]
mod vft_manager;

type State = (u32, HashMap<&'static [u8], CodeId>);

static LOCK: LazyLock<Mutex<State>> =
    LazyLock::new(|| Mutex::const_new((2_000, HashMap::new())));

pub const DEFAULT_BALANCE: u128 = 500_000_000_000_000;

pub struct Connection {
    pub api: GearApi,
    pub accounts: Vec<(GearApi, ActorId, [u8; 4], String)>,
    pub code_ids: Vec<CodeId>,
    pub gas_limit: u64,
    pub salt: [u8; 4]
}

pub async fn connect_to_node(
    balances: &[u128],
    program: &str,
    binaries: &[&'static [u8]],
) -> Connection {
    let mut lock = LOCK.lock().await;

    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();
    let code_ids = {
        let mut res = vec![];

        for &binary in binaries {
            match lock.1.entry(binary) {
                Entry::Occupied(entry) => {
                    println!("code {:p} already uploaded", binary);
                    res.push(*entry.get());
                }

                Entry::Vacant(entry) => {
                    println!("uploading code {:p}", binary);
                    let code_id = api
                        .upload_code(binary)
                        .await
                        .map(|(code_id, ..)| code_id)
                        .unwrap_or_else(|_| CodeId::generate(binary));
                    entry.insert(code_id);
                    res.push(code_id);
                }
            }
        }

        res
    };
    
    let mut accounts = vec![];
    let salt = lock.0;
    for &balance in balances.iter() {
        let salt = lock.0;
        lock.0 += 1;
        let suri = format!("{DEV_PHRASE}//{program}-{salt}:");
        let api2 = GearApi::init_with(WSAddress::dev(), suri.clone())
            .await
            .unwrap();
        
        let account_id: &[u8; 32] = api2.account_id().as_ref();
        println!(
            "account {} with SURI={} and balance={}",
            api2.account_id(),
            suri,
            balance
        );
        api.transfer_keep_alive((*account_id).into(), balance)
            .await
            .unwrap();
        let account_id = ActorId::from(*account_id);
        accounts.push((api2, account_id, salt.to_le_bytes(), suri));
    }

    Connection {
        api,
        accounts,
        code_ids,
        gas_limit,
        salt: salt.to_le_bytes(),
    }
}
