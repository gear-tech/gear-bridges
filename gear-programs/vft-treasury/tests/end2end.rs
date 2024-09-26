use gclient::{EventListener, EventProcessor as _, GearApi, Result};
use gear_core::ids::ProgramId;
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vft_treasury_app::services::vft::{traits::*, Vft as VftC};
use vft_treasury_client::{traits::*, Config, InitConfig, VftTreasury as VftTreasuryC};

async fn upload_program(
    api: &gclient::GearApi,
    listener: &mut gclient::EventListener,
    code: &[u8],
    payload: Vec<u8>,
) -> gclient::Result<ProgramId> {
    let gas_limit = api
        .calculate_upload_gas(None, code.to_vec(), payload.clone(), 0, true)
        .await?
        .min_limit;
    println!("init gas {gas_limit:?}");
    let (message_id, program_id, _) = api
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

async fn upload_vft_program(api: &GearApi, listener: &mut EventListener) -> Result<ProgramId> {
    let payload = ["New".encode(), ("Token", "Token", 18).encode()].concat();

    let program_id =
        upload_program(api, listener, extended_vft_wasm::WASM_BINARY_OPT, payload).await?;
    println!("vft ID = {:?}", ProgramId::from(program_id));
    Ok(program_id)
}

async fn upload_treasury_program(api: &GearApi, listener: &mut EventListener) -> Result<ProgramId> {
    let seed = *b"built/in";
    // a code based on what is in runtime/vara and gear-builtin pallet. Update
    // if the pallet or runtime are changed.
    // ActorWithId<3> is bridge builtin while `seed` comes from pallet-gear-builtin.
    let bridge_builtin_id: ProgramId =
        gear_core::ids::hash((seed, 3u64).encode().as_slice()).into();
    println!("bridge builtin id={:?}", bridge_builtin_id);
    let init_config = InitConfig {
        receiver_contract_address: [2; 20].into(),
        gear_bridge_builtin: bridge_builtin_id,
        ethereum_event_client: 44.into(),
        config: Config {
            gas_for_reply_deposit: 15_000_000_000,
            gas_for_transfer_to_eth_msg: 15_000_000_000,
            gas_for_transfer_tokens: 15_000_000_000,
            gas_to_send_request_to_builtin: 15_000_000_000,
            reply_timeout: 100,
        },
    };

    let payload = ["New".encode(), init_config.encode()].concat();

    let program_id = upload_program(api, listener, vft_treasury::WASM_BINARY, payload).await?;
    println!("treasury ID = {:?}", <ProgramId>::from(program_id));

    Ok(program_id)
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

// ATTENTION: If the test fails with the error: "Inability to pay some fees (e.g. account balance too low)",
// please top up the account's balance.
#[tokio::test]
#[ignore]
async fn test_treasury() -> Result<()> {
    // It will not work on local node as vft-treasury logic relies on pallet-gear-eth-bridge
    // which will be initialized only in ~12 hrs from start on local node.
    let mut api = GearApi::vara_testnet().await?;

    let actor = api.get_actor_id();
    let mut listener = api.subscribe().await?;
    // Check that blocks are still running
    assert!(listener.blocks_running().await?);

    let vft_program_id = upload_vft_program(&api, &mut listener).await?;
    let treasury_program_id = upload_treasury_program(&api, &mut listener).await?;

    let remoting = GClientRemoting::new(api.clone());

    let eth_token_id = H160::from([3; 20]);
    let amount = U256::from(10_000_000_000_u64);
    let gas_limit = 100_000_000_000;

    let mut vft = VftC::new(remoting.clone());
    let ok = vft
        .mint(actor, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok, "failed to mint to {actor}");

    let balance = vft.balance_of(actor).recv(vft_program_id).await.unwrap();
    assert_eq!(balance, amount);

    let ok = vft
        .approve(treasury_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(
        ok,
        "failed to approve {:?} spending {} tokens from {:?}",
        treasury_program_id, amount, actor
    );

    let allowance = vft
        .allowance(actor, treasury_program_id)
        .recv(vft_program_id)
        .await
        .unwrap();
    assert_eq!(allowance, amount);

    let mut treasury = VftTreasuryC::new(remoting.clone());
    treasury
        .map_vara_to_eth_address(eth_token_id, vft_program_id)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .expect("failed to map address");

    let reply = treasury
        .deposit_tokens(vft_program_id, actor, amount, eth_token_id)
        .with_gas_limit(gas_limit)
        .send_recv(treasury_program_id)
        .await
        .unwrap_or_else(|e| {
            eprintln!("error: {:?}", e);
            api.print_node_logs();
            std::panic!()
        });

    assert_eq!(reply.expect("failed to deposit").1, eth_token_id);

    treasury
        .update_ethereum_event_client_address(actor)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .expect("failed to update ETH event client address");

    treasury
        .withdraw_tokens(eth_token_id, actor, amount)
        .with_gas_limit(gas_limit)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .expect("failed to withdraw tokens");

    let balance = vft.balance_of(actor).recv(vft_program_id).await.unwrap();
    assert_eq!(balance, amount);

    Ok(())
}
