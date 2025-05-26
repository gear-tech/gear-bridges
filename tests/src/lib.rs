use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::prelude::*;
use sp_core::Pair as _;
use sp_core::{crypto::DEV_PHRASE, sr25519::Pair};
use sp_runtime::traits::IdentifyAccount;
use sp_runtime::traits::Verify;
use sp_runtime::MultiSignature;
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::{atomic::AtomicU64, LazyLock},
};
use tokio::sync::Mutex;
#[cfg(test)]
mod checkpoint_light_client;
#[cfg(test)]
mod historical_proxy;
#[cfg(test)]
mod vft_manager;

type State = (u32, HashMap<&'static [u8], CodeId>);

static LOCK: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new((1_000, HashMap::new())));

pub const DEFAULT_BALANCE: u128 = 500_000_000_000_000;
pub struct Connection {
    pub api: GearApi,
    pub accounts: Vec<(ActorId, [u8; 4], String)>,
    pub code_ids: Vec<CodeId>,
    pub gas_limit: u64,
    pub salt: [u8; 4],
}

pub async fn connect_to_node(
    balances: &[u128],
    program: &str,
    binaries: &[&'static [u8]],
) -> Connection {

    let mut lock = LOCK.lock().await;
    let api = GearApi::dev().await.unwrap();

    println!(
        "({}-{}) nonce={}",
        program,
        lock.0,
        api.rpc_nonce().await.unwrap()
    );
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
                        .unwrap_or_else(|err| {
                            println!("Failed to upload code: {}", err);
                            CodeId::generate(binary)
                        });
                    entry.insert(code_id);
                    res.push(code_id);
                }
            }
        }

        res
    };

    let mut accounts = vec![];
    let origin = lock.0;
    let mut osalt = lock.0;
    lock.0 += 1_000;
    for &balance in balances.iter() {
        let salt = osalt;
        osalt += 1;
        let suri = format!("{DEV_PHRASE}//{program}-{salt}:");
        let pair = Pair::from_string(&suri, None).expect("Failed to create keypair from SURI");
        let account = <MultiSignature as Verify>::Signer::from(pair.public()).into_account();
        let account_id: &[u8; 32] = account.as_ref();
        let account_id = ActorId::from(*account_id);
        println!(
            "account {} with SURI={} and balance={}",
            account_id, suri, balance
        );
        api.transfer_keep_alive(account_id, balance).await.unwrap();

        accounts.push((account_id, salt.to_le_bytes(), suri));
    }

    Connection {
        api,
        accounts,
        code_ids,
        gas_limit,
        salt: origin.to_le_bytes(),
    }
}
