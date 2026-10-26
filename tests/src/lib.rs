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

static LOCK: LazyLock<Mutex<HashMap<&'static [u8], CodeId>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub const DEFAULT_BALANCE: u128 = 500_000_000_000_000;

pub struct Connection {
    pub api: GearApi,
    pub accounts: Vec<(ActorId, [u8; 16], String)>,
    pub code_ids: Vec<CodeId>,
    pub gas_limit: u64,
    pub salt: [u8; 16],
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
            match lock.entry(binary) {
                Entry::Occupied(entry) => {
                    println!("code {binary:p} already uploaded");
                    res.push(*entry.get());
                }

                Entry::Vacant(entry) => {
                    println!("uploading code {binary:p}");
                    let code_id = match api.upload_code(binary).await {
                        Ok((code_id, ..)) => code_id,
                        Err(err) => {
                            let code_id = CodeId::generate(binary);
                            let stored = api
                                .original_code_at(code_id, None)
                                .await
                                .unwrap_or_else(|_| std::panic!("Failed to upload code: {err}"));
                            assert_eq!(stored, binary, "existing code bytes differ");
                            code_id
                        }
                    };
                    entry.insert(code_id);
                    res.push(code_id);
                }
            }
        }

        res
    };

    let mut accounts = vec![];
    // Reuse the node without colliding with programs or balances from earlier runs.
    let origin = uuid::Uuid::now_v7();
    for (index, &balance) in balances.iter().enumerate() {
        let salt = if index == 0 {
            origin
        } else {
            uuid::Uuid::now_v7()
        };
        let suri = format!("{DEV_PHRASE}//{program}-{salt}");
        let pair = Pair::from_string(&suri, None).expect("Failed to create keypair from SURI");
        let account = <MultiSignature as Verify>::Signer::from(pair.public()).into_account();
        let account_id: &[u8; 32] = account.as_ref();
        let account_id = ActorId::from(*account_id);
        println!("account {account_id} with SURI={suri} and balance={balance}");
        api.transfer_keep_alive(account_id, balance).await.unwrap();

        accounts.push((account_id, salt.into_bytes(), suri));
    }

    Connection {
        api,
        accounts,
        code_ids,
        gas_limit,
        salt: origin.into_bytes(),
    }
}

pub fn create_receipt_rlp(
    erc20_manager_address: H160,
    from: H160,
    receiver: ActorId,
    token: H160,
    amount: U256,
) -> Vec<u8> {
    create_receipt_rlp_with_logs(erc20_manager_address, vec![(from, receiver, token, amount)])
}

pub fn create_receipt_rlp_with_logs(
    erc20_manager_address: H160,
    deposits: Vec<(H160, ActorId, H160, U256)>,
) -> Vec<u8> {
    let logs = deposits
        .into_iter()
        .map(|(from, receiver, token, amount)| {
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
            alloy_primitives::Log {
                address: erc20_manager_address.0.into(),
                data: Into::into(&event),
            }
        })
        .collect();
    let receipt = ReceiptWithBloom::from(Receipt {
        status: true.into(),
        cumulative_gas_used: 100_000u64,
        logs,
    });
    let receipt = ReceiptEnvelope::Eip2930(receipt);
    alloy::eips::eip2718::Encodable2718::encoded_2718(&receipt)
}
