use blake2::{digest::typenum::U32, Blake2b, Digest};
use gclient::{EventProcessor, GearApi, Result, WSAddress};
use gear_core::ids::{MessageId, ProgramId};
use grc20_gateway_app::services::error::Error;
use grc20_gateway_app::services::Config;
use grc20_gateway_app::services::InitConfig;
use gstd::ActorId;
use gstd::{Decode, Encode};
use primitive_types::U256;
use sails_rtl::H160;

pub async fn upload_ft(api: &GearApi, listener: &mut gclient::EventListener) -> Result<ProgramId> {
    let init = ("USDC".to_owned(), "USDC".to_owned(), 6_u8);
    let request = ["New".encode(), init.encode()].concat();
    let path = "./erc20_wasm.opt.wasm";
    let gas_info = api
        .calculate_upload_gas(
            None,
            gclient::code_from_os(path).unwrap(),
            request.clone(),
            0,
            true,
        )
        .await
        .expect("Error calculate upload gas");

    let (message_id, program_id, _hash) = api
        .upload_program_bytes(
            gclient::code_from_os(path).unwrap(),
            gclient::now_micros().to_le_bytes(),
            request,
            gas_info.min_limit,
            0,
        )
        .await
        .expect("Error upload program bytes");

    assert!(listener.message_processed(message_id).await?.succeed());
    println!("Fungible token was uploaded with address {:?}", program_id);

    Ok(program_id)
}

fn decode<T: Decode>(payload: Vec<u8>) -> Result<T> {
    Ok(T::decode(&mut payload.as_slice())?)
}

pub async fn balance_of(
    client: &GearApi,
    program_id: ProgramId,
    listener: &mut gclient::EventListener,
    account: ActorId,
) -> U256 {
    let request = [
        "Erc20".encode(),
        "BalanceOf".to_string().encode(),
        account.encode(),
    ]
    .concat();

    let gas_info = client
        .calculate_handle_gas(None, program_id, request.clone(), 0, true)
        .await
        .expect("Error calculate handle gas");

    let (message_id, _) = client
        .send_message_bytes(program_id, request.clone(), gas_info.min_limit, 0)
        .await
        .expect("Error send message bytes");

    let (_, raw_reply, _) = listener
        .reply_bytes_on(message_id)
        .await
        .expect("Error listen reply");

    let decoded_reply: (String, String, U256) = match raw_reply {
        Ok(raw_reply) => decode(raw_reply).expect("Erroe decode reply"),
        Err(_error) => gstd::panic!("Error in getting reply"),
    };
    decoded_reply.2
}

pub async fn check_balance(
    client: &GearApi,
    program_id: ProgramId,
    listener: &mut gclient::EventListener,
    account: ActorId,
    expected_balance: U256,
) {
    let balance = balance_of(client, program_id, listener, account).await;
    assert_eq!(balance, expected_balance);
}
async fn upload_grc20_gateway(
    api: &GearApi,
    listener: &mut gclient::EventListener,
    ft_id: ProgramId,
    gear_bridge_builtin: ActorId,
) -> Result<ProgramId> {
    let init = InitConfig::new(
        (<[u8; 32]>::from(ft_id)).into(),
        gear_bridge_builtin.into(),
        Config::new(
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
            5,
        ),
    );

    let request = ["New".encode(), init.encode()].concat();
    let path = "../../../target/wasm32-unknown-unknown/release/grc20_gateway_wasm.opt.wasm";
    let gas_info = api
        .calculate_upload_gas(
            None,
            gclient::code_from_os(path).unwrap(),
            request.clone(),
            0,
            true,
        )
        .await
        .expect("Error calculate upload gas");

    let (message_id, program_id, _hash) = api
        .upload_program_bytes(
            gclient::code_from_os(path).unwrap(),
            gclient::now_micros().to_le_bytes(),
            request,
            gas_info.min_limit,
            0,
        )
        .await
        .expect("Error upload program bytes");

    assert!(listener.message_processed(message_id).await?.succeed());

    println!("Grc20 gateway uploaded with address {:?}", program_id);
    let grc20_gateway_id: ActorId = <[u8; 32]>::from(program_id).into();
    println!("Sending message to grant BURN role to grc20 gateway");
    let payload = [
        "Admin".to_string().encode(),
        "GrantRole".to_string().encode(),
        (grc20_gateway_id, Role::Burner).encode(),
    ]
    .concat();

    let (message_id, _) = api
        .send_message_bytes(ft_id, payload, 10_000_000_000, 0)
        .await?;

    assert!(listener.message_processed(message_id).await?.succeed());

    println!("Sending message to grant MINT role to grc20 gateway");
    let payload = [
        "Admin".to_string().encode(),
        "GrantRole".to_string().encode(),
        (grc20_gateway_id, Role::Minter).encode(),
    ]
    .concat();

    let (message_id, _) = api
        .send_message_bytes(ft_id, payload, 10_000_000_000, 0)
        .await?;

    assert!(listener.message_processed(message_id).await?.succeed());

    Ok(program_id)
}

#[tokio::test]
async fn grc20_gateway_test() -> Result<()> {
    //let client = GearApi::init(WSAddress::new("ws://65.21.117.24", Some(8989))).await?;
    let client = GearApi::dev().await?;
    let mut listener = client.subscribe().await?;
    let gear_bridge_builtin = gear_bridge_builtin_actor_id();

    println!(
        "Gear-bridge-builtin id {:?}",
        ProgramId::from(<[u8; 32]>::from(gear_bridge_builtin))
    );
    let ft_id = upload_ft(&client, &mut listener).await?;

    let grc20_gateway_id =
        upload_grc20_gateway(&client, &mut listener, ft_id, gear_bridge_builtin).await?;

    println!("Sending message to grant MINT role to Alice");
    let sender = ActorId::new(
        client
            .account_id()
            .encode()
            .try_into()
            .expect("Unexpected invalid account id length."),
    );

    let payload = [
        "Admin".to_string().encode(),
        "GrantRole".to_string().encode(),
        (sender, Role::Minter).encode(),
    ]
    .concat();

    let (message_id, _) = client
        .send_message_bytes(ft_id, payload, 10_000_000_000, 0)
        .await?;

    assert!(listener.message_processed(message_id).await?.succeed());

    println!("Sending message to mint tokens to Alice");
    let payload = [
        "Admin".to_string().encode(),
        "Mint".to_string().encode(),
        (sender, U256::from(10_000_000_000 as u64)).encode(),
    ]
    .concat();

    let (message_id, _) = client
        .send_message_bytes(ft_id, payload, 10_000_000_000, 0)
        .await?;

    assert!(listener.message_processed(message_id).await?.succeed());

    println!("Sending message to transfer tokens from Vara to Eth");
    let receiver = H160::random();
    let eth_token_id = H160::random();
    let payload = [
        "Grc20Gateway".to_string().encode(),
        "TeleportVaraToEth".to_string().encode(),
        (
            sender,
            U256::from(10_000_000_000 as u64),
            receiver,
            eth_token_id,
        )
            .encode(),
    ]
    .concat();

    let (message_id, _) = client
        .send_message_bytes(grc20_gateway_id, payload, 100_000_000_000, 0)
        .await?;

    let (_, raw_reply, _) = listener
        .reply_bytes_on(message_id)
        .await
        .expect("Error listen reply");

    let decoded_reply: (String, String, Result<(), Error>) = match raw_reply {
        Ok(raw_reply) => decode(raw_reply).expect("Erroe decode reply"),
        Err(_error) => gstd::panic!("Error in getting reply"),
    };

    println!("Reply from grc20-gateway {:?}", decoded_reply.2);

    let alice_balance = balance_of(&client, ft_id, &mut listener, sender).await;
    println!("Alice balance: {:?}", alice_balance);

    Ok(())
}

fn gear_bridge_builtin_actor_id() -> ActorId {
    let seed = b"built/in";
    let builtin_id = 2;
    let mut hasher = Blake2b::<U32>::new();
    hasher.update((seed, builtin_id).encode().as_slice());
    let result: [u8; 32] = hasher.finalize().into();
    result.into()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
pub enum Role {
    Admin,
    Burner,
    Minter,
}
