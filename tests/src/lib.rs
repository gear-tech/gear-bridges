use alloy_consensus::{Receipt, ReceiptEnvelope, ReceiptWithBloom};
use gclient::GearApi;
use gear_core::ids::prelude::*;
use sails_rs::prelude::*;
use sp_core::{crypto::DEV_PHRASE, sr25519::Pair, Pair as _};
use sp_runtime::{
    traits::{IdentifyAccount, Verify},
    MultiSignature,
};
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::LazyLock,
};
use tokio::sync::Mutex;
use vft_manager_app::services::eth_abi::ERC20_MANAGER;

#[cfg(test)]
mod checkpoint_light_client;
#[cfg(test)]
mod historical_proxy;
#[cfg(test)]
mod relayer;
#[cfg(test)]
mod vft;
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
    let gas_limit = api.block_gas_limit().unwrap();
    let code_ids = {
        let mut res = vec![];

        for &binary in binaries {
            match lock.1.entry(binary) {
                Entry::Occupied(entry) => {
                    println!("code {binary:p} already uploaded");
                    res.push(*entry.get());
                }

                Entry::Vacant(entry) => {
                    println!("uploading code {binary:p}");
                    let code_id = api
                        .upload_code(binary)
                        .await
                        .map(|(code_id, ..)| code_id)
                        .unwrap_or_else(|err| {
                            println!("Failed to upload code: {err}");
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
    let mut salt_base = lock.0;
    lock.0 += balances.len() as u32;
    for &balance in balances.iter() {
        let salt = salt_base;
        salt_base += 1;
        let suri = format!("{DEV_PHRASE}//{program}-{salt}");
        let pair = Pair::from_string(&suri, None).expect("Failed to create keypair from SURI");
        let account = <MultiSignature as Verify>::Signer::from(pair.public()).into_account();
        let account_id: &[u8; 32] = account.as_ref();
        let account_id = ActorId::from(*account_id);
        println!("account {account_id} with SURI={suri} and balance={balance}");
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

pub fn create_receipt_rlp(
    erc20_manager_address: H160,
    from: H160,
    receiver: ActorId,
    token: H160,
    amount: U256,
) -> Vec<u8> {
    let event = ERC20_MANAGER::BridgingRequested {
        from: from.0.into(),
        to: receiver.into_bytes().into(),
        token: token.0.into(),
        amount: {
            let mut bytes = [0u8; 32];
            amount.to_little_endian(&mut bytes[..]);

            alloy_primitives::U256::from_le_bytes(bytes)
        },
    };

    let receipt = ReceiptWithBloom::from(Receipt {
        status: true.into(),
        cumulative_gas_used: 100_000u64,
        logs: vec![alloy_primitives::Log {
            address: erc20_manager_address.0.into(),
            data: Into::into(&event),
        }],
    });

    let receipt = ReceiptEnvelope::Eip2930(receipt);

    let mut receipt_rlp = vec![];
    alloy_rlp::Encodable::encode(&receipt, &mut receipt_rlp);

    receipt_rlp
}
