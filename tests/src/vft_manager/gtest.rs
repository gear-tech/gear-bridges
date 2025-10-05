use gtest::{Program, System, WasmProgram};
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use vft_client::{traits::*, Vft as VftC, VftAdmin as VftAdminC, VftFactory as VftFactoryC};
use vft_manager_client::{
    traits::*, Config, Error, InitConfig, TokenSupply, VftManager as VftManagerC,
    VftManagerFactory as VftManagerFactoryC,
};
use vft_vara_client::{traits::VftVaraFactory, Mainnet};

const REMOTING_ACTOR_ID: u64 = 1_000;
const HISTORICAL_PROXY_ID: u64 = 500;
const BRIDGE_BUILTIN_ID: u64 = 300;

const WRONG_GEAR_SUPPLY_VFT: u64 = 666;

const ERC20_MANAGER_ADDRESS: H160 = H160([1; 20]);
const ETH_TOKEN_RECEIVER: H160 = H160([6; 20]);

const ERC20_TOKEN_GEAR_SUPPLY: H160 = H160([10; 20]);
const ERC20_TOKEN_ETH_SUPPLY: H160 = H160([15; 20]);

#[derive(Debug, Clone)]
struct GearBridgeBuiltinMock;

impl WasmProgram for GearBridgeBuiltinMock {
    fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        #[derive(Encode)]
        enum Response {
            MessageSent {
                block_number: u32,
                hash: H256,
                nonce: U256,
                queue_id: u64,
            },
        }

        Ok(Some(
            Response::MessageSent {
                block_number: 1,
                nonce: U256::from(1),
                hash: [1; 32].into(),
                queue_id: 1,
            }
            .encode(),
        ))
    }

    fn clone_boxed(&self) -> Box<dyn WasmProgram> {
        Box::new(self.clone())
    }

    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        unimplemented!()
    }
}

struct Fixture {
    remoting: GTestRemoting,
    vft_manager_program_id: ActorId,
    gear_supply_vft: ActorId,
    eth_supply_vft: ActorId,
}

async fn setup_for_test() -> Fixture {
    let system = System::new();
    system.init_logger();
    system.mint_to(REMOTING_ACTOR_ID, 100_000_000_000_000_000);
    system.mint_to(HISTORICAL_PROXY_ID, 100_000_000_000_000_000);

    let remoting = GTestRemoting::new(system, REMOTING_ACTOR_ID.into());

    // Bridge Builtin
    let gear_bridge_builtin =
        Program::mock_with_id(remoting.system(), BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    let _ = gear_bridge_builtin.send_bytes(REMOTING_ACTOR_ID, b"INIT");

    // Vft Manager
    let vft_manager_code_id = remoting.system().submit_code(vft_manager::WASM_BINARY);
    let init_config = InitConfig {
        gear_bridge_builtin: BRIDGE_BUILTIN_ID.into(),
        historical_proxy_address: HISTORICAL_PROXY_ID.into(),
        config: Config {
            gas_for_token_ops: 15_000_000_000,
            gas_for_reply_deposit: 15_000_000_000,
            gas_to_send_request_to_builtin: 15_000_000_000,
            gas_for_swap_token_maps: 1_500_000_000,
            reply_timeout: 100,
            fee_bridge: 0,
            fee_incoming: 0,
        },
    };
    let vft_manager_program_id = VftManagerFactoryC::new(remoting.clone())
        .new(init_config)
        .send_recv(vft_manager_code_id, b"salt")
        .await
        .unwrap();

    let mut service = vft_manager_client::VftManager::new(remoting.clone());
    service
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    service
        .update_erc_20_manager_address(ERC20_MANAGER_ADDRESS)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    // VFT
    let vft_code_id = remoting.system().submit_code(vft_vara::WASM_BINARY);
    let gear_supply_vft = vft_vara_client::VftVaraFactory::new(remoting.clone())
        .new(Mainnet::No)
        .send_recv(vft_code_id, b"salt")
        .await
        .unwrap();

    // Allocating underlying shards.
    let mut vft_extension = vft_client::VftExtension::new(remoting.clone());
    while vft_extension
        .allocate_next_balances_shard()
        .send_recv(gear_supply_vft)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .send_recv(gear_supply_vft)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    let vft_code_id = remoting.system().submit_code(vft::WASM_BINARY);
    let eth_supply_vft = VftFactoryC::new(remoting.clone())
        .new("Token".into(), "Token".into(), 18)
        .send_recv(vft_code_id, b"salt1")
        .await
        .unwrap();

    // Allocating underlying shards.
    while vft_extension
        .allocate_next_balances_shard()
        .send_recv(eth_supply_vft)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .send_recv(eth_supply_vft)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    let mut vft = VftAdminC::new(remoting.clone());
    vft.set_minter(vft_manager_program_id)
        .send_recv(eth_supply_vft)
        .await
        .unwrap();
    vft.set_burner(vft_manager_program_id)
        .send_recv(eth_supply_vft)
        .await
        .unwrap();

    // Setup mapping
    let mut vft_manager = VftManagerC::new(remoting.clone());
    vft_manager
        .map_vara_to_eth_address(gear_supply_vft, ERC20_TOKEN_GEAR_SUPPLY, TokenSupply::Gear)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    vft_manager
        .map_vara_to_eth_address(
            eth_supply_vft,
            ERC20_TOKEN_ETH_SUPPLY,
            TokenSupply::Ethereum,
        )
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    Fixture {
        remoting,
        vft_manager_program_id,
        gear_supply_vft,
        eth_supply_vft,
    }
}

#[tokio::test]
async fn test_gear_supply_token() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        gear_supply_vft,
        ..
    } = setup_for_test().await;

    let account_id: ActorId = 100_000.into();
    let amount = 1_000_000_000_000u128;
    remoting.system().mint_to(account_id, 100 * amount);

    let mut vft = VftAdminC::new(remoting.clone());

    let amount = U256::from(amount);
    vft.mint(account_id, amount)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(vft_manager_program_id, amount)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();
    assert!(ok);

    let mut vft_manager = VftManagerC::new(remoting.clone().with_actor_id(account_id));
    let reply = vft_manager
        .request_bridging(gear_supply_vft, amount, ETH_TOKEN_RECEIVER)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let expected = Ok((U256::from(1), ERC20_TOKEN_GEAR_SUPPLY));
    assert_eq!(reply, expected);

    let account_balance = balance_of(&remoting, gear_supply_vft, account_id).await;
    assert!(account_balance.is_zero());

    let vft_manager_balance = balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await;
    assert_eq!(vft_manager_balance, amount);
}

#[tokio::test]
async fn test_eth_supply_token() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        eth_supply_vft,
        ..
    } = setup_for_test().await;

    let account_id: ActorId = 100_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);
    let amount = U256::from(10_000_000_000_u64);

    let receipt_rlp = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        [3u8; 20].into(),
        account_id,
        ERC20_TOKEN_ETH_SUPPLY,
        amount,
    );
    VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(0, 0, receipt_rlp)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();

    let account_balance = balance_of(&remoting, eth_supply_vft, account_id).await;
    assert_eq!(account_balance, amount);

    let vft_manager_balance = balance_of(&remoting, eth_supply_vft, vft_manager_program_id).await;
    assert!(vft_manager_balance.is_zero());

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(vft_manager_program_id, amount)
        .send_recv(eth_supply_vft)
        .await
        .unwrap();
    assert!(ok);

    let mut vft_manager = VftManagerC::new(remoting.clone().with_actor_id(account_id));
    let reply = vft_manager
        .request_bridging(eth_supply_vft, amount, ETH_TOKEN_RECEIVER)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let expected = Ok((U256::from(1), ERC20_TOKEN_ETH_SUPPLY));
    assert_eq!(reply, expected);

    let account_balance = balance_of(&remoting, eth_supply_vft, account_id).await;
    assert!(account_balance.is_zero());

    let vft_manager_balance = balance_of(&remoting, eth_supply_vft, vft_manager_program_id).await;
    assert!(vft_manager_balance.is_zero());
}

#[tokio::test]
async fn test_mapping_does_not_exists() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        ..
    } = setup_for_test().await;

    let reply = VftManagerC::new(remoting.clone())
        .request_bridging(
            WRONG_GEAR_SUPPLY_VFT.into(),
            U256::zero(),
            ETH_TOKEN_RECEIVER,
        )
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    assert_eq!(reply.unwrap_err(), Error::NoCorrespondingEthAddress);
}

#[tokio::test]
async fn test_withdraw_fails_with_bad_origin() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        ..
    } = setup_for_test().await;

    let mut vft_manager = VftManagerC::new(remoting.clone());

    let account_id: ActorId = 42.into();
    let receipt_rlp = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        [3u8; 20].into(),
        account_id,
        ERC20_TOKEN_GEAR_SUPPLY,
        U256::zero(),
    );
    let result = vft_manager
        .submit_receipt(0, 0, receipt_rlp)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    assert_eq!(result.unwrap_err(), Error::NotHistoricalProxy);
}

#[tokio::test]
async fn test_requests_fail_on_pause() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        ..
    } = setup_for_test().await;

    let mut vft_manager = VftManagerC::new(remoting.clone());

    vft_manager
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let result = vft_manager
        .request_bridging(ActorId::zero(), U256::zero(), H160::zero())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(result, Err(Error::Paused));

    let result = vft_manager
        .submit_receipt(0, 0, vec![])
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(result, Err(Error::Paused));

    let result = vft_manager
        .handle_request_bridging_interrupted_transfer(MessageId::zero())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(result, Err(Error::Paused));
}

#[tokio::test]
async fn test_pause_works() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        ..
    } = setup_for_test().await;

    let mut vft_manager = VftManagerC::new(remoting.clone());

    let pause_admin = 11111.into();
    let pause_remoting = remoting.clone().with_actor_id(pause_admin);
    pause_remoting
        .system()
        .mint_to(pause_admin, 100_000_000_000_000);
    let mut pause_admin_vft_manager = VftManagerC::new(pause_remoting);

    vft_manager
        .set_pause_admin(pause_admin)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    macro_rules! assert_paused {
        ($paused: expr) => {
            assert_eq!(
                vft_manager
                    .is_paused()
                    .recv(vft_manager_program_id)
                    .await
                    .unwrap(),
                $paused
            );
        };
    }

    assert_paused!(false);

    pause_admin_vft_manager
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_paused!(true);

    pause_admin_vft_manager
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_paused!(false);

    vft_manager
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_paused!(true);

    vft_manager
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_paused!(false);
}

async fn balance_of(
    remoting: &GTestRemoting,
    vft_program_id: ActorId,
    program_id: ActorId,
) -> U256 {
    VftC::new(remoting.clone())
        .balance_of(program_id)
        .recv(vft_program_id)
        .await
        .unwrap()
}
