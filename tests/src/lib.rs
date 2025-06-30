use futures::StreamExt;
use gclient::{EventProcessor, GearApi};
use gear_core::ids::{prelude::*, ProgramId};
use sails_rs::calls::{ActionIo, RemotingAction};
use sails_rs::events::Listener;
use sails_rs::gclient::calls::GClientRemoting;
use sails_rs::prelude::*;
use sp_core::Pair as _;
use sp_core::{crypto::DEV_PHRASE, sr25519::Pair};
use sp_runtime::traits::IdentifyAccount;
use sp_runtime::traits::Verify;
use sp_runtime::MultiSignature;
use std::str::FromStr;
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::LazyLock,
};
use tokio::sync::Mutex;
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

/// Mock endpoint `A` for testing purposes.
///
/// You can use this type by constructing it with `ActorId` which is the id of the actor
/// you want to mock. Then use `recv` and `send` methods.
pub struct MockEndpoint {
    pub suri: String,
    pub actor_id: [u8; 32],
    pub route: &'static [u8],
}

impl MockEndpoint {
    pub fn new<A: ActionIo>(suri: String, actor_id: ActorId) -> Self {
        Self {
            route: A::ROUTE,
            suri,
            actor_id: actor_id.into_bytes(),
        }
    }

    //// Receives a message from the actor and returns the reply.
    ///
    /// Returns message sender and reply.
    pub async fn recv<P: Decode>(&self, api: &GearApi) -> gclient::Result<(ActorId, P)> {
        let api = api.clone().with(&self.suri).unwrap();
        let mut listener = api.subscribe().await?;

        listener
            .proc(|event| {
                if let gclient::Event::Gear(gclient::GearEvent::UserMessageSent {
                    message, ..
                }) = event
                {
                    if message.destination.0 == self.actor_id
                        && message.payload.0.starts_with(self.route)
                    {
                        let source = message.source;
                        let reply = message.destination;

                        let params_raw = &message.payload.0[self.route.len()..];
                        let params =
                            P::decode(&mut &params_raw[..]).expect("Failed to decode params");

                        Some((ActorId::new(source.0), params))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .await
    }

    pub async fn reply_to<R: Encode>(
        &self,
        api: &GearApi,
        source: ActorId,
        reply: R,
        gas_limit: u64,
        value: u128,
    ) -> gclient::Result<(MessageId, H256)> {
        let api = api.clone().with(&self.suri).unwrap();
        let mut bytes = Vec::with_capacity(reply.encoded_size() + self.route.len());

        bytes.extend_from_slice(self.route);
        reply.encode_to(&mut bytes);

        api.send_message_bytes(source, bytes, gas_limit, value)
            .await
    }
}
