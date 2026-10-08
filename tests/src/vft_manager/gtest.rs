use gtest::{Log, Program, System, WasmProgram};
use sails_rs::{calls::*, futures::FutureExt, gtest::calls::*, prelude::*};
use vft_client::{traits::*, Vft as VftC, VftAdmin as VftAdminC, VftFactory as VftFactoryC};
use vft_manager_client::{
    traits::*, Config, Error, InitConfig, MessageStatus, Order, TokenSupply,
    VftManager as VftManagerC, VftManagerFactory as VftManagerFactoryC,
};
use vft_vara_client::{
    traits::{VftAdmin as _, VftNativeExchange as _, VftVaraFactory},
    Mainnet,
};

const REMOTING_ACTOR_ID: u64 = 1_000;
const HISTORICAL_PROXY_ID: u64 = 500;
const BRIDGE_BUILTIN_ID: u64 = 300;

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

// Exercise the benchmark's real user-mailbox reply path, not a mocked manager.
async fn benchmark_reply(
    remoting: &GTestRemoting,
    manager: ActorId,
    key: (u64, u64),
    supply: TokenSupply,
) -> sails_rs::errors::Result<Result<(), Error>> {
    use vft_client::{vft::io::TransferFrom, vft_admin::io::Mint};

    let manual = remoting.clone().with_block_run_mode(BlockRunMode::Manual);
    let caller = manual.actor_id();
    let mut service = VftManagerC::new(manual.clone());
    let pending = service
        .calculate_gas_for_reply(key.0, key.1, supply.clone())
        .send(manager)
        .await?;
    manual.run_next_block();

    let (call, reply) = match supply {
        TokenSupply::Ethereum => (Mint::encode_call(caller, U256::from(100_u32)), Vec::new()),
        TokenSupply::Gear => {
            let mut reply = TransferFrom::ROUTE.to_vec();
            true.encode_to(&mut reply);
            (
                TransferFrom::encode_call(manager, caller, U256::from(100_u32)),
                reply,
            )
        }
    };
    let mailbox = manual.system().get_mailbox(caller);
    for (call, reply) in std::iter::once((call, reply)) {
        let token_call = Log::builder()
            .source(manager)
            .dest(caller)
            .payload_bytes(call);
        if mailbox.contains(&token_call) {
            mailbox.reply_bytes(token_call, reply, 0).unwrap();
            // Execute the token reply hook and the woken benchmark request.
            for _ in 0..2 {
                manual.run_next_block();
            }
        }
    }
    pending
        .recv()
        .now_or_never()
        .expect("benchmark request must produce a terminal reply")
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

    vft_client::allocate_shards(
        remoting.clone(),
        gear_supply_vft,
        gtest::constants::MAX_USER_GAS_LIMIT,
    )
    .await
    .unwrap();

    let vft_code_id = remoting.system().submit_code(vft::WASM_BINARY);
    let eth_supply_vft = VftFactoryC::new(remoting.clone())
        .new("Token".into(), "Token".into(), 18)
        .send_recv(vft_code_id, b"salt1")
        .await
        .unwrap();

    vft_client::allocate_shards(
        remoting.clone(),
        eth_supply_vft,
        gtest::constants::MAX_USER_GAS_LIMIT,
    )
    .await
    .unwrap();

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
async fn test_benchmark_mutations_require_admin_on_deployable_wasm() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        eth_supply_vft,
        ..
    } = setup_for_test().await;
    let caller: ActorId = 100_000.into();
    remoting.system().mint_to(caller, 100_000_000_000_000_000);
    let unauthorized = remoting.clone().with_actor_id(caller);
    let mut admin = VftManagerC::new(remoting.clone());
    let mut outsider = VftManagerC::new(unauthorized.clone());

    // Use the production constructor and a nonempty real receipt history so
    // an empty-history panic cannot masquerade as denial of fill_transactions.
    mint_eth_supply_tokens(
        &remoting,
        manager,
        eth_supply_vft,
        caller,
        U256::from(7_u64),
        0,
    )
    .await;
    let mut mappings = admin.vara_to_eth_addresses().recv(manager).await.unwrap();
    mappings.sort_unstable_by_key(|entry| entry.0);

    for (index, supply) in [TokenSupply::Ethereum, TokenSupply::Gear]
        .into_iter()
        .enumerate()
    {
        let result = benchmark_reply(&unauthorized, manager, (10, index as u64), supply).await;
        assert!(
            matches!(
                &result,
                Err(sails_rs::errors::Error::Rtl(
                    sails_rs::errors::RtlError::ReplyHasError(_, _)
                ))
            ),
            "non-admin completed benchmark receipt: {result:?}"
        );
        assert_eq!(
            admin
                .receipt_status(10, index as u64)
                .recv(manager)
                .await
                .unwrap(),
            vft_manager_client::ReceiptStatus::Unknown,
        );
    }
    outsider
        .fill_transactions()
        .send_recv(manager)
        .await
        .expect_err("non-admin populated processed receipt history");
    outsider
        .calculate_gas_for_token_map_swap()
        .send_recv(manager)
        .await
        .expect_err("non-admin cleared token mappings");
    let mut after = admin.vara_to_eth_addresses().recv(manager).await.unwrap();
    after.sort_unstable_by_key(|entry| entry.0);
    assert_eq!(after, mappings);
    assert_eq!(
        admin
            .transactions(Order::Direct, 0, 10)
            .recv(manager)
            .await
            .unwrap(),
        vec![(0, 0)],
    );
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, caller).await,
        U256::from(7_u64)
    );

    // The attack must not preempt the authenticated deposit for the same key.
    let receipt = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        [3_u8; 20].into(),
        caller,
        ERC20_TOKEN_ETH_SUPPLY,
        U256::from(11_u64),
    );
    assert_eq!(
        VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
            .submit_receipt(10, 0, receipt)
            .send_recv(manager)
            .await
            .unwrap(),
        Ok(()),
    );
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, caller).await,
        U256::from(18_u64)
    );
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(eth_supply_vft)
            .await
            .unwrap(),
        U256::from(18_u64),
    );

    // The same caller can benchmark once legitimately appointed as admin.
    admin.set_admin(caller).send_recv(manager).await.unwrap();
    for (index, supply) in [TokenSupply::Ethereum, TokenSupply::Gear]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            benchmark_reply(&unauthorized, manager, (20, index as u64), supply)
                .await
                .unwrap(),
            Ok(()),
        );
        assert_eq!(
            outsider
                .receipt_status(20, index as u64)
                .recv(manager)
                .await
                .unwrap(),
            vft_manager_client::ReceiptStatus::Processed,
        );
    }
    assert!(outsider
        .fill_transactions()
        .with_gas_limit(gtest::constants::MAX_USER_GAS_LIMIT)
        .send_recv(manager)
        .await
        .unwrap());
    assert_eq!(
        outsider.receipt_status(21, 0).recv(manager).await.unwrap(),
        vft_manager_client::ReceiptStatus::Processed,
    );
    outsider
        .calculate_gas_for_token_map_swap()
        .send_recv(manager)
        .await
        .unwrap();
    assert_eq!(
        outsider
            .vara_to_eth_addresses()
            .recv(manager)
            .await
            .unwrap(),
        Vec::new(),
    );
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, caller).await,
        U256::from(18_u64)
    );
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

    let manager = VftManagerC::new(remoting.clone());
    assert_eq!(
        manager
            .receipt_status(0, 0)
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        vft_manager_client::ReceiptStatus::Processed,
    );
    assert_eq!(
        manager
            .transactions(Order::Direct, 0, 10)
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        vec![(0, 0)],
    );
    let receipt = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        [3u8; 20].into(),
        account_id,
        ERC20_TOKEN_ETH_SUPPLY,
        amount,
    );
    assert_eq!(
        VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
            .submit_receipt(0, 0, receipt)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::AlreadyProcessed),
    );
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        amount
    );
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(eth_supply_vft)
            .await
            .unwrap(),
        amount
    );

    let vft_manager_balance = balance_of(&remoting, eth_supply_vft, vft_manager_program_id).await;
    assert!(vft_manager_balance.is_zero());

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(vft_manager_program_id, amount)
        .send_recv(eth_supply_vft)
        .await
        .unwrap();
    assert!(ok);
    assert_eq!(
        VftC::new(remoting.clone())
            .allowance(account_id, vft_manager_program_id)
            .recv(eth_supply_vft)
            .await
            .unwrap(),
        amount,
    );

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
async fn test_storage_initialization_resumes_without_appending_shards() {
    let system = System::new();
    system.mint_to(REMOTING_ACTOR_ID, 100_000_000_000_000_000);
    let remoting = GTestRemoting::new(system, REMOTING_ACTOR_ID.into());
    let code = remoting.system().submit_code(vft::WASM_BINARY);
    let token = VftFactoryC::new(remoting.clone())
        .new("Resume".into(), "RSM".into(), 6)
        .send_recv(code, b"resume-storage")
        .await
        .unwrap();
    let mut extension = vft_client::VftExtension::new(remoting.clone());
    extension
        .allocate_next_balances_shard()
        .send_recv(token)
        .await
        .unwrap();
    for _ in 0..2 {
        vft_client::allocate_shards(
            remoting.clone(),
            token,
            gtest::constants::MAX_USER_GAS_LIMIT,
        )
        .await
        .unwrap();
        assert!(!extension
            .allocate_next_balances_shard()
            .send_recv(token)
            .await
            .unwrap());
        assert!(!extension
            .allocate_next_allowances_shard()
            .send_recv(token)
            .await
            .unwrap());
    }
    VftAdminC::new(remoting.clone())
        .mint(REMOTING_ACTOR_ID.into(), 7.into())
        .send_recv(token)
        .await
        .unwrap();
    assert_eq!(
        balance_of(&remoting, token, REMOTING_ACTOR_ID.into()).await,
        7.into()
    );
    assert!(VftC::new(remoting.clone())
        .approve(HISTORICAL_PROXY_ID.into(), 3.into())
        .send_recv(token)
        .await
        .unwrap());
    assert_eq!(
        VftC::new(remoting)
            .allowance(REMOTING_ACTOR_ID.into(), HISTORICAL_PROXY_ID.into())
            .recv(token)
            .await
            .unwrap(),
        3.into()
    );
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

    // Queue two identical submissions in the same block to verify the receipt
    // reservation is visible before the first asynchronous VFT call yields.
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
    let reply_2 = ticket_2.recv().await.unwrap();
    assert!(matches!(
        reply_2,
        Err(Error::AlreadyProcessed | Error::ReceiptLeaseActive)
    ));

    let account_balance = balance_of(&remoting, eth_supply_vft, account_id).await;
    assert_eq!(account_balance, amount);
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
async fn test_submit_receipt_processes_every_deposit_log_and_rejects_replay() {
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
    let first = U256::from(11_u64);
    let second = U256::from(17_u64);
    let receipt = crate::create_receipt_rlp_with_logs(
        ERC20_MANAGER_ADDRESS,
        vec![
            ([3_u8; 20].into(), account_id, ERC20_TOKEN_ETH_SUPPLY, first),
            (
                [4_u8; 20].into(),
                account_id,
                ERC20_TOKEN_ETH_SUPPLY,
                second,
            ),
        ],
    );
    let mut manager = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    manager
        .submit_receipt(4, 2, receipt.clone())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        first + second
    );
    assert_eq!(
        manager
            .receipt_status(4, 2)
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        vft_manager_client::ReceiptStatus::Processed,
    );
    assert_eq!(
        manager
            .submit_receipt(4, 2, receipt)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::AlreadyProcessed),
    );
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        first + second
    );
}

#[tokio::test]
async fn test_partial_receipt_retry_skips_completed_deposit_logs() {
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
    let second_vft_code_id = remoting.system().submit_code(vft::WASM_BINARY);
    let second_vft = VftFactoryC::new(remoting.clone())
        .new("Second token".into(), "SEC".into(), 18)
        .send_recv(second_vft_code_id, b"receipt-second-token")
        .await
        .unwrap();
    vft_client::allocate_shards(
        remoting.clone(),
        second_vft,
        gtest::constants::MAX_USER_GAS_LIMIT,
    )
    .await
    .unwrap();
    let second_erc20 = H160([18; 20]);
    VftManagerC::new(remoting.clone())
        .map_vara_to_eth_address(second_vft, second_erc20, TokenSupply::Ethereum)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let first_amount = U256::from(11_u64);
    let second_amount = U256::from(17_u64);
    let receipt = crate::create_receipt_rlp_with_logs(
        ERC20_MANAGER_ADDRESS,
        vec![
            (
                [3_u8; 20].into(),
                account_id,
                ERC20_TOKEN_ETH_SUPPLY,
                first_amount,
            ),
            ([4_u8; 20].into(), account_id, second_erc20, second_amount),
        ],
    );
    let mut manager = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    assert!(matches!(
        manager
            .submit_receipt(7, 3, receipt.clone())
            .send_recv(vft_manager_program_id)
            .await,
        Ok(Err(Error::ReplyFailure(_)))
    ));
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        first_amount
    );
    assert!(balance_of(&remoting, second_vft, account_id)
        .await
        .is_zero());
    assert_eq!(
        manager
            .receipt_status(7, 3)
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        vft_manager_client::ReceiptStatus::Reserved,
    );

    VftAdminC::new(remoting.clone())
        .set_minter(vft_manager_program_id)
        .send_recv(second_vft)
        .await
        .unwrap();
    manager
        .submit_receipt(7, 3, receipt.clone())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        first_amount
    );
    assert_eq!(
        balance_of(&remoting, second_vft, account_id).await,
        second_amount
    );
    assert_eq!(
        manager
            .receipt_status(7, 3)
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        vft_manager_client::ReceiptStatus::Processed,
    );
    assert_eq!(
        manager
            .submit_receipt(7, 3, receipt)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::AlreadyProcessed),
    );
}

#[tokio::test]
async fn test_gear_supply_multilog_unlock_retry_and_replay_preserve_exact_balances() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        gear_supply_vft,
        ..
    } = setup_for_test().await;
    use vft_vara_client::traits::NativeEscrow as _;
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    admin
        .configure_native_wrapper(Some(gear_supply_vft))
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    let mut native_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    native_admin
        .pause()
        .send_recv(gear_supply_vft)
        .await
        .unwrap();
    vft_vara_client::NativeEscrow::new(remoting.clone())
        .configure_manager(vft_manager_program_id)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();
    native_admin
        .resume()
        .send_recv(gear_supply_vft)
        .await
        .unwrap();
    admin
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    let sender: ActorId = 100_000.into();
    let receiver: ActorId = 100_001.into();
    let second_receiver: ActorId = 100_002.into();
    let first_raw = 2_000_000_000_000_u128;
    let second_raw = 3_000_000_000_000_u128;
    let first = U256::from(first_raw);
    let second = U256::from(second_raw);
    let eth_amount = U256::from(17_u64);
    remoting.system().mint_to(sender, 100_000_000_000_000);
    remoting.system().mint_to(receiver, 1_000_000_000_000);
    remoting
        .system()
        .mint_to(second_receiver, 1_000_000_000_000);
    let initial_native = remoting.system().balance_of(receiver);
    let initial_second_native = remoting.system().balance_of(second_receiver);
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(sender))
        .mint()
        .with_value(first_raw + second_raw)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();
    VftAdminC::new(remoting.clone())
        .set_burner(vft_manager_program_id)
        .send_recv(gear_supply_vft)
        .await
        .unwrap();
    assert!(VftC::new(remoting.clone().with_actor_id(sender))
        .approve(vft_manager_program_id, first + second)
        .send_recv(gear_supply_vft)
        .await
        .unwrap());
    VftManagerC::new(remoting.clone().with_actor_id(sender))
        .request_bridging(gear_supply_vft, first + second, ETH_TOKEN_RECEIVER)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, gear_supply_vft, sender).await,
        U256::zero()
    );
    assert_eq!(
        balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await,
        first + second
    );

    let second_vft_code_id = remoting.system().submit_code(vft::WASM_BINARY);
    let second_vft = VftFactoryC::new(remoting.clone())
        .new("Retry token".into(), "RET".into(), 18)
        .send_recv(second_vft_code_id, b"gear-receipt-retry-token")
        .await
        .unwrap();
    vft_client::allocate_shards(
        remoting.clone(),
        second_vft,
        gtest::constants::MAX_USER_GAS_LIMIT,
    )
    .await
    .unwrap();
    let second_erc20 = H160([19; 20]);
    VftManagerC::new(remoting.clone())
        .map_vara_to_eth_address(second_vft, second_erc20, TokenSupply::Ethereum)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let receipt = crate::create_receipt_rlp_with_logs(
        ERC20_MANAGER_ADDRESS,
        vec![
            ([3; 20].into(), receiver, ERC20_TOKEN_GEAR_SUPPLY, first),
            ([4; 20].into(), receiver, second_erc20, eth_amount),
            (
                [5; 20].into(),
                second_receiver,
                ERC20_TOKEN_GEAR_SUPPLY,
                second,
            ),
        ],
    );
    let mut manager = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    let first_return = Log::builder().source(gear_supply_vft).dest(receiver);
    let second_return = Log::builder().source(gear_supply_vft).dest(second_receiver);
    for attempt in 0..2 {
        let outcome = manager
            .submit_receipt(9, 3, receipt.clone())
            .send_recv(vft_manager_program_id)
            .await;
        assert!(
            matches!(&outcome, Ok(Err(Error::ReplyFailure(_)))),
            "unexpected partial receipt outcome: {outcome:?}"
        );
        let first_mailbox = remoting.system().get_mailbox(receiver);
        let second_mailbox = remoting.system().get_mailbox(second_receiver);
        if attempt == 0 {
            assert!(first_mailbox.contains(&first_return));
            assert!(second_mailbox.contains(&second_return));
            let states = admin
                .receipt_deposits(9, 3)
                .recv(vft_manager_program_id)
                .await
                .unwrap();
            assert_eq!(states.len(), 3);
            assert!(states[0].native && states[2].native);
            assert_eq!(
                states[1].outcome,
                vft_manager_client::ReceiptDepositOutcome::Rejected
            );
            assert_eq!(
                states[0].outcome,
                vft_manager_client::ReceiptDepositOutcome::NativeQueued
            );
            assert_eq!(
                states[2].outcome,
                vft_manager_client::ReceiptDepositOutcome::NativeQueued
            );
            assert!(states[0].child.is_some() && states[2].child.is_some());
            assert_ne!(states[0].operation_id, states[2].operation_id);
            use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
            let receipt_hash = H256::from(KeccakHasher::hash(&receipt));
            let original_id = H256::from(KeccakHasher::hash(
                &(
                    b"vara/native-escrow/v1",
                    vft_manager_program_id,
                    ActorId::from(HISTORICAL_PROXY_ID),
                    ERC20_MANAGER_ADDRESS,
                    9_u64,
                    3_u64,
                    states[0].log_index,
                    receipt_hash,
                )
                    .encode(),
            ));
            assert_eq!(states[0].operation_id, original_id);
            // New administrative policy must not reinterpret previously authenticated deposits.
            admin
                .pause()
                .send_recv(vft_manager_program_id)
                .await
                .unwrap();
            admin
                .configure_native_wrapper(None)
                .send_recv(vft_manager_program_id)
                .await
                .unwrap();
            admin
                .unpause()
                .send_recv(vft_manager_program_id)
                .await
                .unwrap();
            first_mailbox.claim_value(first_return.clone()).unwrap();
            second_mailbox.claim_value(second_return.clone()).unwrap();
        } else {
            assert!(!first_mailbox.contains(&first_return));
            assert!(!second_mailbox.contains(&second_return));
        }
        assert_eq!(
            remoting.system().balance_of(receiver),
            initial_native + first_raw
        );
        assert_eq!(
            remoting.system().balance_of(second_receiver),
            initial_second_native + second_raw
        );
        assert_eq!(
            balance_of(&remoting, gear_supply_vft, receiver).await,
            U256::zero()
        );
        assert_eq!(
            balance_of(&remoting, gear_supply_vft, second_receiver).await,
            U256::zero()
        );
        assert_eq!(
            balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await,
            U256::zero()
        );
        assert_eq!(
            manager
                .receipt_status(9, 3)
                .recv(vft_manager_program_id)
                .await
                .unwrap(),
            vft_manager_client::ReceiptStatus::Reserved
        );
    }

    VftAdminC::new(remoting.clone())
        .set_minter(vft_manager_program_id)
        .send_recv(second_vft)
        .await
        .unwrap();
    manager
        .submit_receipt(9, 3, receipt.clone())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!remoting
        .system()
        .get_mailbox(receiver)
        .contains(&first_return));
    assert!(!remoting
        .system()
        .get_mailbox(second_receiver)
        .contains(&second_return));
    assert_eq!(
        remoting.system().balance_of(receiver),
        initial_native + first_raw
    );
    assert_eq!(
        remoting.system().balance_of(second_receiver),
        initial_second_native + second_raw
    );
    assert_eq!(
        balance_of(&remoting, gear_supply_vft, receiver).await,
        U256::zero()
    );
    assert_eq!(
        balance_of(&remoting, gear_supply_vft, second_receiver).await,
        U256::zero()
    );
    assert_eq!(
        balance_of(&remoting, gear_supply_vft, vft_manager_program_id).await,
        U256::zero()
    );
    assert_eq!(
        balance_of(&remoting, second_vft, receiver).await,
        eth_amount
    );
    let settled = admin
        .receipt_deposits(9, 3)
        .recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(settled.len(), 3);
    assert!(settled
        .iter()
        .all(|log| log.outcome == vft_manager_client::ReceiptDepositOutcome::Settled));
    assert!(settled[0].native && settled[2].native);
    assert_eq!(
        manager
            .receipt_status(9, 3)
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        vft_manager_client::ReceiptStatus::Processed
    );
    assert_eq!(
        manager
            .submit_receipt(9, 3, receipt)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::AlreadyProcessed)
    );
    assert!(!remoting
        .system()
        .get_mailbox(receiver)
        .contains(&first_return));
    assert!(!remoting
        .system()
        .get_mailbox(second_receiver)
        .contains(&second_return));
    assert_eq!(
        remoting.system().balance_of(receiver),
        initial_native + first_raw
    );
    assert_eq!(
        remoting.system().balance_of(second_receiver),
        initial_second_native + second_raw
    );
    assert_eq!(
        balance_of(&remoting, second_vft, receiver).await,
        eth_amount
    );
}

#[tokio::test]
async fn test_legacy_processed_receipt_pair_blocks_all_logs() {
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
    let receipt = crate::create_receipt_rlp_with_logs(
        ERC20_MANAGER_ADDRESS,
        vec![
            (
                [3_u8; 20].into(),
                account_id,
                ERC20_TOKEN_ETH_SUPPLY,
                U256::from(11_u64),
            ),
            (
                [4_u8; 20].into(),
                account_id,
                ERC20_TOKEN_ETH_SUPPLY,
                U256::from(17_u64),
            ),
        ],
    );
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    admin
        .insert_transactions(vec![(9, 4)])
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    admin
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let reply = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(9, 4, receipt)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::AlreadyProcessed));
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());
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

    let mut admin = VftManagerC::new(remoting.clone());
    let evidence = admin
        .source_request_evidence(msg_id)
        .recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(evidence.request, msg_id);
    assert_eq!(
        evidence.outcome,
        vft_manager_client::SourceRequestOutcome::Pending
    );
    admin
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(
        admin
            .reconcile_source_request(msg_id, evidence.child, evidence.request_hash)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::InvalidReconciliation)
    );
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
    let queued = admin
        .source_request_evidence(msg_id)
        .recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.child, evidence.child);
    assert_eq!(queued.request_hash, evidence.request_hash);
    assert!(matches!(
        queued.outcome,
        vft_manager_client::SourceRequestOutcome::Queued { .. }
    ));
    assert_eq!(
        admin
            .reconcile_source_request(msg_id, MessageId::from([33; 32]), evidence.request_hash)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::InvalidReconciliation)
    );
    for _ in 0..2 {
        assert_eq!(
            admin
                .reconcile_source_request(msg_id, evidence.child, evidence.request_hash)
                .send_recv(vft_manager_program_id)
                .await
                .unwrap(),
            Ok(queued.outcome.clone())
        );
    }
    admin
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert!(VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .handle_request_bridging_interrupted_transfer(msg_id)
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
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
    let mut admin = VftManagerC::new(remoting.clone());
    let evidence = admin
        .source_request_evidence(msg_id)
        .recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        evidence.outcome,
        vft_manager_client::SourceRequestOutcome::NotQueued
    );
    admin
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            admin
                .reconcile_source_request(msg_id, evidence.child, evidence.request_hash)
                .send_recv(vft_manager_program_id)
                .await
                .unwrap(),
            Ok(vft_manager_client::SourceRequestOutcome::NotQueued)
        );
    }
    admin
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
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

#[tokio::test]
async fn test_emergency_stop_observers_and_expiry() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        ..
    } = setup_for_test().await;

    let observer: ActorId = 11_111.into();
    let pause_admin: ActorId = 22_222.into();
    let unauthorized: ActorId = 33_333.into();
    for actor in [observer, pause_admin, unauthorized] {
        remoting.system().mint_to(actor, 100_000_000_000_000);
    }

    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .set_pause_admin(pause_admin)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    admin
        .add_emergency_stop_observer(observer)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    // Observer registration is replay-safe during recovery.
    admin
        .add_emergency_stop_observer(observer)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(
        admin
            .emergency_stop_observers()
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        vec![observer]
    );
    let mut unauthorized_service = VftManagerC::new(remoting.clone().with_actor_id(unauthorized));
    assert!(unauthorized_service
        .add_emergency_stop_observer(ActorId::from(44_444))
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
    assert!(unauthorized_service
        .remove_emergency_stop_observer(observer)
        .send_recv(vft_manager_program_id)
        .await
        .is_err());

    assert!(admin
        .emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .is_err());

    let mut observer_service = VftManagerC::new(remoting.clone().with_actor_id(observer));
    observer_service
        .emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert!(admin
        .is_emergency_stopped()
        .recv(vft_manager_program_id)
        .await
        .unwrap());
    assert_eq!(
        admin
            .request_bridging(ActorId::zero(), U256::zero(), H160::zero())
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::Paused)
    );
    assert_eq!(
        admin
            .submit_receipt(0, 0, vec![])
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::Paused)
    );

    // Clearing manual pause cannot bypass an active emergency stop.
    admin
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    admin
        .unpause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert!(!admin
        .is_paused()
        .recv(vft_manager_program_id)
        .await
        .unwrap());
    assert!(admin
        .is_emergency_stopped()
        .recv(vft_manager_program_id)
        .await
        .unwrap());
    assert_eq!(
        admin
            .request_bridging(ActorId::zero(), U256::zero(), H160::zero())
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::Paused)
    );
    assert_eq!(
        admin
            .handle_request_bridging_interrupted_transfer(MessageId::zero())
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::Paused)
    );

    assert!(observer_service
        .disable_emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
    assert!(unauthorized_service
        .disable_emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
    let mut pause_admin_service = VftManagerC::new(remoting.clone().with_actor_id(pause_admin));
    pause_admin_service
        .disable_emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert!(!admin
        .is_paused()
        .recv(vft_manager_program_id)
        .await
        .unwrap());
    admin
        .set_pause_admin(ActorId::zero())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(
        admin
            .pause_admin()
            .recv(vft_manager_program_id)
            .await
            .unwrap(),
        ActorId::zero()
    );
    assert!(pause_admin_service
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .is_err());

    observer_service
        .emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert!(pause_admin_service
        .disable_emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
    admin
        .disable_emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    observer_service
        .emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    let until_block = admin
        .emergency_stop_until()
        .recv(vft_manager_program_id)
        .await
        .unwrap();
    remoting.system().run_to_block(until_block - 1);
    assert!(admin
        .is_emergency_stopped()
        .recv(vft_manager_program_id)
        .await
        .unwrap());
    remoting.system().run_to_block(until_block);
    assert!(!admin
        .is_emergency_stopped()
        .recv(vft_manager_program_id)
        .await
        .unwrap());
    assert!(!admin
        .is_paused()
        .recv(vft_manager_program_id)
        .await
        .unwrap());

    admin
        .remove_emergency_stop_observer(observer)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert!(observer_service
        .emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .is_err());
}

#[tokio::test]
async fn test_source_reconciliation_requires_original_canonical_evidence() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        ..
    } = setup_for_test().await;
    let mut service = VftManagerC::new(remoting.clone());
    let request = MessageId::from([99; 32]);
    assert!(service
        .reconcile_source_request(request, request, H256::zero())
        .send_recv(manager)
        .await
        .is_err());
    service.pause().send_recv(manager).await.unwrap();
    assert_eq!(
        service
            .source_request_evidence(request)
            .recv(manager)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        service
            .reconcile_source_request(request, request, H256::zero())
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::InvalidReconciliation)
    );
    let stranger: ActorId = 100_003.into();
    remoting.system().mint_to(stranger, 100_000_000_000_000);
    assert!(VftManagerC::new(remoting.clone().with_actor_id(stranger))
        .reconcile_source_request(request, request, H256::zero())
        .send_recv(manager)
        .await
        .is_err());
    assert!(service
        .request_briding_msg_tracker_state(0, 10)
        .recv(manager)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_ordinary_gear_vft_returns_without_native_burn() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        ..
    } = setup_for_test().await;
    let code = remoting.system().submit_code(vft::WASM_BINARY);
    let token = VftFactoryC::new(remoting.clone())
        .new("Ordinary Gear".into(), "GEAR".into(), 12)
        .send_recv(code, b"ordinary-gear")
        .await
        .unwrap();
    vft_client::allocate_shards(
        remoting.clone(),
        token,
        gtest::constants::MAX_USER_GAS_LIMIT,
    )
    .await
    .unwrap();
    let recipient: ActorId = 100_004.into();
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let amount = U256::from(2_000_000_000_000u128);
    VftAdminC::new(remoting.clone())
        .mint(manager, amount)
        .send_recv(token)
        .await
        .unwrap();
    let erc20 = H160([22; 20]);
    VftManagerC::new(remoting.clone())
        .map_vara_to_eth_address(token, erc20, TokenSupply::Gear)
        .send_recv(manager)
        .await
        .unwrap();
    let receipt = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        H160([3; 20]),
        recipient,
        erc20,
        amount,
    );
    let native_before = remoting.system().balance_of(recipient);
    let mut proxy = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    proxy
        .submit_receipt(30, 2, receipt.clone())
        .send_recv(manager)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(balance_of(&remoting, token, recipient).await, amount);
    assert_eq!(balance_of(&remoting, token, manager).await, U256::zero());
    assert_eq!(remoting.system().balance_of(recipient), native_before);
    assert_eq!(
        proxy
            .submit_receipt(30, 2, receipt)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::AlreadyProcessed)
    );
}

#[tokio::test]
async fn test_native_configuration_is_paused_and_gear_supply_only() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        eth_supply_vft,
        ..
    } = setup_for_test().await;
    let mut service = VftManagerC::new(remoting.clone());
    assert!(service
        .configure_native_wrapper(Some(native))
        .send_recv(manager)
        .await
        .is_err());
    service.pause().send_recv(manager).await.unwrap();
    assert!(service
        .configure_native_wrapper(Some(eth_supply_vft))
        .send_recv(manager)
        .await
        .is_err());
    service
        .configure_native_wrapper(Some(native))
        .send_recv(manager)
        .await
        .unwrap();
    assert_eq!(
        service.native_wrapper().recv(manager).await.unwrap(),
        Some(native)
    );
    assert!(service
        .remove_vara_to_eth_address(native)
        .send_recv(manager)
        .await
        .is_err());
    service
        .configure_native_wrapper(None)
        .send_recv(manager)
        .await
        .unwrap();
    assert_eq!(service.native_wrapper().recv(manager).await.unwrap(), None);
}

#[tokio::test]
async fn test_native_wrapper_wasm_escrow_idempotency_mailbox_and_returned_value() {
    use vft_vara_client::{traits::NativeEscrow as _, PayoutStatus};
    let Fixture {
        remoting,
        gear_supply_vft: native,
        eth_supply_vft: rejects_empty_payload,
        ..
    } = setup_for_test().await;
    let manager: ActorId = 100_005.into();
    let recipient: ActorId = 100_006.into();
    remoting.system().mint_to(manager, 100_000_000_000_000);
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let raw = 3_000_000_000_000u128;
    let amount = U256::from(raw);
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(manager))
        .mint()
        .with_value(raw * 2)
        .send_recv(native)
        .await
        .unwrap();
    let mut admin = vft_vara_client::VftAdmin::new(remoting.clone());
    let mut escrow_admin = vft_vara_client::NativeEscrow::new(remoting.clone());
    assert!(escrow_admin
        .configure_manager(manager)
        .send_recv(native)
        .await
        .is_err());
    admin.pause().send_recv(native).await.unwrap();
    escrow_admin
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    admin.set_burner(manager).send_recv(native).await.unwrap();
    admin.resume().send_recv(native).await.unwrap();
    let id = H256([40; 32]);
    assert!(escrow_admin
        .redeem_escrow(id, manager, recipient, amount)
        .send_recv(native)
        .await
        .is_err());
    let mut escrow = vft_vara_client::NativeEscrow::new(remoting.clone().with_actor_id(manager));
    assert!(escrow
        .redeem_escrow(id, recipient, recipient, amount)
        .send_recv(native)
        .await
        .is_err());
    let native_before = remoting.system().balance_of(recipient);
    let queued = escrow
        .redeem_escrow(id, manager, recipient, amount)
        .send_recv(native)
        .await
        .unwrap();
    assert_eq!(queued.status, PayoutStatus::Queued);
    assert_eq!(balance_of(&remoting, native, manager).await, amount);
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    assert_eq!(remoting.system().balance_of(recipient), native_before);
    let duplicate = escrow
        .redeem_escrow(id, manager, recipient, amount)
        .send_recv(native)
        .await
        .unwrap();
    assert_eq!(duplicate, queued);
    assert!(escrow
        .redeem_escrow(id, manager, recipient, amount + U256::one())
        .send_recv(native)
        .await
        .is_err());
    let payout = Log::builder().source(native).dest(recipient);
    let mailbox = remoting.system().get_mailbox(recipient);
    assert!(mailbox.contains(&payout));
    mailbox.claim_value(payout.clone()).unwrap();
    remoting.system().run_next_block();
    let delivered = escrow.redemption(id).recv(native).await.unwrap().unwrap();
    assert_eq!(delivered.child, queued.child);
    assert_eq!(delivered.status, PayoutStatus::Delivered);
    assert_eq!(delivered.returned_value, 0);
    assert!(mailbox.claim_value(payout).is_err());
    assert_eq!(remoting.system().balance_of(recipient), native_before + raw);
    assert_eq!(
        escrow
            .redeem_escrow(id, manager, recipient, amount)
            .send_recv(native)
            .await
            .unwrap(),
        delivered
    );
    let returned_id = H256([41; 32]);
    escrow
        .redeem_escrow(returned_id, manager, rejects_empty_payload, amount)
        .send_recv(native)
        .await
        .unwrap();
    remoting.system().run_next_block();
    let returned = escrow
        .redemption(returned_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned.status, PayoutStatus::Returned);
    assert_eq!(returned.returned_value, raw);
    assert_eq!(balance_of(&remoting, native, manager).await, U256::zero());
    assert_eq!(
        balance_of(&remoting, native, rejects_empty_payload).await,
        U256::zero()
    );
    assert_eq!(
        escrow
            .redeem_escrow(returned_id, manager, rejects_empty_payload, amount)
            .send_recv(native)
            .await
            .unwrap(),
        returned
    );
    admin.pause().send_recv(native).await.unwrap();
    assert!(escrow_admin
        .configure_manager(recipient)
        .send_recv(native)
        .await
        .is_err());
}

#[tokio::test]
async fn test_native_receipt_dead_continuation_keeps_original_payout() {
    use vft_vara_client::traits::NativeEscrow as _;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        eth_supply_vft: failing_recipient,
        ..
    } = setup_for_test_with_builtin(Some(ReplyBehavior::Queued), 2).await;
    let sender: ActorId = 100_008.into();
    let recipient: ActorId = 100_009.into();
    remoting.system().mint_to(sender, 100_000_000_000_000);
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let raw = 2_000_000_000_000u128;
    let amount = U256::from(raw);
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(native))
        .send_recv(manager)
        .await
        .unwrap();
    let mut native_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    native_admin.pause().send_recv(native).await.unwrap();
    vft_vara_client::NativeEscrow::new(remoting.clone())
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    native_admin
        .set_burner(manager)
        .send_recv(native)
        .await
        .unwrap();
    native_admin.resume().send_recv(native).await.unwrap();
    admin.unpause().send_recv(manager).await.unwrap();
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(sender))
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    VftC::new(remoting.clone().with_actor_id(sender))
        .approve(manager, amount)
        .send_recv(native)
        .await
        .unwrap();
    VftManagerC::new(remoting.clone().with_actor_id(sender))
        .request_bridging(native, amount, ETH_TOKEN_RECEIVER)
        .send_recv(manager)
        .await
        .unwrap()
        .unwrap();
    let receipt = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        H160([3; 20]),
        recipient,
        ERC20_TOKEN_GEAR_SUPPLY,
        amount,
    );
    let mut proxy = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    // Reply funding happens after send in gstd: setup failure must roll back that
    // send, not classify it as an economic retry while an untracked child runs.
    assert!(proxy
        .submit_receipt(31, 1, receipt.clone())
        .with_gas_limit(31_000_000_000)
        .send_recv(manager)
        .await
        .is_err());
    assert_eq!(
        admin.receipt_status(31, 1).recv(manager).await.unwrap(),
        vft_manager_client::ReceiptStatus::Unknown
    );
    assert_eq!(balance_of(&remoting, native, manager).await, amount);
    use sails_rs::calls::ActionIo;
    use vft_manager_client::vft_manager::io::SubmitReceipt;
    let payload = SubmitReceipt::encode_call(31, 1, receipt.clone());
    let original = remoting
        .system()
        .get_program(manager)
        .unwrap()
        .send_bytes_with_gas(
            HISTORICAL_PROXY_ID,
            payload,
            gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        );
    // Admit only the originating manager execution, then withhold block gas
    // until its wait has expired. The actual wrapper child remains in the queue.
    let mut admitted = false;
    for gas in (1..=50).map(|i| i * 1_000_000_000) {
        let result = remoting.system().run_next_block_with_allowance(gas);
        assert!(!result.failed.contains(&original));
        if result.gas_burned.contains_key(&original) && !result.not_executed.contains(&original) {
            admitted = true;
            break;
        }
    }
    assert!(admitted, "original manager dispatch was never admitted");
    remoting.system().run_next_block_with_allowance(0);
    remoting.system().run_next_block_with_allowance(0);
    let resumed_block = remoting.system().run_next_block();
    let original_reply = resumed_block
        .log()
        .iter()
        .find(|entry| entry.reply_to() == Some(original))
        .expect("expired original continuation must reply");
    assert!(SubmitReceipt::decode_reply(original_reply.payload())
        .unwrap()
        .is_err());
    let before = admin.receipt_deposits(31, 1).recv(manager).await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(
        before[0].outcome,
        vft_manager_client::ReceiptDepositOutcome::NativeQueued
    );
    let original_child = before[0]
        .child
        .expect("original native dispatch must be retained");
    let operation = before[0].operation_id;
    let wrapper = vft_vara_client::NativeEscrow::new(remoting.clone());
    let payout = wrapper
        .redemption(operation)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payout.status, vft_vara_client::PayoutStatus::Queued);
    remoting
        .system()
        .run_to_block(remoting.system().block_height() + 3);
    assert_eq!(
        proxy
            .submit_receipt(31, 1, receipt.clone())
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::NativeSettlementPending)
    );
    let resumed = admin.receipt_deposits(31, 1).recv(manager).await.unwrap();
    assert_eq!(resumed[0].child, Some(original_child));
    assert_eq!(
        admin
            .reconcile_receipt(31, 1)
            .send_recv(manager)
            .await
            .unwrap(),
        Ok(vft_manager_client::ReceiptStatus::Reserved)
    );
    assert_eq!(
        wrapper
            .redemption(operation)
            .recv(native)
            .await
            .unwrap()
            .unwrap()
            .child,
        payout.child
    );
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    let mailbox = remoting.system().get_mailbox(recipient);
    let message = Log::builder().source(native).dest(recipient);
    mailbox.claim_value(message.clone()).unwrap();
    remoting.system().run_next_block();
    admin.pause().send_recv(manager).await.unwrap();
    let mut reconciler = VftManagerC::new(remoting.clone().with_actor_id(sender));
    assert_eq!(
        reconciler
            .reconcile_receipt(31, 1)
            .send_recv(manager)
            .await
            .unwrap(),
        Ok(vft_manager_client::ReceiptStatus::Processed)
    );
    assert_eq!(
        reconciler
            .reconcile_receipt(31, 1)
            .send_recv(manager)
            .await
            .unwrap(),
        Ok(vft_manager_client::ReceiptStatus::Processed)
    );
    admin.unpause().send_recv(manager).await.unwrap();
    assert_eq!(
        admin.receipt_deposits(31, 1).recv(manager).await.unwrap()[0].outcome,
        vft_manager_client::ReceiptDepositOutcome::Settled
    );
    assert!(mailbox.claim_value(message).is_err());
    assert_eq!(
        proxy
            .submit_receipt(31, 1, receipt)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::AlreadyProcessed)
    );
    // A real program rejecting the original native payout retains its returned
    // reserve obligation and cannot be treated as a fresh economic retry.
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(sender))
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    VftC::new(remoting.clone().with_actor_id(sender))
        .approve(manager, amount)
        .send_recv(native)
        .await
        .unwrap();
    VftManagerC::new(remoting.clone().with_actor_id(sender))
        .request_bridging(native, amount, ETH_TOKEN_RECEIVER)
        .send_recv(manager)
        .await
        .unwrap()
        .unwrap();
    let failed_receipt = crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        H160([3; 20]),
        failing_recipient,
        ERC20_TOKEN_GEAR_SUPPLY,
        amount,
    );
    assert_eq!(
        proxy
            .submit_receipt(32, 1, failed_receipt.clone())
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::NativeSettlementPending)
    );
    let failed_row = admin
        .receipt_deposits(32, 1)
        .recv(manager)
        .await
        .unwrap()
        .remove(0);
    let returned = wrapper
        .redemption(failed_row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned.status, vft_vara_client::PayoutStatus::Returned);
    assert_eq!(returned.returned_value, raw);
    for _ in 0..2 {
        assert_eq!(
            reconciler
                .reconcile_receipt(32, 1)
                .send_recv(manager)
                .await
                .unwrap(),
            Err(Error::NativeSettlementReturned)
        );
    }
    assert_eq!(
        admin.receipt_status(32, 1).recv(manager).await.unwrap(),
        vft_manager_client::ReceiptStatus::Reserved
    );
    assert_eq!(
        admin.receipt_deposits(32, 1).recv(manager).await.unwrap()[0].outcome,
        vft_manager_client::ReceiptDepositOutcome::Unknown
    );
    assert_eq!(
        proxy
            .submit_receipt(32, 1, failed_receipt)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::NativeSettlementReturned)
    );
    assert_eq!(
        wrapper
            .redemption(failed_row.operation_id)
            .recv(native)
            .await
            .unwrap()
            .unwrap()
            .child,
        returned.child
    );
    assert_eq!(balance_of(&remoting, native, manager).await, U256::zero());
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
