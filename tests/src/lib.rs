use std::{collections::hash_map::Entry, sync::LazyLock};

use alloy::primitives::map::{HashMap, HashSet};
use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, prelude::*};
use tokio::sync::Mutex;
use sp_core::crypto::DEV_PHRASE;
#[cfg(test)]
mod historical_proxy;

static LOCK: LazyLock<Mutex<(u32, std::collections::HashMap<&'static [u8], CodeId, std::hash::RandomState>)>> =
    LazyLock::new(|| Mutex::const_new((2_000, std::collections::HashMap::new())));

pub struct Connection {
    accounts: Vec<(GearApi, ActorId, [u8; 4])>,
    code_ids: Vec<CodeId>,
}

pub async fn connect_to_node(
    balances: &[u128],
    program: &str,
    binaries: &[&'static [u8]],
) -> Connection {
    let mut lock = LOCK.lock().await;

    let api = GearApi::dev().await.unwrap();

    let code_ids = {
        let mut res = vec![];

        for &binary in binaries {
            match lock.1.entry(binary) {
                Entry::Occupied(entry) => {
                    res.push(*entry.get());
                }

                Entry::Vacant(entry) => {
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
    for (i, &balance) in balances.iter().enumerate() {
        let salt = lock.0;
        lock.0 += 1;

        let suri = format!("{DEV_PHRASE}//{program}-{salt}");
        let api2 = GearApi::init_with(WSAddress::dev(), suri).await.unwrap();

        let account_id: &[u8; 32] = api2.account_id().as_ref();

        api.transfer_keep_alive((*account_id).into(), balance)
            .await
            .unwrap();
        let account_id = ActorId::from(*account_id);
        accounts.push((api2, account_id, salt.to_le_bytes()));
    }

    Connection { accounts, code_ids }
}
