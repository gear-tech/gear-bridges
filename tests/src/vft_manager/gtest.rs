use gtest::{Log, Program, System, WasmProgram};
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use vft_client::{traits::*, Vft as VftC, VftAdmin as VftAdminC, VftFactory as VftFactoryC};
use vft_manager_client::{
    traits::*, Config, Error, InitConfig, MessageStatus, Order, TokenSupply, TxDetails,
    VftManager as VftManagerC, VftManagerFactory as VftManagerFactoryC,
};
use vft_vara_client::{traits::VftVaraFactory, Mainnet};

const REMOTING_ACTOR_ID: u64 = 1_000;
const HISTORICAL_PROXY_ID: u64 = 500;
const BRIDGE_BUILTIN_ID: u64 = 300;
const MALFORMED_TOKEN_ID: u64 = 400;

const WRONG_GEAR_SUPPLY_VFT: u64 = 666;

const ERC20_MANAGER_ADDRESS: H160 = H160([1; 20]);
const ETH_TOKEN_RECEIVER: H160 = H160([6; 20]);

const ERC20_TOKEN_GEAR_SUPPLY: H160 = H160([10; 20]);
const ERC20_TOKEN_ETH_SUPPLY: H160 = H160([15; 20]);

#[derive(Debug, Clone, Copy)]
enum ReplyBehavior {
    Queued,
    Rejected,
    Malformed,
}

#[derive(Debug, Clone)]
struct ReplyMock(ReplyBehavior);

fn queued_bridge_reply() -> Vec<u8> {
    #[derive(Encode)]
    enum Response {
        MessageSent {
            block_number: u32,
            hash: H256,
            nonce: U256,
            queue_id: u64,
        },
    }

    Response::MessageSent {
        block_number: 1,
        nonce: U256::from(1),
        hash: [1; 32].into(),
        queue_id: 1,
    }
    .encode()
}

impl WasmProgram for ReplyMock {
    fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        match self.0 {
            ReplyBehavior::Queued => Ok(Some(queued_bridge_reply())),
            ReplyBehavior::Rejected => Err("rejected"),
            ReplyBehavior::Malformed => Ok(Some(vec![0xff])),
        }
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

async fn mint_eth_supply_tokens(
    remoting: &GTestRemoting,
    vft_manager_program_id: ActorId,
    eth_supply_vft: ActorId,
    account_id: ActorId,
    amount: U256,
    transaction_index: u64,
) {
    let receipt_rlp = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        [3u8; 20].into(),
        account_id,
        ERC20_TOKEN_ETH_SUPPLY,
        amount,
    );
    VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(0, transaction_index, receipt_rlp)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(remoting, eth_supply_vft, account_id).await,
        amount
    );
}

async fn setup_for_test() -> Fixture {
    setup_for_test_with_builtin(Some(ReplyBehavior::Queued), 100).await
}

async fn setup_for_test_with_builtin(
    builtin_behavior: Option<ReplyBehavior>,
    reply_timeout: u32,
) -> Fixture {
    let system = System::new();
    system.init_logger();
    system.mint_to(REMOTING_ACTOR_ID, 100_000_000_000_000_000);
    system.mint_to(HISTORICAL_PROXY_ID, 100_000_000_000_000_000);

    let remoting = GTestRemoting::new(system, REMOTING_ACTOR_ID.into());

    // Bridge Builtin
    if let Some(behavior) = builtin_behavior {
        let gear_bridge_builtin =
            Program::mock_with_id(remoting.system(), BRIDGE_BUILTIN_ID, ReplyMock(behavior));
        let _ = gear_bridge_builtin.send_bytes(REMOTING_ACTOR_ID, b"INIT");
    } else {
        remoting
            .system()
            .mint_to(BRIDGE_BUILTIN_ID, 100_000_000_000_000);
    }

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
            reply_timeout,
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

    mint_eth_supply_tokens(
        &remoting,
        vft_manager_program_id,
        eth_supply_vft,
        account_id,
        amount,
        0,
    )
    .await;

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
async fn test_submit_receipt_concurrent_replay_prevents_double_mint() {
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

    let manual = remoting
        .clone()
        .with_block_run_mode(BlockRunMode::Manual)
        .with_actor_id(HISTORICAL_PROXY_ID.into());
    let mut client_1 = VftManagerC::new(manual.clone());
    let mut client_2 = VftManagerC::new(manual.clone());

    let ticket_1 = client_1
        .submit_receipt(0, 0, receipt_rlp.clone())
        .send(vft_manager_program_id)
        .await
        .unwrap();
    let ticket_2 = client_2
        .submit_receipt(0, 0, receipt_rlp)
        .send(vft_manager_program_id)
        .await
        .unwrap();

    for _ in 0..3 {
        manual.run_next_block();
    }

    ticket_1.recv().await.unwrap().unwrap();
    assert_eq!(ticket_2.recv().await.unwrap(), Err(Error::AlreadyProcessed));
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        amount
    );
}

#[tokio::test]
async fn test_failed_mint_releases_receipt_for_retry() {
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

    let mut vft = VftAdminC::new(remoting.clone());
    vft.set_minter(REMOTING_ACTOR_ID.into())
        .send_recv(eth_supply_vft)
        .await
        .unwrap();

    let failed = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(0, 0, receipt_rlp.clone())
        .send_recv(vft_manager_program_id)
        .await;
    assert!(matches!(failed, Err(_) | Ok(Err(_))));
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());

    vft.set_minter(vft_manager_program_id)
        .send_recv(eth_supply_vft)
        .await
        .unwrap();
    VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(0, 0, receipt_rlp.clone())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        amount
    );

    let replay = VftManagerC::new(remoting.with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(0, 0, receipt_rlp)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(replay, Err(Error::AlreadyProcessed));
}

#[tokio::test]
async fn test_failed_burn_is_not_recoverable() {
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

    let result = VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .request_bridging(eth_supply_vft, amount, ETH_TOKEN_RECEIVER)
        .send_recv(vft_manager_program_id)
        .await;
    assert!(result.is_err());

    let entries = VftManagerC::new(remoting.clone())
        .request_briding_msg_tracker_state(0, 100)
        .recv(vft_manager_program_id)
        .await
        .unwrap();
    let (msg_id, info) = entries
        .into_iter()
        .find(|(_, info)| info.details.sender == account_id && info.details.amount == amount)
        .expect("failed burn must remain visible for forensic inspection");
    assert_eq!(info.status, MessageStatus::TokenDepositCompleted(false));

    let recovery = VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await;
    assert!(recovery.is_err());
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());
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

#[tokio::test]
async fn test_upgrade_rejects_unpaused_destination_without_moving_balances() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        gear_supply_vft,
        ..
    } = setup_for_test().await;

    let code_id = remoting.system().submit_code(vft_manager::WASM_BINARY);
    let destination = VftManagerFactoryC::new(remoting.clone())
        .new(InitConfig {
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
        })
        .send_recv(code_id, b"unpaused-destination")
        .await
        .unwrap();

    let amount = U256::from(1_000_000_000_000u64);
    VftAdminC::new(remoting.clone())
        .mint(vft_manager_program_id, amount)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();

    let mut manager = VftManagerC::new(remoting.clone());
    manager.unpause().send_recv(destination).await.unwrap();
    manager
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    assert!(manager
        .upgrade(destination)
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
    assert!(manager
        .is_paused()
        .recv(vft_manager_program_id)
        .await
        .unwrap());
    assert_eq!(
        balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await,
        amount
    );
    assert!(balance_of(&remoting, gear_supply_vft, destination)
        .await
        .is_zero());
}

#[tokio::test]
async fn test_bridge_timeout_is_quarantined_and_late_reply_does_not_refund() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        eth_supply_vft,
        ..
    } = setup_for_test_with_builtin(None, 2).await;

    let account_id: ActorId = 100_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);
    let amount = U256::from(10_000_000_000_u64);
    mint_eth_supply_tokens(
        &remoting,
        vft_manager_program_id,
        eth_supply_vft,
        account_id,
        amount,
        0,
    )
    .await;

    let manual = remoting
        .clone()
        .with_block_run_mode(BlockRunMode::Manual)
        .with_actor_id(account_id);
    let mut manager = VftManagerC::new(manual.clone());
    let ticket = manager
        .request_bridging(eth_supply_vft, amount, ETH_TOKEN_RECEIVER)
        .send(vft_manager_program_id)
        .await
        .unwrap();

    for _ in 0..8 {
        manual.run_next_block();
    }

    assert!(matches!(
        ticket.recv().await.unwrap(),
        Err(Error::ReplyFailure(_))
    ));
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());

    let entries = VftManagerC::new(remoting.clone())
        .request_briding_msg_tracker_state(0, 100)
        .recv(vft_manager_program_id)
        .await
        .unwrap();
    let (msg_id, info) = entries
        .into_iter()
        .find(|(_, info)| info.details.sender == account_id && info.details.amount == amount)
        .unwrap();
    assert_eq!(info.status, MessageStatus::SendingMessageToBridgeBuiltin);

    assert!(VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .is_err());

    let request = Log::builder().source(vft_manager_program_id);
    let mailbox = remoting.system().get_mailbox(BRIDGE_BUILTIN_ID);
    assert!(mailbox.contains(&request));
    mailbox
        .reply_bytes(request.clone(), queued_bridge_reply(), 0)
        .unwrap();
    remoting.system().run_next_block();

    let info = VftManagerC::new(remoting.clone())
        .request_briding_msg_tracker_state(0, 100)
        .recv(vft_manager_program_id)
        .await
        .unwrap()
        .into_iter()
        .find(|(id, _)| id == &msg_id)
        .unwrap()
        .1;
    assert_eq!(
        info.status,
        MessageStatus::BridgeResponseReceived(Some((U256::from(1), [1; 32].into(), 1)))
    );
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());
    assert!(mailbox
        .reply_bytes(request, queued_bridge_reply(), 0)
        .is_err());
}

#[tokio::test]
async fn test_malformed_bridge_success_reply_is_quarantined() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        eth_supply_vft,
        ..
    } = setup_for_test_with_builtin(Some(ReplyBehavior::Malformed), 100).await;

    let account_id: ActorId = 100_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);
    let amount = U256::from(10_000_000_000_u64);
    mint_eth_supply_tokens(
        &remoting,
        vft_manager_program_id,
        eth_supply_vft,
        account_id,
        amount,
        0,
    )
    .await;

    let result = VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .request_bridging(eth_supply_vft, amount, ETH_TOKEN_RECEIVER)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(result, Err(Error::InvalidMessageStatus));
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());

    let (msg_id, info) = VftManagerC::new(remoting.clone())
        .request_briding_msg_tracker_state(0, 100)
        .recv(vft_manager_program_id)
        .await
        .unwrap()
        .into_iter()
        .find(|(_, info)| info.details.sender == account_id && info.details.amount == amount)
        .unwrap();
    assert_eq!(info.status, MessageStatus::SendingMessageToBridgeBuiltin);
    assert!(VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());
}

#[tokio::test]
async fn test_definite_bridge_rejection_refunds_once() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        eth_supply_vft,
        ..
    } = setup_for_test_with_builtin(Some(ReplyBehavior::Rejected), 100).await;

    let account_id: ActorId = 100_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);
    let amount = U256::from(10_000_000_000_u64);
    mint_eth_supply_tokens(
        &remoting,
        vft_manager_program_id,
        eth_supply_vft,
        account_id,
        amount,
        0,
    )
    .await;

    let result = VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .request_bridging(eth_supply_vft, amount, ETH_TOKEN_RECEIVER)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(result, Err(Error::MessageFailed));
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        amount
    );

    let (msg_id, info) = VftManagerC::new(remoting.clone())
        .request_briding_msg_tracker_state(0, 100)
        .recv(vft_manager_program_id)
        .await
        .unwrap()
        .into_iter()
        .find(|(_, info)| info.details.sender == account_id && info.details.amount == amount)
        .unwrap();
    assert_eq!(info.status, MessageStatus::TokensReturnComplete(true));
    assert!(VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        amount
    );
}

fn tx_details(
    vara_token_id: ActorId,
    sender: ActorId,
    amount: U256,
    token_supply: TokenSupply,
) -> TxDetails {
    TxDetails {
        vara_token_id,
        sender,
        amount,
        receiver: ETH_TOKEN_RECEIVER,
        token_supply,
    }
}

async fn seed_msg_info(
    remoting: &GTestRemoting,
    vft_manager_program_id: ActorId,
    msg_id: MessageId,
    status: MessageStatus,
    details: TxDetails,
) {
    let mut manager = VftManagerC::new(remoting.clone());
    manager
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    manager
        .insert_message_info(msg_id, status, details)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    manager
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_malformed_token_refund_reply_remains_in_flight() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        ..
    } = setup_for_test().await;

    let token = Program::mock_with_id(
        remoting.system(),
        MALFORMED_TOKEN_ID,
        ReplyMock(ReplyBehavior::Malformed),
    );
    token.send_bytes(REMOTING_ACTOR_ID, b"INIT");
    remoting.system().run_next_block();

    let account_id: ActorId = 100_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);
    let msg_id: MessageId = [3u8; 32].into();
    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::TokenDepositCompleted(true),
        tx_details(
            MALFORMED_TOKEN_ID.into(),
            account_id,
            U256::from(1),
            TokenSupply::Gear,
        ),
    )
    .await;

    let result = VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(result, Err(Error::InvalidMessageStatus));

    let info = VftManagerC::new(remoting.clone())
        .request_briding_msg_tracker_state(0, 100)
        .recv(vft_manager_program_id)
        .await
        .unwrap()
        .into_iter()
        .find(|(id, _)| id == &msg_id)
        .unwrap()
        .1;
    assert_eq!(info.status, MessageStatus::SendingMessageToReturnTokens);
    assert!(VftManagerC::new(remoting.with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
}

#[tokio::test]
async fn test_transactions_large_count_is_bounded_by_entries() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        ..
    } = setup_for_test().await;

    let transactions = VftManagerC::new(remoting)
        .transactions(Order::Direct, 0, u32::MAX)
        .recv(vft_manager_program_id)
        .await
        .unwrap();
    assert!(transactions.is_empty());
}

#[tokio::test]
async fn test_interrupted_transfer_concurrent_reentry_eth_supply() {
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
    let msg_id: MessageId = [1u8; 32].into();

    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::TokenDepositCompleted(true),
        tx_details(eth_supply_vft, account_id, amount, TokenSupply::Ethereum),
    )
    .await;

    // Queue two recovery calls for the same `msg_id` into a single block. The first
    // call commits the `SendingMessageToReturnTokens` status when it starts waiting
    // for the VFT reply, so the second call observes the refund already in flight.
    let remoting = remoting.with_block_run_mode(BlockRunMode::Manual);
    let mut client_1 = VftManagerC::new(remoting.clone().with_actor_id(account_id));
    let mut client_2 = VftManagerC::new(remoting.clone().with_actor_id(account_id));

    let ticket1 = client_1
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send(vft_manager_program_id)
        .await
        .unwrap();
    let ticket2 = client_2
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send(vft_manager_program_id)
        .await
        .unwrap();

    for _ in 0..3 {
        remoting.run_next_block();
    }

    ticket1.recv().await.unwrap().unwrap();
    assert!(ticket2.recv().await.is_err());

    // Exactly one refund must be executed despite the two concurrent calls.
    let account_balance = balance_of(&remoting, eth_supply_vft, account_id).await;
    assert_eq!(account_balance, amount);
}

#[tokio::test]
async fn test_interrupted_transfer_concurrent_reentry_gear_supply() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        gear_supply_vft,
        ..
    } = setup_for_test().await;

    let account_id: ActorId = 100_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);
    let amount = U256::from(1_000_000_000_000u128);
    let msg_id: MessageId = [2u8; 32].into();

    // Pre-fund the manager with liquidity of other users: twice the refunded amount,
    // so a duplicated unlock has funds to drain.
    VftAdminC::new(remoting.clone())
        .mint(vft_manager_program_id, U256::from(2_000_000_000_000u128))
        .send_recv(gear_supply_vft)
        .await
        .unwrap();

    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::TokenDepositCompleted(true),
        tx_details(gear_supply_vft, account_id, amount, TokenSupply::Gear),
    )
    .await;

    let remoting = remoting.with_block_run_mode(BlockRunMode::Manual);
    let mut client_1 = VftManagerC::new(remoting.clone().with_actor_id(account_id));
    let mut client_2 = VftManagerC::new(remoting.clone().with_actor_id(account_id));

    let ticket1 = client_1
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send(vft_manager_program_id)
        .await
        .unwrap();
    let ticket2 = client_2
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send(vft_manager_program_id)
        .await
        .unwrap();

    for _ in 0..3 {
        remoting.run_next_block();
    }

    ticket1.recv().await.unwrap().unwrap();
    assert!(ticket2.recv().await.is_err());

    // Exactly one unlock must be executed despite the two concurrent calls.
    let account_balance = balance_of(&remoting, gear_supply_vft, account_id).await;
    assert_eq!(account_balance, amount);
    let manager_balance = balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await;
    assert_eq!(manager_balance, amount);
}

#[tokio::test]
async fn test_interrupted_transfer_recovers_from_intermediate_statuses() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        gear_supply_vft,
        eth_supply_vft,
    } = setup_for_test().await;

    let account_id: ActorId = 100_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);

    let mut vft_manager = VftManagerC::new(remoting.clone().with_actor_id(account_id));

    // Token lock/burn is complete, but the message to the bridge built-in actor
    // hasn't been sent.
    let eth_amount = U256::from(10_000_000_000_u64);
    let msg_id: MessageId = [3u8; 32].into();
    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::TokenDepositCompleted(true),
        tx_details(
            eth_supply_vft,
            account_id,
            eth_amount,
            TokenSupply::Ethereum,
        ),
    )
    .await;

    vft_manager
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        eth_amount
    );

    // Reconciliation confirmed that the bridge request was not queued.
    let msg_id: MessageId = [4u8; 32].into();
    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::BridgeResponseReceived(None),
        tx_details(
            eth_supply_vft,
            account_id,
            eth_amount,
            TokenSupply::Ethereum,
        ),
    )
    .await;

    vft_manager
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        U256::from(20_000_000_000_u64)
    );

    // Token refund message has been sent but it has failed.
    let gear_amount = U256::from(1_000_000_000_000u128);
    VftAdminC::new(remoting.clone())
        .mint(vft_manager_program_id, gear_amount)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();

    let msg_id: MessageId = [5u8; 32].into();
    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::TokensReturnComplete(false),
        tx_details(gear_supply_vft, account_id, gear_amount, TokenSupply::Gear),
    )
    .await;

    vft_manager
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, gear_supply_vft, account_id).await,
        gear_amount
    );
}

#[tokio::test]
async fn test_interrupted_transfer_rejects_in_flight_refund() {
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

    // A refund for this message is already in flight: the status was committed when
    // the refund message to the VFT program was sent and the program started waiting
    // for the reply.
    let msg_id: MessageId = [6u8; 32].into();
    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::SendingMessageToReturnTokens,
        tx_details(
            eth_supply_vft,
            account_id,
            U256::from(10_000_000_000_u64),
            TokenSupply::Ethereum,
        ),
    )
    .await;

    let result = VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await;
    assert!(result.is_err());

    // No duplicated refund has been executed.
    let account_balance = balance_of(&remoting, eth_supply_vft, account_id).await;
    assert!(account_balance.is_zero());
}

#[tokio::test]
async fn test_interrupted_transfer_caller_restrictions() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        eth_supply_vft,
        ..
    } = setup_for_test().await;

    let account_id: ActorId = 100_000.into();
    let stranger_id: ActorId = 200_000.into();
    remoting
        .system()
        .mint_to(account_id, 100_000_000_000_000_000);
    remoting
        .system()
        .mint_to(stranger_id, 100_000_000_000_000_000);

    let amount = U256::from(10_000_000_000_u64);
    let msg_id: MessageId = [7u8; 32].into();
    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        msg_id,
        MessageStatus::TokenDepositCompleted(true),
        tx_details(eth_supply_vft, account_id, amount, TokenSupply::Ethereum),
    )
    .await;

    // A third party can't trigger the recovery. The panic rolls the state back, so
    // the message info stays intact for the legit caller.
    let result = VftManagerC::new(remoting.clone().with_actor_id(stranger_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await;
    assert!(result.is_err());

    // The admin can trigger the recovery; the refund still goes to the original sender.
    VftManagerC::new(remoting.clone())
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();

    let account_balance = balance_of(&remoting, eth_supply_vft, account_id).await;
    assert_eq!(account_balance, amount);
}

#[tokio::test]
async fn test_interrupted_transfer_concurrent_reentry_pool_drain() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        gear_supply_vft,
        ..
    } = setup_for_test().await;

    let victim: ActorId = 100_001.into();
    let attacker: ActorId = 100_002.into();
    let amount = U256::from(1_000_000_000_000u128);

    remoting.system().mint_to(victim, 100_000_000_000_000_000);
    remoting.system().mint_to(attacker, 100_000_000_000_000_000);

    let mut vft_admin = VftAdminC::new(remoting.clone());
    vft_admin
        .mint(victim, amount)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();
    vft_admin
        .mint(attacker, amount)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();

    // Both users bridge once, so the vft-manager custody pool holds 2 * amount.
    for who in [victim, attacker] {
        VftC::new(remoting.clone().with_actor_id(who))
            .approve(vft_manager_program_id, amount)
            .send_recv(gear_supply_vft)
            .await
            .unwrap();
        VftManagerC::new(remoting.clone().with_actor_id(who))
            .request_bridging(gear_supply_vft, amount, ETH_TOKEN_RECEIVER)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap()
            .unwrap();
    }

    let pool = balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await;
    assert_eq!(pool, amount + amount);
    assert!(balance_of(&remoting, gear_supply_vft, attacker)
        .await
        .is_zero());

    // The attacker's request is interrupted right after the lock — the documented
    // recovery entry point for `handle_request_bridging_interrupted_transfer`.
    let stuck = MessageId::from([42u8; 32]);
    seed_msg_info(
        &remoting,
        vft_manager_program_id,
        stuck,
        MessageStatus::TokenDepositCompleted(true),
        tx_details(gear_supply_vft, attacker, amount, TokenSupply::Gear),
    )
    .await;

    // Two recovery calls submitted as two extrinsics of the same block.
    let manual = remoting
        .clone()
        .with_block_run_mode(BlockRunMode::Manual)
        .with_actor_id(attacker);

    let mut client_1 = VftManagerC::new(manual.clone());
    let mut client_2 = VftManagerC::new(manual.clone());

    let call_1 = client_1
        .handle_request_bridging_interrupted_transfer(stuck)
        .send(vft_manager_program_id)
        .await
        .unwrap();
    let call_2 = client_2
        .handle_request_bridging_interrupted_transfer(stuck)
        .send(vft_manager_program_id)
        .await
        .unwrap();

    for _ in 0..3 {
        manual.run_next_block();
    }

    call_1.recv().await.unwrap().unwrap();
    assert!(call_2.recv().await.is_err());

    // Exactly one refund is executed: the attacker gets their own interrupted
    // transfer back and the victim's locked tokens stay in the custody pool.
    let attacker_balance = balance_of(&remoting, gear_supply_vft, attacker).await;
    assert_eq!(attacker_balance, amount);
    let pool_after = balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await;
    assert_eq!(pool_after, amount);
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
