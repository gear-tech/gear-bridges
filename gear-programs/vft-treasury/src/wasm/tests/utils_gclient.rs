/* gclient utils */
use gclient::EventProcessor;
use gclient::{EventListener, GearApi, Result};
use gear_core::ids::*;
use sails_rs::{Decode, Encode};
use sails_rs::{H160, U256};
use vft_treasury_app::services::{Config, InitConfig};

pub async fn upload_program(
    client: &gclient::GearApi,
    listener: &mut gclient::EventListener,
    code: &[u8],
    payload: Vec<u8>,
) -> gclient::Result<ProgramId> {
    let gas_limit = client
        .calculate_upload_gas(None, code.to_vec(), payload.clone(), 0, true)
        .await?
        .min_limit;
    println!("init gas {gas_limit:?}");
    let (message_id, program_id, _) = client
        .upload_program_bytes(
            code,
            gclient::now_micros().to_le_bytes(),
            payload,
            gas_limit,
            0,
        )
        .await?;
    assert!(listener.message_processed(message_id).await?.succeed());

    Ok(program_id)
}

pub fn decode<T: Decode>(payload: Vec<u8>) -> gclient::Result<T> {
    Ok(T::decode(&mut payload.as_slice())?)
}

async fn send_request_with_reply<R>(
    client: &GearApi,
    listener: &mut gclient::EventListener,
    destination: ActorId,
    service: &str,
    method: &str,
    arguments: impl Encode,
) -> Result<R>
where
    R: Decode,
{
    let payload = [service.encode(), method.encode(), arguments.encode()].concat();
    let gas_info = client
        .calculate_handle_gas(None, destination, payload.clone(), 0, true)
        .await?;

    let (message_id, _) = client
        .send_message_bytes(destination, payload, gas_info.min_limit, 0)
        .await?;

    let (_, raw_reply, _) = listener.reply_bytes_on(message_id).await?;

    let decoded_reply: (String, String, R) = match raw_reply {
        Ok(raw_reply) => decode(raw_reply)?,
        Err(e) => panic!("no reply: {:?}", e),
    };

    Ok(decoded_reply.2)
}

async fn send_request_with_reply_gas<R>(
    client: &GearApi,
    listener: &mut gclient::EventListener,
    destination: ActorId,
    service: &str,
    method: &str,
    arguments: impl Encode,
    gas: u64,
) -> Result<R>
where
    R: Decode,
{
    let payload = [service.encode(), method.encode(), arguments.encode()].concat();

    let (message_id, _) = client
        .send_message_bytes(destination, payload, gas, 0)
        .await?;

    let (_, raw_reply, _) = listener.reply_bytes_on(message_id).await?;

    let decoded_reply: (String, String, R) = match raw_reply {
        Ok(raw_reply) => decode(raw_reply)?,
        Err(e) => panic!("no reply: {:?}", e),
    };

    Ok(decoded_reply.2)
}
pub struct Vft(ProgramId);

impl Vft {
    pub async fn new(client: &GearApi, listener: &mut EventListener) -> Result<Self> {
        let payload = ["New".encode(), ("Token", "Token", 18).encode()].concat();

        let program_id = upload_program(
            client,
            listener,
            extended_vft_wasm::WASM_BINARY_OPT,
            payload,
        )
        .await?;
        println!("vft ID = {:?}", ProgramId::from(program_id));
        Ok(Self(program_id))
    }

    pub fn program_id(&self) -> ActorId {
        self.0
    }

    pub async fn balance_of(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        account: ActorId,
    ) -> Result<U256> {
        send_request_with_reply(client, listener, self.0, "Vft", "BalanceOf", account).await
    }

    pub async fn mint(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        account: ActorId,
        amount: U256,
    ) -> Result<bool> {
        send_request_with_reply(client, listener, self.0, "Vft", "Mint", (account, amount)).await
    }

    pub async fn approve(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        spender: ActorId,
        allowance: U256,
    ) -> Result<bool> {
        send_request_with_reply(
            client,
            listener,
            self.0,
            "Vft",
            "Approve",
            (spender, allowance),
        )
        .await
    }

    pub async fn allowance(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        owner: ActorId,
        spender: ActorId,
    ) -> Result<U256> {
        send_request_with_reply(
            client,
            listener,
            self.0,
            "Vft",
            "Allowance",
            (owner, spender),
        )
        .await
    }
}

pub struct VftTreasury(ProgramId);

impl VftTreasury {
    pub async fn new(client: &GearApi, listener: &mut EventListener) -> Result<Self> {
        let seed = *b"built/in";
        // a code based on what is in runtime/vara and gear-builtin pallete. Update
        // if the pallete or runtime are changed.
        // ActorWithId<3> is bridge builtin while `seed` comes from pallet-gear-builtin.
        let bridge_builtin_id: ProgramId =
            gear_core::ids::hash((seed, 3u64).encode().as_slice()).into();
        println!("bridge builtin id={:?}", bridge_builtin_id);
        let init_config = InitConfig::new(
            [2; 20].into(),
            bridge_builtin_id,
            44.into(),
            Config {
                gas_for_reply_deposit: 15_000_000_000,
                gas_for_transfer_to_eth_msg: 15_000_000_000,
                gas_for_transfer_tokens: 15_000_000_000,
                gas_to_send_request_to_builtin: 15_000_000_000,
                reply_timeout: 100,
            },
        );

        let payload = ["New".encode(), init_config.encode()].concat();

        let program_id = upload_program(
            client,
            listener,
            vft_treasury_wasm::WASM_BINARY_OPT,
            payload,
        )
        .await?;
        println!("treasury ID = {:?}", <ProgramId>::from(program_id));
        Ok(Self(program_id))
    }

    pub fn program_id(&self) -> ActorId {
        self.0
    }

    pub async fn deposit_tokens(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        gas: u64,
        vara_token_id: ActorId,
        from: ActorId,
        amount: U256,
        to: H160,
    ) -> Result<Result<(U256, H160), vft_treasury_app::services::error::Error>> {
        send_request_with_reply_gas(
            client,
            listener,
            self.0,
            "VftTreasury",
            "DepositTokens",
            (vara_token_id, from, amount, to),
            gas,
        )
        .await
    }

    pub async fn withdraw_tokens(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        gas: u64,
        ethereum_token_id: H160,
        recepient: ActorId,
        amount: U256,
    ) -> Result<Result<(), vft_treasury_app::services::error::Error>> {
        send_request_with_reply_gas(
            client,
            listener,
            self.0,
            "VftTreasury",
            "WithdrawTokens",
            (ethereum_token_id, recepient, amount),
            gas,
        )
        .await
    }

    pub async fn map_vara_to_eth_address(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        ethereum_token_id: H160,
        vara_token_id: ActorId,
    ) -> Result<Result<(), vft_treasury_app::services::error::Error>> {
        send_request_with_reply(
            client,
            listener,
            self.0,
            "VftTreasury",
            "MapVaraToEthAddress",
            (ethereum_token_id, vara_token_id),
        )
        .await
    }

    pub async fn vara_to_eth_addresses(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
    ) -> Result<Vec<(ActorId, H160)>> {
        send_request_with_reply(
            client,
            listener,
            self.0,
            "VftTreasury",
            "MapVaraToEthAddress",
            (),
        )
        .await
    }

    pub async fn update_ethereum_event_client_address(
        &self,
        client: &GearApi,
        listener: &mut EventListener,
        new_address: ActorId,
    ) -> Result<Result<(), vft_treasury_app::services::error::Error>> {
        send_request_with_reply(
            client,
            listener,
            self.0,
            "VftTreasury",
            "UpdateEthereumEventClientAddress",
            new_address,
        )
        .await
    }
}

pub trait ApiUtils {
    fn get_actor_id(&self) -> ActorId;
}

impl ApiUtils for GearApi {
    fn get_actor_id(&self) -> ActorId {
        ActorId::new(
            self.account_id()
                .encode()
                .try_into()
                .expect("Unexpected invalid account id length."),
        )
    }
}
