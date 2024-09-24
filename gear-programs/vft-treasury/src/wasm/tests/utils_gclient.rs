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

pub fn gclient_decode<T: Decode>(payload: Vec<u8>) -> gclient::Result<T> {
    Ok(T::decode(&mut payload.as_slice())?)
}

macro_rules! impl_api {
    ($fn_name: ident, $service_name: expr, $query_name: expr,($($param_name:ident: $param_type:ty),*) -> $return_type: ty) => {
        #[allow(clippy::too_many_arguments)]
        pub async fn $fn_name(&self, api: &GearApi, listener: &mut EventListener, $($param_name: $param_type),*) -> Result<$return_type> {
            let query = [$service_name.encode(), $query_name.encode(), ($($param_name),*).encode()].concat();
            let gas_info = api
                .calculate_handle_gas(None, self.0.into(), query.clone(), 0, true)
                .await?;

            let (message_id, _) = api
                .send_message_bytes(self.0.into(), query.clone(), gas_info.min_limit, 0)
                .await?;

            let (_, raw_reply, _) = listener
                .reply_bytes_on(message_id)
                .await?;

            let decoded_reply: (String, String, $return_type) = match raw_reply {
                Ok(raw_reply) => gclient_decode(raw_reply)?,
                Err(e) => panic!("no reply: {:?}", e)
            };

            Ok(decoded_reply.2)
        }
    };

    ($fn_name: ident, $service_name: expr, $query_name: expr,($($param_name:ident: $param_type:ty),*)) => {
        #[allow(clippy::too_many_arguments)]
        pub async fn $fn_name(&self, api: &GearApi, listener: &mut EventListener, $($param_name: $param_type),*) -> Result<()> {
            let query = [$service_name.encode(), $query_name.encode(), ($($param_name),*).encode()].concat();
            let gas_info = api
                .calculate_handle_gas(None, self.0.into(), query.clone(), 0, true)
                .await?;

            let (message_id, _) = api
                .send_message_bytes(self.0.into(), query.clone(), gas_info.min_limit, 0)
                .await?;

            assert!(listener.message_processed(message_id).await?.succeed());

            Ok(())
        }
    };

    (manual gas $fn_name: ident, $service_name: expr, $query_name: expr,($($param_name:ident: $param_type:ty),*)) => {
        #[allow(clippy::too_many_arguments)]
        pub async fn $fn_name(&self, api: &GearApi, listener: &mut EventListener, gas: u64, $($param_name: $param_type),*) -> Result<()> {
            let query = [$service_name.encode(), $query_name.encode(), ($($param_name),*).encode()].concat();

            let (message_id, _) = api
                .send_message_bytes(self.0.into(), query.clone(), gas, 0)
                .await?;

            assert!(listener.message_processed(message_id).await?.succeed());

            Ok(())
        }
    };

    (manual gas $fn_name: ident, $service_name: expr, $query_name: expr,($($param_name:ident: $param_type:ty),*) -> $return_type: ty) => {
        #[allow(clippy::too_many_arguments)]
        pub async fn $fn_name(&self, api: &GearApi, listener: &mut EventListener, gas: u64, $($param_name: $param_type),*) -> Result<$return_type> {
            let query = [$service_name.encode(), $query_name.encode(), ($($param_name),*).encode()].concat();
            let (message_id, _) = api
                .send_message_bytes(self.0.into(), query.clone(), gas, 0)
                .await?;

            let (_, raw_reply, _) = listener
                .reply_bytes_on(message_id)
                .await?;

            let decoded_reply: (String, String, $return_type) = match raw_reply {
                Ok(raw_reply) => gclient_decode(raw_reply)?,
                Err(e) => panic!("no reply: {:?}", e)
            };

            Ok(decoded_reply.2)
        }
    };

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

    impl_api!(balance_of, "Vft", "BalanceOf", (account: ActorId) -> U256);
    impl_api!(mint, "Vft", "Mint", (account: ActorId, amount: U256) -> bool);
    impl_api!(approve, "Vft", "Approve", (spender: ActorId, allowance: U256) -> bool);
    impl_api!(allowance, "Vft", "Allowance", (owner: ActorId, spender: ActorId) -> U256);
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

    impl_api!(
        manual gas deposit_tokens, "VftTreasury", "DepositTokens", (vara_token_id: ActorId, from: ActorId, amount: U256, to: H160) -> Result<(U256, H160), vft_treasury_app::services::error::Error>
    );

    impl_api!(
        manual gas withdraw_tokens, "VftTreasury", "WithdrawTokens", (ethereum_token: H160, recepient: ActorId, amount: U256)
    );

    impl_api!(
        map_vara_to_eth_address,
        "VftTreasury",
        "MapVaraToEthAddress",
        (ethereum_token: H160, vara_token: ActorId)
    );

    impl_api!(vara_to_eth_addresses,
        "VftTreasury",
        "VaraToEthAddresses",
        () -> Vec<(ActorId, H160)>
    );

    impl_api!(update_ethereum_event_client_address,
        "VftTreasury",
        "UpdateEthereumEventClientAddress",
        (new_address: ActorId) -> Result<(), vft_treasury_app::services::error::Error>
    );
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
