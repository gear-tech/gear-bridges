use gtest::{Log, Program, System, WasmProgram};
use sails_rs::{calls::*, futures::FutureExt, gtest::calls::*, prelude::*};
use vft_client::{traits::*, Vft as VftC, VftAdmin as VftAdminC, VftFactory as VftFactoryC};
use vft_manager_client::{
    traits::*, Config, Error, InitConfig, MessageStatus, NativeWrapperConfig, NativeWrapperMode,
    Order, TokenSupply,
    VftManager as VftManagerC, VftManagerFactory as VftManagerFactoryC,
};
use vft_vara_client::{
    traits::{VftAdmin as _, VftNativeExchange as _, VftVaraFactory},
    Mainnet,
};

#[path = "v1.rs"]
mod v1;

const REMOTING_ACTOR_ID: u64 = 1_000;
const HISTORICAL_PROXY_ID: u64 = 500;
const BRIDGE_BUILTIN_ID: u64 = 300;

const WRONG_GEAR_SUPPLY_VFT: u64 = 666;

const ERC20_MANAGER_ADDRESS: H160 = H160([1; 20]);
const ETH_TOKEN_RECEIVER: H160 = H160([6; 20]);

const ERC20_TOKEN_GEAR_SUPPLY: H160 = H160([10; 20]);
const ERC20_TOKEN_ETH_SUPPLY: H160 = H160([15; 20]);

fn held_wrapper_config(
    remoting: &GTestRemoting,
    actor_id: ActorId,
    ethereum_token: H160,
) -> NativeWrapperConfig {
    NativeWrapperConfig {
        actor_id,
        ethereum_token,
        code_id: H256::from(remoting.system().submit_code(vft_vara::WASM_BINARY).into_bytes()),
        mode: NativeWrapperMode::HeldEscrowV2,
    }
}

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

fn deposit_receipt(receiver: ActorId, token: H160, amount: U256) -> Vec<u8> {
    crate::create_receipt_rlp(
        ERC20_MANAGER_ADDRESS,
        H160([3; 20]),
        receiver,
        token,
        amount,
    )
}

async fn deploy_vft(
    remoting: &GTestRemoting,
    name: &str,
    symbol: &str,
    decimals: u8,
    salt: &[u8],
) -> ActorId {
    let code = remoting.system().submit_code(vft::WASM_BINARY);
    let token = VftFactoryC::new(remoting.clone())
        .new(name.into(), symbol.into(), decimals)
        .send_recv(code, salt)
        .await
        .unwrap();
    vft_client::allocate_shards(
        remoting.clone(),
        token,
        gtest::constants::MAX_USER_GAS_LIMIT,
    )
    .await
    .unwrap();
    token
}

async fn receipt_status(
    service: &VftManagerC<GTestRemoting>,
    manager: ActorId,
    slot: u64,
    index: u64,
) -> vft_manager_client::ReceiptStatus {
    service
        .receipt_status(slot, index)
        .recv(manager)
        .await
        .unwrap()
}

async fn receipt_deposits(
    service: &VftManagerC<GTestRemoting>,
    manager: ActorId,
    slot: u64,
    index: u64,
) -> Vec<vft_manager_client::ReceiptDepositState> {
    service
        .receipt_deposits(slot, index)
        .recv(manager)
        .await
        .unwrap()
}

async fn mint_eth_supply_tokens(
    remoting: &GTestRemoting,
    vft_manager_program_id: ActorId,
    eth_supply_vft: ActorId,
    account_id: ActorId,
    amount: U256,
    transaction_index: u64,
) {
    let receipt_rlp = deposit_receipt(account_id, ERC20_TOKEN_ETH_SUPPLY, amount);
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
    valid_reply: bool,
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
        TokenSupply::Ethereum => (
            Mint::encode_call(caller, U256::from(100_u32)),
            if valid_reply {
                Vec::new()
            } else {
                Mint::ROUTE.to_vec()
            },
        ),
        TokenSupply::Gear => {
            let mut reply = TransferFrom::ROUTE.to_vec();
            valid_reply.encode_to(&mut reply);
            if !valid_reply {
                // A trailing byte is malformed, unlike an exact authenticated false.
                reply.push(0xff);
            }
            (
                TransferFrom::encode_call(manager, caller, U256::from(100_u32)),
                reply,
            )
        }
    };
    let mailbox = manual.system().get_mailbox(caller);
    let token_call = Log::builder()
        .source(manager)
        .dest(caller)
        .payload_bytes(call);
    if mailbox.contains(&token_call) {
        let mut service = VftManagerC::new(manual.clone());
        let progress = receipt_deposits(&service, manager, key.0, key.1).await;
        assert_eq!(
            progress.len(),
            1,
            "benchmark must exercise per-log progress"
        );
        let deposit = &progress[0];
        assert_eq!(deposit.log_index, 0);
        assert_eq!(
            deposit.outcome,
            vft_manager_client::ReceiptDepositOutcome::InFlight
        );
        assert!(deposit.child.is_some());
        assert_eq!(deposit.receiver, caller);
        assert_eq!(deposit.token_id, caller);
        assert_eq!(deposit.amount, U256::from(100_u32));
        assert_eq!(deposit.supply, supply);
        assert!(!deposit.native);
        assert_ne!(deposit.operation_id, H256::zero());
        assert_eq!(
            receipt_status(&service, manager, key.0, key.1).await,
            vft_manager_client::ReceiptStatus::Reserved,
        );
        // A concurrent sample cannot replace the progress used by the reply hook.
        let concurrent = service
            .calculate_gas_for_reply(key.0, key.1, supply)
            .send(manager)
            .await
            .unwrap();
        manual.run_next_block();
        assert_eq!(
            concurrent
                .recv()
                .now_or_never()
                .expect("concurrent sample must reply without draining the original")
                .unwrap(),
            Err(Error::AlreadyProcessed),
        );
        assert_eq!(
            receipt_deposits(&service, manager, key.0, key.1).await,
            progress,
        );
        mailbox.reply_bytes(token_call, reply, 0).unwrap();
        // Execute the token reply hook and the woken benchmark request.
        for _ in 0..2 {
            manual.run_next_block();
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
    setup_with_wrapper(builtin_behavior, reply_timeout, false).await
}

async fn setup_with_wrapper(
    builtin_behavior: Option<ReplyBehavior>,
    reply_timeout: u32,
    escrow_v1: bool,
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
    let gear_supply_vft = if escrow_v1 {
        use vft_vara_v1_client::traits::VftVaraFactory as _;
        vft_vara_v1_client::VftVaraFactory::new(remoting.clone())
            .new(vft_vara_v1_client::Mainnet::No)
            .send_recv(remoting.system().submit_code(vft_vara_v1::WASM_BINARY), b"salt")
            .await
            .unwrap()
    } else {
        vft_vara_client::VftVaraFactory::new(remoting.clone())
            .new(Mainnet::No)
            .send_recv(remoting.system().submit_code(vft_vara::WASM_BINARY), b"salt")
            .await
            .unwrap()
    };

    vft_client::allocate_shards(
        remoting.clone(),
        gear_supply_vft,
        gtest::constants::MAX_USER_GAS_LIMIT,
    )
    .await
    .unwrap();

    let eth_supply_vft = deploy_vft(&remoting, "Token", "Token", 18, b"salt1").await;

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
        let result =
            benchmark_reply(&unauthorized, manager, (10, index as u64), supply, true).await;
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
            receipt_status(&admin, manager, 10, index as u64).await,
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
    let receipt = deposit_receipt(caller, ERC20_TOKEN_ETH_SUPPLY, U256::from(11_u64));
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
            benchmark_reply(
                &unauthorized,
                manager,
                (20, index as u64),
                supply.clone(),
                true
            )
            .await
            .unwrap(),
            Ok(()),
        );
        assert_eq!(
            receipt_status(&outsider, manager, 20, index as u64).await,
            vft_manager_client::ReceiptStatus::Processed,
        );
        let key = (20, index as u64);
        let progress = receipt_deposits(&outsider, manager, key.0, key.1).await;
        assert_eq!(progress.len(), 1);
        assert_eq!(
            progress[0].outcome,
            vft_manager_client::ReceiptDepositOutcome::Settled
        );
        assert!(progress[0].child.is_some());
        assert_eq!(
            benchmark_reply(&unauthorized, manager, key, supply, true)
                .await
                .unwrap(),
            Err(Error::AlreadyProcessed),
        );
        assert_eq!(
            receipt_deposits(&outsider, manager, key.0, key.1).await,
            progress,
        );
    }
    assert!(outsider
        .fill_transactions()
        .with_gas_limit(gtest::constants::MAX_USER_GAS_LIMIT)
        .send_recv(manager)
        .await
        .unwrap());
    assert_eq!(
        receipt_status(&outsider, manager, 21, 0).await,
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
async fn test_reply_benchmark_preserves_ambiguous_progress_and_original_child() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        ..
    } = setup_for_test().await;
    let service = VftManagerC::new(remoting.clone());
    for (index, supply) in [TokenSupply::Ethereum, TokenSupply::Gear]
        .into_iter()
        .enumerate()
    {
        let key = (25, index as u64);
        assert_eq!(
            benchmark_reply(&remoting, manager, key, supply.clone(), false)
                .await
                .unwrap(),
            Err(Error::InvalidReply),
        );
        let progress = receipt_deposits(&service, manager, key.0, key.1).await;
        assert_eq!(progress.len(), 1);
        assert_eq!(
            progress[0].outcome,
            vft_manager_client::ReceiptDepositOutcome::Unknown
        );
        assert!(progress[0].child.is_some());
        assert_eq!(
            receipt_status(&service, manager, key.0, key.1).await,
            vft_manager_client::ReceiptStatus::Reserved,
        );
        assert_eq!(
            benchmark_reply(&remoting, manager, key, supply, true)
                .await
                .unwrap(),
            Err(Error::AlreadyProcessed),
        );
        assert_eq!(
            receipt_deposits(&service, manager, key.0, key.1).await,
            progress,
        );
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

    let manager = VftManagerC::new(remoting.clone());
    assert_eq!(
        receipt_status(&manager, vft_manager_program_id, 0, 0).await,
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
    let receipt = deposit_receipt(account_id, ERC20_TOKEN_ETH_SUPPLY, amount);
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
    let receipt_rlp = deposit_receipt(account_id, ERC20_TOKEN_ETH_SUPPLY, amount);

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
    let receipt_rlp = deposit_receipt(account_id, ERC20_TOKEN_ETH_SUPPLY, amount);

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
        receipt_status(&manager, vft_manager_program_id, 4, 2).await,
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
    let second_vft = deploy_vft(
        &remoting,
        "Second token",
        "SEC",
        18,
        b"receipt-second-token",
    )
    .await;
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
        receipt_status(&manager, vft_manager_program_id, 7, 3).await,
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
        receipt_status(&manager, vft_manager_program_id, 7, 3).await,
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
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            gear_supply_vft,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
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

    let second_vft = deploy_vft(
        &remoting,
        "Retry token",
        "RET",
        18,
        b"gear-receipt-retry-token",
    )
    .await;
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
            let states = receipt_deposits(&admin, vft_manager_program_id, 9, 3).await;
            assert_eq!(states.len(), 3);
            assert!(states[0].native && states[2].native);
            assert_eq!(
                states[1].outcome,
                vft_manager_client::ReceiptDepositOutcome::Retryable
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
            let receipt_hash = KeccakHasher::hash(&receipt);
            let original_id = KeccakHasher::hash(
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
            );
            assert_eq!(states[0].operation_id, original_id);
            // New administrative policy must not reinterpret previously authenticated deposits.
            admin
                .pause()
                .send_recv(vft_manager_program_id)
                .await
                .unwrap();
            assert!(admin
                .configure_native_wrapper(None)
                .send_recv(vft_manager_program_id)
                .await
                .is_err());
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
            receipt_status(&manager, vft_manager_program_id, 9, 3).await,
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
    let settled = receipt_deposits(&admin, vft_manager_program_id, 9, 3).await;
    assert_eq!(settled.len(), 3);
    assert!(settled
        .iter()
        .all(|log| log.outcome == vft_manager_client::ReceiptDepositOutcome::Settled));
    assert!(settled[0].native && settled[2].native);
    assert_eq!(
        receipt_status(&manager, vft_manager_program_id, 9, 3).await,
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
async fn test_processed_receipt_pair_blocks_all_logs_without_key_import() {
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
    VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(9, 4, receipt.clone())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();

    let reply = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(9, 4, receipt)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(reply, Err(Error::AlreadyProcessed));
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, account_id).await,
        U256::from(28u32)
    );
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
    let receipt_rlp = deposit_receipt(account_id, ERC20_TOKEN_GEAR_SUPPLY, U256::zero());
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
        .begin_cutover(destination, 32)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
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
    for actor in [account_id, remoting.actor_id()] {
        assert!(VftManagerC::new(remoting.clone().with_actor_id(actor))
            .handle_request_bridging_interrupted_transfer(msg_id)
            .send_recv(vft_manager_program_id)
            .await
            .is_err());
    }
    admin
        .add_emergency_stop_observer(account_id)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    VftManagerC::new(remoting.clone().with_actor_id(account_id))
        .emergency_stop()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    for actor in [account_id, remoting.actor_id()] {
        assert!(VftManagerC::new(remoting.clone().with_actor_id(actor))
            .handle_request_bridging_interrupted_transfer(msg_id)
            .send_recv(vft_manager_program_id)
            .await
            .is_err());
    }
    assert_eq!(
        admin
            .source_request_evidence(msg_id)
            .recv(vft_manager_program_id)
            .await
            .unwrap()
            .unwrap()
            .encode(),
        evidence.encode()
    );
    assert!(balance_of(&remoting, eth_supply_vft, account_id)
        .await
        .is_zero());
    assert_eq!(
        admin
            .reconcile_source_request(msg_id, evidence.child, evidence.request_hash)
            .send_recv(vft_manager_program_id)
            .await
            .unwrap(),
        Err(Error::InvalidReconciliation)
    );
    let destination = upgrade_destination(&remoting, vft_manager_program_id).await;
    assert_cutover_rejected(
        &remoting,
        vft_manager_program_id,
        destination,
        eth_supply_vft,
    )
    .await;
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
    admin
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    admin
        .upgrade(destination)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    assert_eq!(
        admin
            .cutover_status()
            .recv(vft_manager_program_id)
            .await
            .unwrap()
            .phase,
        vft_manager_client::CutoverPhase::Retired
    );
    assert_eq!(
        admin
            .source_request_evidence(msg_id)
            .recv(vft_manager_program_id)
            .await
            .unwrap()
            .unwrap()
            .child,
        evidence.child
    );
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
    let destination = upgrade_destination(&remoting, vft_manager_program_id).await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .pause()
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();
    for emergency in [false, true] {
        if emergency {
            admin
                .add_emergency_stop_observer(account_id)
                .send_recv(vft_manager_program_id)
                .await
                .unwrap();
            VftManagerC::new(remoting.clone().with_actor_id(account_id))
                .emergency_stop()
                .send_recv(vft_manager_program_id)
                .await
                .unwrap();
        }
        for actor in [account_id, remoting.actor_id()] {
            assert!(VftManagerC::new(remoting.clone().with_actor_id(actor))
                .handle_request_bridging_interrupted_transfer(msg_id)
                .send_recv(vft_manager_program_id)
                .await
                .is_err());
        }
        assert!(balance_of(&remoting, eth_supply_vft, account_id)
            .await
            .is_zero());
    }
    assert_cutover_rejected(
        &remoting,
        vft_manager_program_id,
        destination,
        eth_supply_vft,
    )
    .await;
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
async fn test_definite_17_unit_refund_remains_available_under_pause_and_emergency() {
    for emergency in [false, true] {
        let Fixture {
            remoting,
            vft_manager_program_id: manager,
            eth_supply_vft: token,
            ..
        } = setup_for_test_with_builtin(Some(ReplyBehavior::Rejected), 100).await;
        let owner: ActorId = 100_015.into();
        let outsider: ActorId = 100_016.into();
        for actor in [owner, outsider] {
            remoting.system().mint_to(actor, 100_000_000_000_000);
        }
        let amount = U256::from(17u32);
        mint_eth_supply_tokens(&remoting, manager, token, owner, amount, 0).await;
        let mut token_admin = VftAdminC::new(remoting.clone());
        token_admin
            .set_minter(ActorId::zero())
            .send_recv(token)
            .await
            .unwrap();
        // Force the definite builtin rejection's automatic refund to fail. The
        // retained original request, not a synthetic tracker row, owns recovery.
        assert!(VftManagerC::new(remoting.clone().with_actor_id(owner))
            .request_bridging(token, amount, ETH_TOKEN_RECEIVER)
            .send_recv(manager)
            .await
            .is_err());
        let mut admin = VftManagerC::new(remoting.clone());
        let (request, info) = admin
            .request_briding_msg_tracker_state(0, 100)
            .recv(manager)
            .await
            .unwrap()
            .into_iter()
            .find(|(_, info)| info.details.sender == owner)
            .unwrap();
        assert_eq!(info.status, MessageStatus::TokensReturnComplete(false));
        let evidence = admin
            .source_request_evidence(request)
            .recv(manager)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            evidence.outcome,
            vft_manager_client::SourceRequestOutcome::NotQueued
        );
        assert_eq!(balance_of(&remoting, token, owner).await, U256::zero());
        if emergency {
            admin
                .add_emergency_stop_observer(owner)
                .send_recv(manager)
                .await
                .unwrap();
            VftManagerC::new(remoting.clone().with_actor_id(owner))
                .emergency_stop()
                .send_recv(manager)
                .await
                .unwrap();
            assert!(admin.is_emergency_stopped().recv(manager).await.unwrap());
        } else {
            admin.pause().send_recv(manager).await.unwrap();
            assert!(admin.is_paused().recv(manager).await.unwrap());
        }
        assert_eq!(
            VftManagerC::new(remoting.clone().with_actor_id(owner))
                .request_bridging(token, amount, ETH_TOKEN_RECEIVER)
                .send_recv(manager)
                .await
                .unwrap(),
            Err(Error::Paused)
        );
        token_admin
            .set_minter(manager)
            .send_recv(token)
            .await
            .unwrap();
        assert!(VftManagerC::new(remoting.clone().with_actor_id(outsider))
            .handle_request_bridging_interrupted_transfer(request)
            .send_recv(manager)
            .await
            .is_err());
        assert_eq!(balance_of(&remoting, token, owner).await, U256::zero());
        // Both authorized paths return to the original owner, never to the admin.
        let caller = if emergency {
            remoting.actor_id()
        } else {
            owner
        };
        VftManagerC::new(remoting.clone().with_actor_id(caller))
            .handle_request_bridging_interrupted_transfer(request)
            .send_recv(manager)
            .await
            .unwrap()
            .unwrap();
        for actor in [owner, remoting.actor_id(), outsider] {
            assert!(VftManagerC::new(remoting.clone().with_actor_id(actor))
                .handle_request_bridging_interrupted_transfer(request)
                .send_recv(manager)
                .await
                .is_err());
        }
        assert_eq!(balance_of(&remoting, token, owner).await, amount);
        assert_eq!(balance_of(&remoting, token, outsider).await, U256::zero());
        assert_eq!(balance_of(&remoting, token, manager).await, U256::zero());
        assert_eq!(
            VftC::new(remoting.clone())
                .total_supply()
                .recv(token)
                .await
                .unwrap(),
            amount
        );
        assert_eq!(
            admin
                .source_request_evidence(request)
                .recv(manager)
                .await
                .unwrap()
                .unwrap()
                .encode(),
            evidence.encode()
        );
        let (_, info) = admin
            .request_briding_msg_tracker_state(0, 100)
            .recv(manager)
            .await
            .unwrap()
            .into_iter()
            .find(|(id, _)| *id == request)
            .unwrap();
        assert_eq!(info.status, MessageStatus::TokensReturnComplete(true));
    }
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
    // Recovery reaches its original-state authorization even during an emergency
    // stop; an unknown request is rejected, not hidden behind the pause guard.
    assert!(admin
        .handle_request_bridging_interrupted_transfer(MessageId::zero())
        .send_recv(vft_manager_program_id)
        .await
        .is_err());

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
    let token = deploy_vft(&remoting, "Ordinary Gear", "GEAR", 12, b"ordinary-gear").await;
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
    let receipt = deposit_receipt(recipient, erc20, amount);
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

fn submit_receipt_events(
    remoting: &GTestRemoting,
    manager: ActorId,
    slot: u64,
    index: u64,
    receipt: Vec<u8>,
) -> (
    Result<(), Error>,
    Vec<vft_manager_client::vft_manager::events::VftManagerEvents>,
) {
    use sails_rs::events::EventIo;
    use vft_manager_client::vft_manager::{events::VftManagerEvents, io::SubmitReceipt};
    let request = remoting
        .system()
        .get_program(manager)
        .unwrap()
        .send_bytes_with_gas(
            HISTORICAL_PROXY_ID,
            SubmitReceipt::encode_call(slot, index, receipt),
            gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        );
    let mut events = Vec::new();
    let mut reply = None;
    for _ in 0..20 {
        let result = remoting.system().run_next_block();
        for entry in result.log() {
            if entry.reply_to() == Some(request) {
                reply = Some(SubmitReceipt::decode_reply(entry.payload()).unwrap());
            }
            if entry.source() == manager && entry.destination().is_zero() {
                events.push(VftManagerEvents::decode_event(entry.payload()).unwrap());
            }
        }
        if let Some(reply) = reply {
            return (reply, events);
        }
    }
    std::panic!("receipt submission did not reply");
}

#[tokio::test]
async fn test_receipt_resolution_exact_unlock_false_is_terminal_without_transfer() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        ..
    } = setup_for_test().await;
    let token = deploy_vft(&remoting, "Ordinary Gear", "GEAR", 12, b"self-unlock").await;
    let amount = U256::from(17u64);
    VftAdminC::new(remoting.clone())
        .mint(manager, amount)
        .send_recv(token)
        .await
        .unwrap();
    let erc20 = H160([23; 20]);
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .map_vara_to_eth_address(token, erc20, TokenSupply::Gear)
        .send_recv(manager)
        .await
        .unwrap();
    let receipt = deposit_receipt(manager, erc20, amount);
    let (reply, events) = submit_receipt_events(&remoting, manager, 36, 0, receipt.clone());
    assert_eq!(reply, Err(Error::ReceiptResolved));
    assert!(events.is_empty());
    let rows = receipt_deposits(&admin, manager, 36, 0).await;
    assert_eq!(rows.len(), 1);
    let child = rows[0]
        .child
        .expect("exact false must retain the original economic child");
    assert_eq!(
        rows[0].outcome,
        vft_manager_client::ReceiptDepositOutcome::Rejected
    );
    assert_eq!(
        admin
            .receipt_rejection(36, 0, 0)
            .recv(manager)
            .await
            .unwrap(),
        Some(vft_manager_client::ReceiptRejectionEvidence::UnlockFalse { child })
    );
    assert_eq!(
        receipt_status(&admin, manager, 36, 0).await,
        vft_manager_client::ReceiptStatus::Resolved
    );
    assert_eq!(balance_of(&remoting, token, manager).await, amount);
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(token)
            .await
            .unwrap(),
        amount
    );
    let (replay, events) = submit_receipt_events(&remoting, manager, 36, 0, receipt);
    assert_eq!(replay, Err(Error::AlreadyProcessed));
    assert!(events.is_empty());
    assert_eq!(receipt_deposits(&admin, manager, 36, 0).await, rows);
    admin.pause().send_recv(manager).await.unwrap();
    for _ in 0..2 {
        let (resolved, events) = resolve_receipt_events(&remoting, manager, 36, 0, &rows[0]);
        assert_eq!(
            resolved,
            Ok(vft_manager_client::ReceiptDepositOutcome::Rejected)
        );
        assert!(events.is_empty());
    }
    let destination = upgrade_destination(&remoting, manager).await;
    configure_cutover_destination(&remoting, manager, destination).await;
    admin
        .begin_cutover(destination, 40)
        .send_recv(manager)
        .await
        .unwrap();
    admin
        .prepare_cutover(manager, 40)
        .send_recv(destination)
        .await
        .unwrap();
    admin.upgrade(destination).send_recv(manager).await.unwrap();
    admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .unwrap();
    assert_eq!(balance_of(&remoting, token, destination).await, amount);
    assert_eq!(receipt_deposits(&admin, manager, 36, 0).await, rows);
}

#[tokio::test]
async fn test_native_wrapper_error_retries_after_resume() {
    use vft_vara_client::traits::NativeEscrow as _;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        ..
    } = setup_for_test().await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
        .send_recv(manager)
        .await
        .unwrap();
    let mut wrapper_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    let mut escrow = vft_vara_client::NativeEscrow::new(remoting.clone());
    escrow
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin
        .set_burner(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    let raw = 2_000_000_000_000u128;
    let amount = U256::from(raw);
    vft_vara_client::VftNativeExchange::new(remoting.clone())
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    assert!(VftC::new(remoting.clone())
        .transfer(manager, amount)
        .send_recv(native)
        .await
        .unwrap());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    admin.unpause().send_recv(manager).await.unwrap();
    let recipient: ActorId = 100_130.into();
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let native_before = remoting.system().balance_of(recipient);
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, amount);
    let (reply, events) = submit_receipt_events(&remoting, manager, 37, 0, receipt.clone());
    assert!(matches!(reply, Err(Error::ReplyFailure(_))));
    assert!(events.is_empty());
    let row = receipt_deposits(&admin, manager, 37, 0).await.remove(0);
    assert!(row.child.is_some());
    assert_eq!(
        row.outcome,
        vft_manager_client::ReceiptDepositOutcome::Retryable
    );
    assert_eq!(
        receipt_status(&admin, manager, 37, 0).await,
        vft_manager_client::ReceiptStatus::Reserved
    );
    assert_eq!(
        admin
            .receipt_rejection(37, 0, 0)
            .recv(manager)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        escrow
            .redemption(row.operation_id)
            .recv(native)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_eq!(remoting.system().balance_of(recipient), native_before);
    assert_eq!(balance_of(&remoting, native, manager).await, amount);
    admin.pause().send_recv(manager).await.unwrap();
    let (resolved, events) = resolve_receipt_events(&remoting, manager, 37, 0, &row);
    assert_eq!(resolved, Err(Error::InvalidReconciliation));
    assert!(events.is_empty());
    assert_eq!(
        receipt_deposits(&admin, manager, 37, 0).await,
        vec![row.clone()]
    );
    wrapper_admin.resume().send_recv(native).await.unwrap();
    admin.unpause().send_recv(manager).await.unwrap();
    let (reply, events) = submit_receipt_events(&remoting, manager, 37, 0, receipt.clone());
    assert_eq!(reply, Err(Error::NativeSettlementPending));
    assert!(events.is_empty());
    let retried = receipt_deposits(&admin, manager, 37, 0).await.remove(0);
    assert_eq!(retried.operation_id, row.operation_id);
    assert_ne!(retried.child, row.child);
    assert_eq!(
        retried.outcome,
        vft_manager_client::ReceiptDepositOutcome::NativeQueued
    );
    let queued = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.status, vft_vara_client::PayoutStatus::Queued);
    assert_eq!(queued.amount, amount);
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_eq!(balance_of(&remoting, native, manager).await, U256::zero());
    assert_eq!(
        submit_receipt_events(&remoting, manager, 37, 0, receipt.clone()).0,
        Err(Error::NativeSettlementPending)
    );
    assert_eq!(
        escrow
            .redemption(row.operation_id)
            .recv(native)
            .await
            .unwrap(),
        Some(queued.clone())
    );
    assert_eq!(
        receipt_deposits(&admin, manager, 37, 0).await,
        vec![retried.clone()]
    );
    // A saved hold cannot mask delivery before the manager resolves the liability.
    wrapper_admin.pause().send_recv(native).await.unwrap();
    assert_eq!(
        escrow
            .hold_redemption(row.operation_id, queued.clone(), true)
            .send_recv(native)
            .await
            .unwrap(),
        queued
    );
    admin.pause().send_recv(manager).await.unwrap();
    let mailbox = remoting.system().get_mailbox(recipient);
    let payout = Log::builder().source(native).dest(recipient);
    mailbox.claim_value(payout.clone()).unwrap();
    remoting.system().run_next_block();
    let (resolved, events) = resolve_receipt_events(&remoting, manager, 37, 0, &retried);
    assert_eq!(resolved, Err(Error::InvalidReconciliation));
    assert!(events.is_empty());
    assert_eq!(
        receipt_deposits(&admin, manager, 37, 0).await,
        vec![retried]
    );
    for _ in 0..2 {
        assert_eq!(
            admin
                .reconcile_receipt(37, 0)
                .send_recv(manager)
                .await
                .unwrap(),
            Ok(vft_manager_client::ReceiptStatus::Processed)
        );
    }
    let delivered = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.child, queued.child);
    assert_eq!(delivered.status, vft_vara_client::PayoutStatus::Delivered);
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_eq!(remoting.system().balance_of(recipient), native_before + raw);
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    assert!(mailbox.claim_value(payout).is_err());
    assert_eq!(
        escrow
            .held_redemption(row.operation_id)
            .recv(native)
            .await
            .unwrap(),
        Some(queued)
    );
    admin.unpause().send_recv(manager).await.unwrap();
    assert_eq!(
        submit_receipt_events(&remoting, manager, 37, 0, receipt).0,
        Err(Error::AlreadyProcessed)
    );
}

#[tokio::test]
async fn test_native_over_return_refunds_once() {
    use sails_rs::events::EventIo;
    use vft_manager_client::{
        vft_manager::{events::VftManagerEvents, io::ReconcileReceipt},
        ReceiptDepositOutcome, ReceiptStatus,
    };
    use vft_vara_client::{traits::NativeEscrow as _, PayoutStatus};
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        ..
    } = setup_for_test().await;
    let recipient: ActorId = 100_136.into();
    remoting.system().mint_to(recipient, 100_000_000_000_000);
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
        .send_recv(manager)
        .await
        .unwrap();
    let mut native_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    native_admin.pause().send_recv(native).await.unwrap();
    let escrow = vft_vara_client::NativeEscrow::new(remoting.clone());
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
    let raw = 2_000_000_000_000u128;
    let amount = U256::from(raw);
    vft_vara_client::VftNativeExchange::new(remoting.clone())
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    assert!(VftC::new(remoting.clone())
        .transfer(manager, amount)
        .send_recv(native)
        .await
        .unwrap());
    let wrapper_before = remoting.system().balance_of(native);
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, amount);
    let (reply, events) = submit_receipt_events(&remoting, manager, 40, 0, receipt.clone());
    assert_eq!(reply, Err(Error::NativeSettlementPending));
    assert!(events.is_empty());
    let row = receipt_deposits(&admin, manager, 40, 0).await.remove(0);
    let queued = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.status, PayoutStatus::Queued);
    let mailbox = remoting.system().get_mailbox(recipient);
    let payout = Log::builder().source(native).dest(recipient);
    mailbox
        .reply_bytes(payout.clone(), Vec::new(), raw + 1)
        .unwrap();
    remoting.system().run_next_block();
    let backed = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(backed.status, PayoutStatus::Queued);
    assert_eq!(backed.child, queued.child);
    assert_eq!(backed.returned_value, raw + 1);
    assert_eq!(remoting.system().balance_of(native), wrapper_before + 1);
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(native)
            .await
            .unwrap(),
        U256::zero()
    );
    let mut refunds = 0;
    for _ in 0..2 {
        let request = remoting
            .system()
            .get_program(manager)
            .unwrap()
            .send_bytes_with_gas(
                REMOTING_ACTOR_ID,
                ReconcileReceipt::encode_call(40, 0),
                gtest::constants::MAX_USER_GAS_LIMIT,
                0,
            );
        let mut reply = None;
        for _ in 0..20 {
            let result = remoting.system().run_next_block();
            for entry in result.log() {
                if entry.reply_to() == Some(request) {
                    reply = Some(ReconcileReceipt::decode_reply(entry.payload()).unwrap());
                }
                if entry.source() != manager || !entry.destination().is_zero() {
                    continue;
                }
                match VftManagerEvents::decode_event(entry.payload()).unwrap() {
                    VftManagerEvents::ReceiptDepositRefunded {
                        operation_id,
                        child,
                        payout_child,
                        amount: credited,
                        returned_value,
                        receiver,
                        native: is_native,
                        ..
                    } => {
                        refunds += 1;
                        assert_eq!(operation_id, row.operation_id);
                        assert_eq!(Some(child), row.child);
                        assert_eq!(payout_child, queued.child);
                        assert_eq!(
                            (credited, returned_value, receiver),
                            (amount, raw + 1, recipient)
                        );
                        assert!(is_native);
                    }
                    VftManagerEvents::ReceiptDepositSettled { .. }
                    | VftManagerEvents::BridgingAccepted { .. } => {
                        std::panic!("Excess-backed refund is not a native delivery")
                    }
                    _ => {}
                }
            }
            if reply.is_some() {
                break;
            }
        }
        assert_eq!(reply, Some(Ok(ReceiptStatus::Processed)));
        assert_eq!(balance_of(&remoting, native, recipient).await, amount);
        assert_eq!(
            VftC::new(remoting.clone())
                .total_supply()
                .recv(native)
                .await
                .unwrap(),
            amount
        );
        assert_eq!(remoting.system().balance_of(native), wrapper_before + 1);
    }
    assert_eq!(refunds, 1);
    let refunded = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        refunded,
        vft_vara_client::Redemption {
            status: PayoutStatus::Refunded,
            ..backed
        }
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_eq!(escrow.blocking_redemptions().recv(native).await.unwrap(), 0);
    assert_eq!(
        receipt_deposits(&admin, manager, 40, 0).await[0].outcome,
        ReceiptDepositOutcome::Refunded
    );
    assert!(!mailbox.contains(&payout));
    assert!(mailbox.reply_bytes(payout, Vec::new(), raw + 1).is_err());
    let (reply, events) = submit_receipt_events(&remoting, manager, 40, 0, receipt);
    assert_eq!(reply, Err(Error::AlreadyProcessed));
    assert!(events.is_empty());
    assert_eq!(balance_of(&remoting, native, recipient).await, amount);
}

#[tokio::test]
async fn test_receipt_resolution_live_lease_and_malformed_unlock_fail_closed() {
    use vft_client::vft::io::TransferFrom;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        ..
    } = setup_for_test().await;
    let token: ActorId = 100_132.into();
    let recipient: ActorId = 100_133.into();
    for actor in [token, recipient] {
        remoting.system().mint_to(actor, 100_000_000_000_000);
    }
    let erc20 = H160([24; 20]);
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .map_vara_to_eth_address(token, erc20, TokenSupply::Gear)
        .send_recv(manager)
        .await
        .unwrap();
    let manual = remoting
        .clone()
        .with_actor_id(HISTORICAL_PROXY_ID.into())
        .with_block_run_mode(BlockRunMode::Manual);
    let mut proxy = VftManagerC::new(manual.clone());
    let pending = proxy
        .submit_receipt(38, 0, deposit_receipt(recipient, erc20, U256::from(17u64)))
        .send(manager)
        .await
        .unwrap();
    manual.run_next_block();
    let row = receipt_deposits(&admin, manager, 38, 0).await.remove(0);
    assert_eq!(
        row.outcome,
        vft_manager_client::ReceiptDepositOutcome::InFlight
    );
    let mut manual_admin =
        VftManagerC::new(remoting.clone().with_block_run_mode(BlockRunMode::Manual));
    let pause = manual_admin.pause().send(manager).await.unwrap();
    manual.run_next_block();
    pause.recv().await.unwrap();
    assert_eq!(
        resolve_receipt_events(&remoting, manager, 38, 0, &row).0,
        Err(Error::ReceiptLeaseActive)
    );
    assert_eq!(
        receipt_deposits(&admin, manager, 38, 0).await,
        vec![row.clone()]
    );
    let request =
        Log::builder()
            .source(manager)
            .dest(token)
            .payload_bytes(TransferFrom::encode_call(
                manager,
                recipient,
                U256::from(17u64),
            ));
    let mailbox = remoting.system().get_mailbox(token);
    let mut reply = TransferFrom::ROUTE.to_vec();
    false.encode_to(&mut reply);
    reply.push(0xff);
    mailbox.reply_bytes(request.clone(), reply, 0).unwrap();
    for _ in 0..3 {
        manual.run_next_block();
    }
    assert_eq!(
        pending.recv().now_or_never().unwrap().unwrap(),
        Err(Error::InvalidReply)
    );
    let ambiguous = receipt_deposits(&admin, manager, 38, 0).await.remove(0);
    assert_eq!(ambiguous.child, row.child);
    assert_eq!(
        ambiguous.outcome,
        vft_manager_client::ReceiptDepositOutcome::Unknown
    );
    assert_eq!(
        admin
            .resolve_receipt_deposit(38, 0, 0, row.operation_id, row.child)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::InvalidReconciliation)
    );
    assert_eq!(
        receipt_status(&admin, manager, 38, 0).await,
        vft_manager_client::ReceiptStatus::Reserved
    );
    assert_eq!(
        admin
            .receipt_rejection(38, 0, 0)
            .recv(manager)
            .await
            .unwrap(),
        None
    );
    assert!(
        !mailbox.contains(&request),
        "resolution must not retry an ambiguous economic child"
    );
    let destination = upgrade_destination(&remoting, manager).await;
    configure_cutover_destination(&remoting, manager, destination).await;
    admin
        .begin_cutover(destination, 40)
        .send_recv(manager)
        .await
        .unwrap();
    admin
        .prepare_cutover(manager, 40)
        .send_recv(destination)
        .await
        .unwrap();
    assert!(admin.upgrade(destination).send_recv(manager).await.is_err());
    assert_eq!(
        receipt_deposits(&admin, manager, 38, 0).await,
        vec![ambiguous]
    );
    assert_eq!(
        receipt_status(&admin, manager, 38, 0).await,
        vft_manager_client::ReceiptStatus::Reserved
    );
}

#[tokio::test]
async fn test_receipt_resolution_reconciles_native_despite_ambiguous_sibling() {
    use vft_client::vft::io::TransferFrom;
    use vft_manager_client::ReceiptDepositOutcome;
    use vft_vara_client::traits::NativeEscrow as _;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        ..
    } = setup_for_test().await;
    let recipient: ActorId = 100_134.into();
    let token: ActorId = 100_135.into();
    remoting.system().mint_to(recipient, 100_000_000_000_000);
    remoting.system().mint_to(token, 100_000_000_000_000);
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
        .send_recv(manager)
        .await
        .unwrap();
    let mut wrapper_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    let mut wrapper = vft_vara_client::NativeEscrow::new(remoting.clone());
    wrapper
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin
        .set_burner(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    let raw = 2_000_000_000_000u128;
    let amount = U256::from(raw);
    vft_vara_client::VftNativeExchange::new(remoting.clone())
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    assert!(VftC::new(remoting.clone())
        .transfer(manager, amount)
        .send_recv(native)
        .await
        .unwrap());
    let erc20 = H160([25; 20]);
    admin
        .map_vara_to_eth_address(token, erc20, TokenSupply::Gear)
        .send_recv(manager)
        .await
        .unwrap();
    admin.unpause().send_recv(manager).await.unwrap();
    let receipt = crate::create_receipt_rlp_with_logs(
        ERC20_MANAGER_ADDRESS,
        vec![
            (H160([3; 20]), recipient, ERC20_TOKEN_GEAR_SUPPLY, amount),
            (H160([4; 20]), recipient, erc20, U256::from(17u64)),
        ],
    );
    let manual = remoting
        .clone()
        .with_actor_id(HISTORICAL_PROXY_ID.into())
        .with_block_run_mode(BlockRunMode::Manual);
    let mut proxy = VftManagerC::new(manual.clone());
    let pending = proxy
        .submit_receipt(39, 0, receipt)
        .send(manager)
        .await
        .unwrap();
    for _ in 0..5 {
        manual.run_next_block();
    }
    let request =
        Log::builder()
            .source(manager)
            .dest(token)
            .payload_bytes(TransferFrom::encode_call(
                manager,
                recipient,
                U256::from(17u64),
            ));
    let mut reply = TransferFrom::ROUTE.to_vec();
    true.encode_to(&mut reply);
    reply.push(0xff);
    remoting
        .system()
        .get_mailbox(token)
        .reply_bytes(request, reply, 0)
        .unwrap();
    for _ in 0..3 {
        manual.run_next_block();
    }
    assert_eq!(
        pending.recv().now_or_never().unwrap().unwrap(),
        Err(Error::InvalidReply)
    );
    let before = receipt_deposits(&admin, manager, 39, 0).await;
    assert_eq!(before[0].outcome, ReceiptDepositOutcome::NativeQueued);
    assert_eq!(before[1].outcome, ReceiptDepositOutcome::Unknown);
    remoting
        .system()
        .get_mailbox(recipient)
        .claim_value(Log::builder().source(native).dest(recipient))
        .unwrap();
    remoting.system().run_next_block();
    assert_eq!(
        admin
            .reconcile_receipt(39, 0)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::InvalidReply)
    );
    let after = receipt_deposits(&admin, manager, 39, 0).await;
    assert_eq!(after[0].outcome, ReceiptDepositOutcome::Settled);
    assert_eq!(after[0].child, before[0].child);
    assert_eq!(after[1], before[1]);
    assert_eq!(
        receipt_status(&admin, manager, 39, 0).await,
        vft_manager_client::ReceiptStatus::Reserved
    );
    assert_eq!(
        wrapper
            .outstanding_redemptions()
            .recv(native)
            .await
            .unwrap(),
        0
    );
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
}

#[tokio::test]
async fn test_invalid_receipt_inputs_are_terminal_without_economic_children() {
    use vft_manager_client::{vft_manager::events::VftManagerEvents, ReceiptRejectionEvidence};
    use vft_vara_client::traits::NativeEscrow as _;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        eth_supply_vft,
    } = setup_for_test().await;
    let recipient: ActorId = 100_107.into();
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
        .send_recv(manager)
        .await
        .unwrap();
    let mut wrapper_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    let mut wrapper = vft_vara_client::NativeEscrow::new(remoting.clone());
    wrapper
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin
        .set_burner(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    admin.unpause().send_recv(manager).await.unwrap();
    // Real backed custody makes a forbidden burn observable, not merely a
    // failure caused by the manager having no tokens.
    let backing = 1_000_000_000_000u128;
    vft_vara_client::VftNativeExchange::new(remoting.clone())
        .mint()
        .with_value(backing)
        .send_recv(native)
        .await
        .unwrap();
    assert!(VftC::new(remoting.clone())
        .transfer(manager, U256::from(backing))
        .send_recv(native)
        .await
        .unwrap());
    let native_balance = remoting.system().balance_of(native);
    let recipient_balance = remoting.system().balance_of(recipient);
    let inputs = [
        (
            ActorId::zero(),
            ERC20_TOKEN_GEAR_SUPPLY,
            U256::one(),
            ReceiptRejectionEvidence::InvalidRecipient,
        ),
        (
            recipient,
            ERC20_TOKEN_GEAR_SUPPLY,
            U256::zero(),
            ReceiptRejectionEvidence::InvalidAmount,
        ),
        (
            recipient,
            ERC20_TOKEN_GEAR_SUPPLY,
            U256::one() << 80,
            ReceiptRejectionEvidence::NativeAmountOverflow,
        ),
        (
            ActorId::zero(),
            ERC20_TOKEN_ETH_SUPPLY,
            U256::one(),
            ReceiptRejectionEvidence::InvalidRecipient,
        ),
        (
            recipient,
            ERC20_TOKEN_ETH_SUPPLY,
            U256::zero(),
            ReceiptRejectionEvidence::InvalidAmount,
        ),
    ];
    let mut retained = Vec::new();
    for (index, (receiver, token, amount, evidence)) in inputs.into_iter().enumerate() {
        let index = index as u64;
        let receipt = deposit_receipt(receiver, token, amount);
        let (reply, events) = submit_receipt_events(&remoting, manager, 34, index, receipt.clone());
        assert_eq!(reply, Err(Error::ReceiptResolved));
        assert_eq!(
            receipt_status(&admin, manager, 34, index).await,
            vft_manager_client::ReceiptStatus::Resolved
        );
        let rows = receipt_deposits(&admin, manager, 34, index).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].outcome,
            vft_manager_client::ReceiptDepositOutcome::Rejected
        );
        assert_eq!(rows[0].child, None, "invalid input must not dispatch");
        assert_eq!((rows[0].receiver, rows[0].amount), (receiver, amount));
        assert_eq!(rows[0].native, index < 3);
        assert_eq!(
            admin
                .receipt_rejection(34, index, 0)
                .recv(manager)
                .await
                .unwrap(),
            Some(evidence.clone())
        );
        assert_eq!(
            admin
                .receipt_rejection(34, index, 1)
                .recv(manager)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            wrapper
                .redemption(rows[0].operation_id)
                .recv(native)
                .await
                .unwrap(),
            None
        );
        assert!(events.iter().all(|event| !matches!(
            event,
            VftManagerEvents::BridgingAccepted { .. }
                | VftManagerEvents::ReceiptDepositSettled { .. }
                | VftManagerEvents::ReceiptDepositRefunded { .. }
        )));
        let (replay, replay_events) = submit_receipt_events(&remoting, manager, 34, index, receipt);
        assert_eq!(replay, Err(Error::AlreadyProcessed));
        assert!(replay_events.is_empty());
        assert_eq!(receipt_deposits(&admin, manager, 34, index).await, rows);
        retained.push((rows, evidence));
    }
    assert_eq!(
        balance_of(&remoting, native, manager).await,
        U256::from(backing)
    );
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(native)
            .await
            .unwrap(),
        U256::from(backing)
    );
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(eth_supply_vft)
            .await
            .unwrap(),
        U256::zero()
    );
    assert_eq!(remoting.system().balance_of(native), native_balance);
    assert_eq!(remoting.system().balance_of(recipient), recipient_balance);
    assert_eq!(
        wrapper
            .outstanding_redemptions()
            .recv(native)
            .await
            .unwrap(),
        0
    );
    assert!(admin
        .transactions(Order::Direct, 0, 10)
        .recv(manager)
        .await
        .unwrap()
        .is_empty());
    let destination = upgrade_destination(&remoting, manager).await;
    configure_cutover_destination(&remoting, manager, destination).await;
    assert!(
        admin
            .begin_cutover(destination, 33)
            .send_recv(manager)
            .await
            .is_err(),
        "terminal evidence must still contribute to admission high-water"
    );
    admin
        .begin_cutover(destination, 40)
        .send_recv(manager)
        .await
        .unwrap();
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .prepare_cutover(manager, 40)
        .send_recv(destination)
        .await
        .unwrap();
    admin.upgrade(destination).send_recv(manager).await.unwrap();
    wrapper_admin.pause().send_recv(native).await.unwrap();
    wrapper
        .configure_manager(destination)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin
        .set_burner(destination)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .unwrap();
    assert_eq!(
        admin.cutover_status().recv(manager).await.unwrap().phase,
        vft_manager_client::CutoverPhase::Retired
    );
    assert_eq!(
        admin
            .cutover_status()
            .recv(destination)
            .await
            .unwrap()
            .phase,
        vft_manager_client::CutoverPhase::Ready
    );
    assert_eq!(
        balance_of(&remoting, native, destination).await,
        U256::from(backing)
    );
    for (index, (rows, evidence)) in retained.into_iter().enumerate() {
        let index = index as u64;
        assert_eq!(
            receipt_status(&admin, manager, 34, index).await,
            vft_manager_client::ReceiptStatus::Resolved
        );
        assert_eq!(receipt_deposits(&admin, manager, 34, index).await, rows);
        assert_eq!(
            admin
                .receipt_rejection(34, index, 0)
                .recv(manager)
                .await
                .unwrap(),
            Some(evidence)
        );
    }
}

#[tokio::test]
async fn test_invalid_receipt_sibling_does_not_settle_or_strand_valid_retry() {
    use vft_manager_client::vft_manager::events::VftManagerEvents;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        eth_supply_vft: token,
        ..
    } = setup_for_test().await;
    let recipient: ActorId = 100_108.into();
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let amount = U256::from(17u64);
    let receipt = crate::create_receipt_rlp_with_logs(
        ERC20_MANAGER_ADDRESS,
        vec![
            (
                H160([3; 20]),
                ActorId::zero(),
                ERC20_TOKEN_ETH_SUPPLY,
                amount,
            ),
            (H160([4; 20]), recipient, ERC20_TOKEN_ETH_SUPPLY, amount),
        ],
    );
    let admin = VftManagerC::new(remoting.clone());
    let mut token_admin = VftAdminC::new(remoting.clone());
    token_admin
        .set_minter(REMOTING_ACTOR_ID.into())
        .send_recv(token)
        .await
        .unwrap();
    let (failed, events) = submit_receipt_events(&remoting, manager, 35, 0, receipt.clone());
    assert!(matches!(failed, Err(Error::ReplyFailure(_))));
    assert!(events.is_empty());
    assert_eq!(
        receipt_status(&admin, manager, 35, 0).await,
        vft_manager_client::ReceiptStatus::Reserved
    );
    let initial = receipt_deposits(&admin, manager, 35, 0).await;
    assert_eq!(initial.len(), 2);
    assert_eq!(
        initial[0].outcome,
        vft_manager_client::ReceiptDepositOutcome::Rejected
    );
    assert_eq!(
        initial[0].child, None,
        "invalid sibling must never dispatch"
    );
    assert_eq!(
        admin
            .receipt_rejection(35, 0, 0)
            .recv(manager)
            .await
            .unwrap(),
        Some(vft_manager_client::ReceiptRejectionEvidence::InvalidRecipient)
    );
    assert_eq!(
        initial[1].outcome,
        vft_manager_client::ReceiptDepositOutcome::Retryable
    );
    assert_eq!(
        admin
            .receipt_rejection(35, 0, 1)
            .recv(manager)
            .await
            .unwrap(),
        None
    );
    assert!(
        initial[1].child.is_some(),
        "valid sibling must still be attempted"
    );
    assert_eq!(balance_of(&remoting, token, recipient).await, U256::zero());
    token_admin
        .set_minter(manager)
        .send_recv(token)
        .await
        .unwrap();
    let (resolved, events) = submit_receipt_events(&remoting, manager, 35, 0, receipt.clone());
    assert_eq!(resolved, Err(Error::ReceiptResolved));
    assert_eq!(
        receipt_status(&admin, manager, 35, 0).await,
        vft_manager_client::ReceiptStatus::Resolved
    );
    let rows = receipt_deposits(&admin, manager, 35, 0).await;
    assert_eq!(rows[0], initial[0]);
    assert_eq!(
        rows[1].outcome,
        vft_manager_client::ReceiptDepositOutcome::Settled
    );
    assert_eq!(balance_of(&remoting, token, recipient).await, amount);
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(token)
            .await
            .unwrap(),
        amount
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                VftManagerEvents::ReceiptDepositSettled { log_index: 1, .. }
            ))
            .count(),
        1
    );
    assert!(events.iter().all(|event| match event {
        VftManagerEvents::ReceiptDepositSettled { log_index, .. } => *log_index == 1,
        VftManagerEvents::BridgingAccepted { to, .. } => *to == recipient,
        VftManagerEvents::ReceiptDepositRefunded { .. } => false,
        _ => true,
    }));
    let (replay, events) = submit_receipt_events(&remoting, manager, 35, 0, receipt);
    assert_eq!(replay, Err(Error::AlreadyProcessed));
    assert!(events.is_empty());
    assert_eq!(receipt_deposits(&admin, manager, 35, 0).await, rows);
    assert_eq!(balance_of(&remoting, token, recipient).await, amount);
    assert!(admin
        .transactions(Order::Direct, 0, 10)
        .recv(manager)
        .await
        .unwrap()
        .is_empty());
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
    let eth_config = NativeWrapperConfig {
        actor_id: eth_supply_vft,
        ethereum_token: ERC20_TOKEN_ETH_SUPPLY,
        code_id: H256::from(remoting.system().submit_code(vft::WASM_BINARY).into_bytes()),
        mode: NativeWrapperMode::HeldEscrowV2,
    };
    assert!(service
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY
        )))
        .send_recv(manager)
        .await
        .is_err());
    service.pause().send_recv(manager).await.unwrap();
    assert!(service
        .configure_native_wrapper(Some(eth_config.clone()))
        .send_recv(manager)
        .await
        .is_err());
    // A preexisting nonzero-minimum Gear token must fail qualification; never
    // lower its policy here and silently assume previously reaped dust returned.
    service
        .remove_vara_to_eth_address(eth_supply_vft)
        .send_recv(manager)
        .await
        .unwrap();
    service
        .map_vara_to_eth_address(eth_supply_vft, ERC20_TOKEN_ETH_SUPPLY, TokenSupply::Gear)
        .send_recv(manager)
        .await
        .unwrap();
    VftAdminC::new(remoting.clone())
        .set_minimum_balance(U256::one())
        .send_recv(eth_supply_vft)
        .await
        .unwrap();
    assert!(service
        .configure_native_wrapper(Some(eth_config))
        .send_recv(manager)
        .await
        .is_err());
    for approval in [H160::zero(), ERC20_TOKEN_ETH_SUPPLY] {
        assert!(service
            .configure_native_wrapper(Some(held_wrapper_config(&remoting, native, approval)))
            .send_recv(manager)
            .await
            .is_err());
        assert_eq!(service.native_wrapper().recv(manager).await.unwrap(), None);
    }
    assert!(service
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            ActorId::zero(),
            ERC20_TOKEN_GEAR_SUPPLY
        )))
        .send_recv(manager)
        .await
        .is_err());
    service
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
        .send_recv(manager)
        .await
        .unwrap();
    assert_eq!(
        service.native_wrapper().recv(manager).await.unwrap(),
        Some(held_wrapper_config(&remoting, native, ERC20_TOKEN_GEAR_SUPPLY))
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

async fn assert_native_exit_rejected(
    remoting: &GTestRemoting,
    native: ActorId,
    inheritor: ActorId,
) {
    let mut admin = vft_vara_client::VftAdmin::new(remoting.clone());
    admin.pause().send_recv(native).await.unwrap();
    let reserve = remoting.system().balance_of(native);
    let beneficiary_balance = remoting.system().balance_of(inheritor);
    let supply = VftC::new(remoting.clone())
        .total_supply()
        .recv(native)
        .await
        .unwrap();
    assert!(admin.exit(inheritor).send_recv(native).await.is_err());
    assert!(remoting.system().is_active_program(native));
    assert_eq!(remoting.system().inheritor_of(native), None);
    assert_eq!(remoting.system().balance_of(native), reserve);
    assert_eq!(remoting.system().balance_of(inheritor), beneficiary_balance);
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(native)
            .await
            .unwrap(),
        supply
    );
}

#[tokio::test]
async fn test_native_fractional_split_preserves_aggregate_escrow() {
    use vft_vara_client::traits::{NativeEscrow as _, VftExtension as _};
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        ..
    } = setup_for_test().await;
    let sender: ActorId = 100_105.into();
    let recipient: ActorId = 100_106.into();
    remoting.system().mint_to(sender, 100_000_000_000_000);
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
        .send_recv(manager)
        .await
        .unwrap();
    let mut native_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    native_admin.pause().send_recv(native).await.unwrap();
    assert!(native_admin
        .set_minimum_balance(U256::one())
        .send_recv(native)
        .await
        .is_err());
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
    assert_eq!(
        vft_vara_client::VftExtension::new(remoting.clone())
            .minimum_balance()
            .recv(native)
            .await
            .unwrap(),
        U256::zero()
    );
    let total = U256::from(1_500_000_000_000u64);
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(sender))
        .mint()
        .with_value(total.as_u128())
        .send_recv(native)
        .await
        .unwrap();
    VftC::new(remoting.clone().with_actor_id(sender))
        .transfer(manager, total)
        .send_recv(native)
        .await
        .unwrap();
    let mut proxy = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    let initial_native = remoting.system().balance_of(recipient);
    for (index, raw) in [1_000_000_000_000u64, 500_000_000_000]
        .into_iter()
        .enumerate()
    {
        let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, U256::from(raw));
        assert_eq!(
            proxy
                .submit_receipt(33, index as u64, receipt.clone())
                .send_recv(manager)
                .await
                .unwrap(),
            Err(Error::NativeSettlementPending)
        );
        let remaining = if index == 0 {
            U256::from(500_000_000_000u64)
        } else {
            U256::zero()
        };
        assert_eq!(balance_of(&remoting, native, manager).await, remaining);
        assert_eq!(
            VftC::new(remoting.clone())
                .total_supply()
                .recv(native)
                .await
                .unwrap(),
            remaining
        );
        remoting
            .system()
            .get_mailbox(recipient)
            .claim_value(Log::builder().source(native).dest(recipient))
            .unwrap();
        remoting.system().run_next_block();
        assert_eq!(
            admin
                .reconcile_receipt(33, index as u64)
                .send_recv(manager)
                .await
                .unwrap(),
            Ok(vft_manager_client::ReceiptStatus::Processed)
        );
        assert_eq!(
            proxy
                .submit_receipt(33, index as u64, receipt)
                .send_recv(manager)
                .await
                .unwrap(),
            Err(Error::AlreadyProcessed)
        );
    }
    assert_eq!(
        remoting.system().balance_of(recipient),
        initial_native + total.as_u128()
    );
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
    remoting.system().mint_to(manager, 1_000_000_000_000_000);
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    let inheritor: ActorId = 100_099.into();
    remoting.system().mint_to(inheritor, 1_000_000_000_000);
    let raw = 3_000_000_000_000u128;
    let amount = U256::from(raw);
    assert_eq!(
        vft_vara_client::NativeEscrow::new(remoting.clone())
            .outstanding_redemptions()
            .recv(native)
            .await
            .unwrap(),
        0
    );
    assert_native_exit_rejected(&remoting, native, inheritor).await;
    vft_vara_client::VftAdmin::new(remoting.clone())
        .resume()
        .send_recv(native)
        .await
        .unwrap();
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(manager))
        .mint()
        .with_value(raw * 2)
        .send_recv(native)
        .await
        .unwrap();
    assert_native_exit_rejected(&remoting, native, inheritor).await;
    vft_vara_client::VftAdmin::new(remoting.clone())
        .resume()
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
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_native_exit_rejected(&remoting, native, inheritor).await;
    assert_eq!(
        escrow.redemption(id).recv(native).await.unwrap(),
        Some(queued.clone())
    );
    assert!(escrow_admin
        .configure_manager(recipient)
        .send_recv(native)
        .await
        .is_err());
    assert!(escrow_admin
        .hold_redemption(id, queued.clone(), false)
        .send_recv(native)
        .await
        .is_err());
    assert_eq!(
        escrow_admin
            .hold_redemption(id, queued.clone(), true)
            .send_recv(native)
            .await
            .unwrap(),
        queued
    );
    assert_eq!(escrow.blocking_redemptions().recv(native).await.unwrap(), 0);
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_eq!(
        escrow
            .redeem_escrow(id, manager, recipient, amount)
            .send_recv(native)
            .await
            .unwrap(),
        queued
    );
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    escrow_admin
        .configure_manager(recipient)
        .send_recv(native)
        .await
        .unwrap();
    admin.resume().send_recv(native).await.unwrap();
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
    assert_eq!(escrow.blocking_redemptions().recv(native).await.unwrap(), 0);
    assert_eq!(
        escrow.held_redemption(id).recv(native).await.unwrap(),
        Some(queued.clone())
    );
    admin.pause().send_recv(native).await.unwrap();
    assert_eq!(
        escrow_admin
            .hold_redemption(id, queued.clone(), false)
            .send_recv(native)
            .await
            .unwrap(),
        queued
    );
    assert!(escrow_admin
        .hold_redemption(id, delivered.clone(), true)
        .send_recv(native)
        .await
        .is_err());
    escrow_admin
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    admin.resume().send_recv(native).await.unwrap();
    assert_eq!(
        escrow
            .redeem_escrow(id, manager, recipient, amount)
            .send_recv(native)
            .await
            .unwrap(),
        delivered
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_native_exit_rejected(&remoting, native, inheritor).await;
    assert_eq!(
        escrow.redemption(id).recv(native).await.unwrap(),
        Some(delivered.clone())
    );
    escrow_admin
        .configure_manager(recipient)
        .send_recv(native)
        .await
        .unwrap();
    assert_eq!(
        escrow.redemption(id).recv(native).await.unwrap(),
        Some(delivered.clone())
    );
    // The original manager/child remain immutable even across manager rotation.
    assert_eq!(delivered.from, manager);
    escrow_admin
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    admin.resume().send_recv(native).await.unwrap();
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
    assert_eq!(returned.status, PayoutStatus::Queued);
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_eq!(
        balance_of(&remoting, native, rejects_empty_payload).await,
        U256::zero()
    );
    // The callback commits backing independently of credit. Recovery may mint
    // only in a separate retryable command, including while paused.
    admin.pause().send_recv(native).await.unwrap();
    assert!(escrow_admin
        .configure_manager(recipient)
        .send_recv(native)
        .await
        .is_err());
    let refunded = escrow
        .redeem_escrow(returned_id, manager, rejects_empty_payload, amount)
        .send_recv(native)
        .await
        .unwrap();
    assert_eq!(refunded.status, PayoutStatus::Refunded);
    assert_eq!(refunded.child, returned.child);
    assert!(admin.is_paused().recv(native).await.unwrap());
    assert_eq!(returned.returned_value, raw);
    assert_eq!(balance_of(&remoting, native, manager).await, U256::zero());
    assert_eq!(
        balance_of(&remoting, native, rejects_empty_payload).await,
        amount
    );
    assert_eq!(
        escrow
            .redeem_escrow(returned_id, manager, rejects_empty_payload, amount)
            .send_recv(native)
            .await
            .unwrap(),
        refunded
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        0
    );
    assert_native_exit_rejected(&remoting, native, inheritor).await;
    assert_eq!(
        escrow.redemption(returned_id).recv(native).await.unwrap(),
        Some(refunded)
    );
    // An authenticated success reply carrying value is not delivery or refund evidence.
    // Its original child remains an obligation.
    admin.resume().send_recv(native).await.unwrap();
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(manager))
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    let ambiguous_id = H256([42; 32]);
    let ambiguous_queued = escrow
        .redeem_escrow(ambiguous_id, manager, recipient, amount)
        .send_recv(native)
        .await
        .unwrap();
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    let ambiguous_message = Log::builder().source(native).dest(recipient);
    mailbox
        .reply_bytes(ambiguous_message.clone(), Vec::new(), 1_000_000_000_000)
        .unwrap();
    remoting.system().run_next_block();
    let ambiguous = escrow
        .redemption(ambiguous_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ambiguous.child, ambiguous_queued.child);
    assert_eq!(ambiguous.status, PayoutStatus::Ambiguous);
    assert_eq!(ambiguous.returned_value, 1_000_000_000_000);
    assert_eq!(
        escrow
            .redeem_escrow(ambiguous_id, manager, recipient, amount)
            .send_recv(native)
            .await
            .unwrap(),
        ambiguous
    );
    assert!(mailbox
        .reply_bytes(ambiguous_message, Vec::new(), 0)
        .is_err());
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_native_exit_rejected(&remoting, native, inheritor).await;
    assert_eq!(
        escrow.redemption(ambiguous_id).recv(native).await.unwrap(),
        Some(ambiguous.clone())
    );
    admin.pause().send_recv(native).await.unwrap();
    assert!(escrow_admin
        .configure_manager(recipient)
        .send_recv(native)
        .await
        .is_err());
    let supply = VftC::new(remoting.clone())
        .total_supply()
        .recv(native)
        .await
        .unwrap();
    let wrapper_balance = remoting.system().balance_of(native);
    assert_eq!(
        escrow_admin
            .hold_redemption(ambiguous_id, ambiguous.clone(), false)
            .send_recv(native)
            .await
            .unwrap(),
        ambiguous
    );
    assert_eq!(
        escrow_admin
            .hold_redemption(ambiguous_id, ambiguous.clone(), false)
            .send_recv(native)
            .await
            .unwrap(),
        ambiguous
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_eq!(escrow.blocking_redemptions().recv(native).await.unwrap(), 0);
    assert_eq!(
        escrow
            .redeem_escrow(ambiguous_id, manager, recipient, amount)
            .send_recv(native)
            .await
            .unwrap(),
        ambiguous
    );
    escrow_admin
        .configure_manager(recipient)
        .send_recv(native)
        .await
        .unwrap();
    assert_eq!(
        escrow
            .held_redemption(ambiguous_id)
            .recv(native)
            .await
            .unwrap(),
        Some(ambiguous.clone())
    );
    assert_eq!(
        escrow.redemption(ambiguous_id).recv(native).await.unwrap(),
        Some(ambiguous)
    );
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(native)
            .await
            .unwrap(),
        supply
    );
    assert_eq!(remoting.system().balance_of(native), wrapper_balance);
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
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
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
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, amount);
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
        receipt_status(&admin, manager, 31, 1).await,
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
    let before = receipt_deposits(&admin, manager, 31, 1).await;
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
    let resumed = receipt_deposits(&admin, manager, 31, 1).await;
    assert_eq!(resumed[0].child, Some(original_child));
    // Withhold the real wrapper's read-only query until its configured deadline.
    // A failed query must leave both original economic children untouched.
    use vft_manager_client::vft_manager::io::ReconcileReceipt;
    let query = remoting
        .system()
        .get_program(manager)
        .unwrap()
        .send_bytes_with_gas(
            REMOTING_ACTOR_ID,
            ReconcileReceipt::encode_call(31, 1),
            gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        );
    let mut query_admitted = false;
    for gas in (1..=50).map(|i| i * 1_000_000_000) {
        let result = remoting.system().run_next_block_with_allowance(gas);
        assert!(!result.failed.contains(&query));
        if result.gas_burned.contains_key(&query) && !result.not_executed.contains(&query) {
            query_admitted = true;
            break;
        }
    }
    assert!(
        query_admitted,
        "read-only reconciliation was never admitted"
    );
    remoting.system().run_next_block_with_allowance(0);
    remoting.system().run_next_block_with_allowance(0);
    let result = remoting.system().run_next_block();
    let reply = result
        .log()
        .iter()
        .find(|entry| entry.reply_to() == Some(query))
        .expect("configured query timeout must return a terminal reply");
    assert!(matches!(
        ReconcileReceipt::decode_reply(reply.payload()).unwrap(),
        Err(Error::ReplyTimeout(_))
    ));
    assert_eq!(receipt_deposits(&admin, manager, 31, 1).await, resumed);
    assert_eq!(
        wrapper.redemption(operation).recv(native).await.unwrap(),
        Some(payout.clone())
    );
    assert_eq!(balance_of(&remoting, native, manager).await, U256::zero());
    assert_eq!(
        wrapper
            .outstanding_redemptions()
            .recv(native)
            .await
            .unwrap(),
        1
    );
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
    let destination = upgrade_destination(&remoting, manager).await;
    admin.pause().send_recv(manager).await.unwrap();
    assert_cutover_rejected(&remoting, manager, destination, native).await;
    admin.unpause().send_recv(manager).await.unwrap();
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
        receipt_deposits(&admin, manager, 31, 1).await[0].outcome,
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
    // A real program rejecting the original native payout receives exactly backed
    // wrapped tokens, never another native payout or a native-delivered outcome.
    vft_vara_client::VftNativeExchange::new(remoting.clone().with_actor_id(sender))
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    // Draining cannot open another outgoing request. Fund this independent
    // pre-boundary payout by depositing existing wrapped custody directly.
    VftC::new(remoting.clone().with_actor_id(sender))
        .transfer(manager, amount)
        .send_recv(native)
        .await
        .unwrap();
    let failed_receipt = deposit_receipt(failing_recipient, ERC20_TOKEN_GEAR_SUPPLY, amount);
    assert_eq!(
        proxy
            .submit_receipt(32, 1, failed_receipt.clone())
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::NativeSettlementPending)
    );
    let failed_row = receipt_deposits(&admin, manager, 32, 1).await.remove(0);
    let returned = wrapper
        .redemption(failed_row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned.status, vft_vara_client::PayoutStatus::Queued);
    assert_eq!(
        balance_of(&remoting, native, failing_recipient).await,
        U256::zero()
    );
    assert_eq!(returned.returned_value, raw);
    use sails_rs::events::EventIo;
    use vft_manager_client::vft_manager::events::VftManagerEvents;
    let mut refunds = 0;
    for _ in 0..2 {
        let request = remoting
            .system()
            .get_program(manager)
            .unwrap()
            .send_bytes_with_gas(
                sender,
                ReconcileReceipt::encode_call(32, 1),
                gtest::constants::MAX_USER_GAS_LIMIT,
                0,
            );
        let mut reply = None;
        for _ in 0..10 {
            let result = remoting.system().run_next_block();
            for entry in result.log() {
                if entry.reply_to() == Some(request) {
                    reply = Some(ReconcileReceipt::decode_reply(entry.payload()).unwrap());
                }
                if entry.source() != manager || !entry.destination().is_zero() {
                    continue;
                }
                match VftManagerEvents::decode_event(entry.payload()).unwrap() {
                    VftManagerEvents::ReceiptDepositRefunded {
                        slot,
                        transaction_index,
                        log_index,
                        deposit_count,
                        operation_id,
                        eth_token_id,
                        vara_token_id,
                        sender: eth_sender,
                        receiver,
                        amount: refunded_amount,
                        native: native_outcome,
                        child,
                        payout_child,
                        returned_value,
                    } => {
                        refunds += 1;
                        assert_eq!(
                            (slot, transaction_index, log_index, deposit_count),
                            (32, 1, 0, 1)
                        );
                        assert_eq!(
                            (operation_id, eth_token_id, vara_token_id),
                            (failed_row.operation_id, ERC20_TOKEN_GEAR_SUPPLY, native)
                        );
                        assert_eq!(
                            (eth_sender, receiver, refunded_amount),
                            (H160([3; 20]), failing_recipient, amount)
                        );
                        assert!(native_outcome);
                        assert_eq!(Some(child), failed_row.child);
                        assert_eq!((payout_child, returned_value), (returned.child, raw));
                    }
                    VftManagerEvents::ReceiptDepositSettled { .. }
                    | VftManagerEvents::BridgingAccepted { .. } => {
                        std::panic!("Wrapped refund misclassified as native delivery")
                    }
                    _ => {}
                }
            }
            if reply.is_some() {
                break;
            }
        }
        assert_eq!(
            reply,
            Some(Ok(vft_manager_client::ReceiptStatus::Processed))
        );
    }
    assert_eq!(refunds, 1, "Original refund event must be emitted once");
    assert_eq!(
        receipt_status(&admin, manager, 32, 1).await,
        vft_manager_client::ReceiptStatus::Processed
    );
    assert_eq!(
        receipt_deposits(&admin, manager, 32, 1).await[0].outcome,
        vft_manager_client::ReceiptDepositOutcome::Refunded
    );
    assert_eq!(
        proxy
            .submit_receipt(32, 1, failed_receipt)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::AlreadyProcessed)
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
    assert_eq!(
        balance_of(&remoting, native, failing_recipient).await,
        amount
    );
    assert_eq!(
        wrapper
            .outstanding_redemptions()
            .recv(native)
            .await
            .unwrap(),
        0
    );
    admin.pause().send_recv(manager).await.unwrap();
    admin.upgrade(destination).send_recv(manager).await.unwrap();
    assert_eq!(
        admin.cutover_status().recv(manager).await.unwrap().phase,
        vft_manager_client::CutoverPhase::Retired
    );
    native_admin.pause().send_recv(native).await.unwrap();
    vft_vara_client::NativeEscrow::new(remoting.clone())
        .configure_manager(destination)
        .send_recv(native)
        .await
        .unwrap();
    native_admin
        .set_burner(destination)
        .send_recv(native)
        .await
        .unwrap();
    native_admin.resume().send_recv(native).await.unwrap();
    admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .unwrap();
    admin.unpause().send_recv(destination).await.unwrap();
    assert_eq!(
        admin
            .cutover_status()
            .recv(destination)
            .await
            .unwrap()
            .phase,
        vft_manager_client::CutoverPhase::Active
    );
    assert_eq!(
        wrapper.manager().recv(native).await.unwrap(),
        Some(destination)
    );
    assert_eq!(
        balance_of(&remoting, native, failing_recipient).await,
        amount
    );
    assert_eq!(
        wrapper
            .redemption(failed_row.operation_id)
            .recv(native)
            .await
            .unwrap(),
        Some(vft_vara_client::Redemption {
            status: vft_vara_client::PayoutStatus::Refunded,
            ..returned
        })
    );
}

async fn upgrade_destination(remoting: &GTestRemoting, manager: ActorId) -> ActorId {
    upgrade_destination_with_salt(remoting, manager, b"quiescent-upgrade").await
}

async fn upgrade_destination_with_salt(
    remoting: &GTestRemoting,
    manager: ActorId,
    salt: &[u8],
) -> ActorId {
    let config = VftManagerC::new(remoting.clone())
        .get_config()
        .recv(manager)
        .await
        .unwrap();
    VftManagerFactoryC::new(remoting.clone())
        .new(InitConfig {
            gear_bridge_builtin: BRIDGE_BUILTIN_ID.into(),
            historical_proxy_address: HISTORICAL_PROXY_ID.into(),
            config,
        })
        .send_recv(
            remoting.system().submit_code(vft_manager::WASM_BINARY),
            salt,
        )
        .await
        .unwrap()
}

async fn configure_cutover_destination(
    remoting: &GTestRemoting,
    source: ActorId,
    destination: ActorId,
) {
    let mut admin = VftManagerC::new(remoting.clone());
    if let Some(erc20) = admin.erc_20_manager_address().recv(source).await.unwrap() {
        admin
            .update_erc_20_manager_address(erc20)
            .send_recv(destination)
            .await
            .unwrap();
    }
    let mappings = admin.vara_to_eth_addresses().recv(source).await.unwrap();
    for (token, erc20, supply) in &mappings {
        admin
            .map_vara_to_eth_address(*token, *erc20, supply.clone())
            .send_recv(destination)
            .await
            .unwrap();
    }
    let wrapper = admin
        .native_wrapper()
        .recv(source)
        .await
        .unwrap();
    admin
        .configure_native_wrapper(wrapper)
        .send_recv(destination)
        .await
        .unwrap();
}

async fn assert_cutover_rejected(
    remoting: &GTestRemoting,
    manager: ActorId,
    destination: ActorId,
    token: ActorId,
) {
    let mut admin = VftManagerC::new(remoting.clone());
    if admin.cutover_status().recv(manager).await.unwrap().phase
        == vft_manager_client::CutoverPhase::Active
    {
        configure_cutover_destination(remoting, manager, destination).await;
        admin
            .begin_cutover(destination, 1000)
            .send_recv(manager)
            .await
            .unwrap();
        admin
            .prepare_cutover(manager, 1000)
            .send_recv(destination)
            .await
            .unwrap();
    }
    let before = balance_of(remoting, token, manager).await;
    assert!(admin.upgrade(destination).send_recv(manager).await.is_err());
    assert!(admin.is_paused().recv(manager).await.unwrap());
    assert_eq!(balance_of(remoting, token, manager).await, before);
    assert_eq!(balance_of(remoting, token, destination).await, U256::zero());
    assert_eq!(
        admin.cutover_status().recv(manager).await.unwrap().phase,
        vft_manager_client::CutoverPhase::Draining
    );
    assert_eq!(
        receipt_status(&admin, manager, 999, 0).await,
        vft_manager_client::ReceiptStatus::Unknown
    );
}

#[tokio::test]
async fn test_upgrade_rejects_pending_receipt_and_retains_original_evidence() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        eth_supply_vft: token,
        ..
    } = setup_for_test().await;
    let recipient: ActorId = 100_011.into();
    mint_eth_supply_tokens(&remoting, manager, token, recipient, U256::from(7u32), 0).await;
    VftAdminC::new(remoting.clone())
        .set_minter(ActorId::zero())
        .send_recv(token)
        .await
        .unwrap();
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_ETH_SUPPLY, U256::from(11u32));
    assert!(
        VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
            .submit_receipt(1, 0, receipt)
            .send_recv(manager)
            .await
            .unwrap()
            .is_err()
    );
    let destination = upgrade_destination(&remoting, manager).await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    assert_cutover_rejected(&remoting, manager, destination, token).await;
    assert_eq!(
        receipt_status(&admin, manager, 1, 0).await,
        vft_manager_client::ReceiptStatus::Reserved
    );
    for order in [Order::Direct, Order::Reverse] {
        assert_eq!(
            admin
                .transactions(order, 0, 10)
                .recv(manager)
                .await
                .unwrap(),
            vec![(0, 0)]
        );
    }
    // Rejected cutovers do not fence recovery on the original manager.
    admin.unpause().send_recv(manager).await.unwrap();
}

#[tokio::test]
async fn test_non_native_unrepresentable_mint_remains_a_cutover_blocker() {
    use vft_manager_client::{ReceiptDepositOutcome, ReceiptStatus};
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        eth_supply_vft: token,
        ..
    } = setup_for_test().await;
    let recipient: ActorId = 100_137.into();
    remoting.system().mint_to(recipient, 1_000_000_000_000);
    VftAdminC::new(remoting.clone())
        .set_minter(manager)
        .send_recv(token)
        .await
        .unwrap();
    let amount = U256::one() << 80;
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_ETH_SUPPLY, amount);
    let (reply, events) = submit_receipt_events(&remoting, manager, 41, 0, receipt);
    assert!(matches!(reply, Err(Error::ReplyFailure(_))));
    assert!(events.is_empty());
    let mut admin = VftManagerC::new(remoting.clone());
    let row = receipt_deposits(&admin, manager, 41, 0).await.remove(0);
    assert!(!row.native);
    assert_eq!(row.supply, TokenSupply::Ethereum);
    assert_eq!(row.amount, amount);
    assert!(row.child.is_some());
    assert_eq!(row.outcome, ReceiptDepositOutcome::Retryable);
    assert_eq!(
        receipt_status(&admin, manager, 41, 0).await,
        ReceiptStatus::Reserved
    );
    assert_eq!(
        admin
            .receipt_rejection(41, 0, 0)
            .recv(manager)
            .await
            .unwrap(),
        None
    );
    assert_eq!(balance_of(&remoting, token, recipient).await, U256::zero());
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(token)
            .await
            .unwrap(),
        U256::zero()
    );
    admin.pause().send_recv(manager).await.unwrap();
    let (resolved, events) = resolve_receipt_events(&remoting, manager, 41, 0, &row);
    assert_eq!(resolved, Err(Error::InvalidReconciliation));
    assert!(events.is_empty());
    assert_eq!(receipt_deposits(&admin, manager, 41, 0).await, vec![row]);
    let destination = upgrade_destination(&remoting, manager).await;
    assert_cutover_rejected(&remoting, manager, destination, token).await;
    assert_eq!(
        receipt_status(&admin, manager, 41, 0).await,
        ReceiptStatus::Reserved
    );
}

#[tokio::test]
async fn test_cutover_retains_custody_domains_and_retired_program_queries() {
    use vft_manager_client::CutoverPhase;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: token,
        eth_supply_vft,
    } = setup_for_test().await;
    let amount = U256::from(1_000_000_000_000u64);
    let sender: ActorId = 100_012.into();
    remoting.system().mint_to(sender, 100_000_000_000_000);
    mint_eth_supply_tokens(
        &remoting,
        manager,
        eth_supply_vft,
        sender,
        U256::from(7u32),
        10,
    )
    .await;
    VftAdminC::new(remoting.clone())
        .mint(manager, amount)
        .send_recv(token)
        .await
        .unwrap();
    let destination = upgrade_destination(&remoting, manager).await;
    configure_cutover_destination(&remoting, manager, destination).await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .begin_cutover(destination, 32)
        .send_recv(manager)
        .await
        .unwrap();
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .prepare_cutover(manager, 32)
        .send_recv(destination)
        .await
        .unwrap();
    assert!(admin.unpause().send_recv(destination).await.is_err());
    let reserve = remoting.system().balance_of(manager);
    admin.upgrade(destination).send_recv(manager).await.unwrap();
    assert!(remoting.system().is_active_program(manager));
    assert_eq!(remoting.system().inheritor_of(manager), None);
    assert_eq!(balance_of(&remoting, token, manager).await, U256::zero());
    assert_eq!(balance_of(&remoting, token, destination).await, amount);
    assert_eq!(remoting.system().balance_of(manager), reserve);
    let retired = admin.cutover_status().recv(manager).await.unwrap();
    assert_eq!(retired.phase, CutoverPhase::Retired);
    assert_eq!(retired.completed_transfer_count, retired.transfer_count);
    assert_eq!(retired.in_flight_transfer_count, 0);
    assert!(admin.unpause().send_recv(manager).await.is_err());
    assert_eq!(
        admin
            .transactions(Order::Direct, 0, 10)
            .recv(manager)
            .await
            .unwrap(),
        vec![(0, 10)]
    );
    assert_eq!(receipt_deposits(&admin, manager, 0, 10).await.len(), 1);
    assert!(admin
        .transactions(Order::Direct, 0, 10)
        .recv(destination)
        .await
        .unwrap()
        .is_empty());
    let transfers = admin.cutover_transfers(0, 10).recv(manager).await.unwrap();
    assert_eq!(transfers.len(), 1);
    assert_eq!(
        transfers[0].outcome,
        vft_manager_client::CustodyTransferOutcome::Completed
    );
    assert!(admin
        .cutover_transfers(1, 10)
        .recv(manager)
        .await
        .unwrap()
        .is_empty());
    admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .unwrap();
    assert_eq!(
        admin
            .cutover_status()
            .recv(destination)
            .await
            .unwrap()
            .phase,
        CutoverPhase::Ready
    );
    assert!(admin.is_paused().recv(destination).await.unwrap());
    admin.unpause().send_recv(destination).await.unwrap();
    let active = admin.cutover_status().recv(destination).await.unwrap();
    assert_eq!(active.phase, CutoverPhase::Active);
    assert_eq!(active.predecessor, Some(manager));
    assert_eq!(active.minimum_receipt_slot, 33);
    let receipt = deposit_receipt(sender, ERC20_TOKEN_ETH_SUPPLY, U256::from(11u32));
    let mut proxy = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    for slot in [0, 32] {
        assert_eq!(
            proxy
                .submit_receipt(slot, 10, receipt.clone())
                .send_recv(destination)
                .await
                .unwrap(),
            Err(Error::ReceiptOutsideDeployment)
        );
        assert_eq!(
            proxy
                .reconcile_receipt(slot, 10)
                .send_recv(destination)
                .await
                .unwrap(),
            Err(Error::ReceiptOutsideDeployment)
        );
    }
    assert_eq!(
        proxy
            .submit_receipt(33, 10, receipt.clone())
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::Retired)
    );
    assert_eq!(
        proxy
            .reconcile_receipt(0, 10)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::Retired)
    );
    VftAdminC::new(remoting.clone())
        .set_minter(destination)
        .send_recv(eth_supply_vft)
        .await
        .unwrap();
    proxy
        .submit_receipt(33, 10, receipt)
        .send_recv(destination)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, eth_supply_vft, sender).await,
        U256::from(18u32)
    );
    let later = upgrade_destination_with_salt(&remoting, destination, b"later-cutover").await;
    admin
        .begin_cutover(later, 64)
        .send_recv(destination)
        .await
        .unwrap();
    let draining = admin.cutover_status().recv(destination).await.unwrap();
    assert_eq!(draining.predecessor, Some(manager));
    assert_eq!(draining.minimum_receipt_slot, 33);
    assert_eq!(draining.last_receipt_slot, Some(64));
}

#[tokio::test]
async fn test_upgrade_rejects_failed_refund_until_original_request_is_drained() {
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        eth_supply_vft: token,
        ..
    } = setup_for_test_with_builtin(Some(ReplyBehavior::Rejected), 100).await;
    let sender: ActorId = 100_013.into();
    remoting.system().mint_to(sender, 100_000_000_000_000);
    let amount = U256::from(17u32);
    mint_eth_supply_tokens(&remoting, manager, token, sender, amount, 0).await;
    let mut token_admin = VftAdminC::new(remoting.clone());
    token_admin
        .set_minter(ActorId::zero())
        .send_recv(token)
        .await
        .unwrap();
    assert!(VftManagerC::new(remoting.clone().with_actor_id(sender))
        .request_bridging(token, amount, ETH_TOKEN_RECEIVER)
        .send_recv(manager)
        .await
        .is_err());
    let mut admin = VftManagerC::new(remoting.clone());
    let (request, info) = admin
        .request_briding_msg_tracker_state(0, 100)
        .recv(manager)
        .await
        .unwrap()
        .into_iter()
        .find(|(_, info)| info.details.sender == sender)
        .unwrap();
    assert_eq!(info.status, MessageStatus::TokensReturnComplete(false));
    let destination = upgrade_destination(&remoting, manager).await;
    admin.pause().send_recv(manager).await.unwrap();
    assert_cutover_rejected(&remoting, manager, destination, token).await;
    token_admin
        .set_minter(manager)
        .send_recv(token)
        .await
        .unwrap();
    admin.unpause().send_recv(manager).await.unwrap();
    admin
        .handle_request_bridging_interrupted_transfer(request)
        .send_recv(manager)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(balance_of(&remoting, token, sender).await, amount);
    admin.pause().send_recv(manager).await.unwrap();
    admin.upgrade(destination).send_recv(manager).await.unwrap();
    assert!(remoting.system().is_active_program(manager));
    assert_eq!(
        admin.cutover_status().recv(manager).await.unwrap().phase,
        vft_manager_client::CutoverPhase::Retired
    );
    assert_eq!(remoting.system().inheritor_of(manager), None);
}

#[tokio::test]
async fn test_cutover_draining_slot_boundary_and_economic_freeze() {
    use vft_manager_client::CutoverPhase;
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        eth_supply_vft: token,
        gear_supply_vft,
    } = setup_for_test().await;
    let destination = upgrade_destination(&remoting, manager).await;
    configure_cutover_destination(&remoting, manager, destination).await;
    let mut admin = VftManagerC::new(remoting.clone());
    let config = admin.get_config().recv(manager).await.unwrap();
    admin.pause().send_recv(manager).await.unwrap();
    assert!(admin.pause().send_recv(manager).await.is_err());
    admin
        .begin_cutover(destination, 32)
        .send_recv(manager)
        .await
        .unwrap();
    let intent = admin.cutover_status().recv(manager).await.unwrap();
    assert_eq!(intent.phase, CutoverPhase::Draining);
    let unauthorized: ActorId = 100_042.into();
    remoting.system().mint_to(unauthorized, 100_000_000_000_000);
    let mut stranger = VftManagerC::new(remoting.clone().with_actor_id(unauthorized));
    assert!(stranger.pause().send_recv(manager).await.is_err());
    admin.pause().send_recv(manager).await.unwrap();
    admin.pause().send_recv(manager).await.unwrap();
    assert!(admin.is_paused().recv(manager).await.unwrap());
    assert_eq!(admin.cutover_status().recv(manager).await.unwrap(), intent);
    admin.unpause().send_recv(manager).await.unwrap();
    admin
        .begin_cutover(destination, 32)
        .send_recv(manager)
        .await
        .unwrap();
    assert!(admin
        .begin_cutover(destination, 33)
        .send_recv(manager)
        .await
        .is_err());
    assert_eq!(admin.cutover_status().recv(manager).await.unwrap(), intent);
    assert!(admin
        .update_erc_20_manager_address(H160([8; 20]))
        .send_recv(manager)
        .await
        .is_err());
    assert!(admin
        .update_historical_proxy_address(ActorId::zero())
        .send_recv(manager)
        .await
        .is_err());
    assert!(admin
        .configure_native_wrapper(None)
        .send_recv(manager)
        .await
        .is_err());
    assert!(admin
        .map_vara_to_eth_address(999.into(), H160([9; 20]), TokenSupply::Gear)
        .send_recv(manager)
        .await
        .is_err());
    assert!(admin
        .remove_vara_to_eth_address(token)
        .send_recv(manager)
        .await
        .is_err());
    assert!(admin
        .update_config(config.clone())
        .send_recv(manager)
        .await
        .is_err());
    assert!(admin.update_vfts(vec![]).send_recv(manager).await.is_err());
    assert!(admin.fill_transactions().send_recv(manager).await.is_err());
    assert!(admin
        .calculate_gas_for_reply(32, 99, TokenSupply::Gear)
        .send_recv(manager)
        .await
        .is_err());
    assert!(admin
        .calculate_gas_for_token_map_swap()
        .send_recv(manager)
        .await
        .is_err());
    admin
        .set_pause_admin(remoting.actor_id())
        .send_recv(manager)
        .await
        .unwrap();
    admin
        .add_emergency_stop_observer(remoting.actor_id())
        .send_recv(manager)
        .await
        .unwrap();
    assert_eq!(admin.cutover_status().recv(manager).await.unwrap(), intent);
    assert_eq!(
        admin
            .request_bridging(gear_supply_vft, U256::one(), ETH_TOKEN_RECEIVER)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::CutoverNotReady)
    );
    let recipient: ActorId = 100_100.into();
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_ETH_SUPPLY, U256::from(17u32));
    let mut proxy = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()));
    assert_eq!(
        proxy
            .submit_receipt(33, 1, vec![0xff])
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::ReceiptOutsideDeployment)
    );
    assert_eq!(
        proxy
            .reconcile_receipt(33, 1)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::ReceiptOutsideDeployment)
    );
    proxy
        .submit_receipt(32, 1, receipt.clone())
        .send_recv(manager)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        balance_of(&remoting, token, recipient).await,
        U256::from(17u32)
    );
    admin.pause().send_recv(manager).await.unwrap();
    assert_eq!(
        proxy
            .submit_receipt(32, 2, receipt.clone())
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::Paused)
    );
    admin.unpause().send_recv(manager).await.unwrap();
    assert_eq!(
        admin
            .request_bridging(gear_supply_vft, U256::one(), ETH_TOKEN_RECEIVER)
            .send_recv(manager)
            .await
            .unwrap(),
        Err(Error::CutoverNotReady)
    );
    admin
        .prepare_cutover(manager, 32)
        .send_recv(destination)
        .await
        .unwrap();
    for call in [
        proxy
            .submit_receipt(32, 2, receipt.clone())
            .send_recv(destination)
            .await
            .unwrap(),
        proxy
            .reconcile_receipt(32, 2)
            .send_recv(destination)
            .await
            .unwrap()
            .map(|_| ()),
    ] {
        assert_eq!(call, Err(Error::ReceiptOutsideDeployment));
    }
    assert_eq!(
        proxy
            .submit_receipt(33, 2, receipt)
            .send_recv(destination)
            .await
            .unwrap(),
        Err(Error::CutoverNotReady)
    );
    assert!(admin.unpause().send_recv(destination).await.is_err());
    assert_eq!(
        admin
            .cutover_status()
            .recv(destination)
            .await
            .unwrap()
            .phase,
        CutoverPhase::AwaitingSource
    );
    assert!(admin
        .finalize_cutover()
        .send_recv(destination)
        .await
        .is_err());
}

#[tokio::test]
async fn test_begin_cutover_rejects_boundary_below_processed_or_reserved_high_water() {
    for settled in [true, false] {
        let Fixture {
            remoting,
            vft_manager_program_id: manager,
            eth_supply_vft: token,
            ..
        } = setup_for_test().await;
        let mut admin = VftManagerC::new(remoting.clone());
        let destination = upgrade_destination(&remoting, manager).await;
        if !settled {
            VftAdminC::new(remoting.clone())
                .set_minter(ActorId::zero())
                .send_recv(token)
                .await
                .unwrap();
        }
        let receipt = deposit_receipt(100_101.into(), ERC20_TOKEN_ETH_SUPPLY, U256::from(17u32));
        let result = VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
            .submit_receipt(101, 0, receipt)
            .send_recv(manager)
            .await
            .unwrap();
        assert_eq!(result.is_ok(), settled);
        let before = admin.cutover_status().recv(manager).await.unwrap();
        assert!(admin
            .begin_cutover(destination, 100)
            .send_recv(manager)
            .await
            .is_err());
        assert_eq!(admin.cutover_status().recv(manager).await.unwrap(), before);
        assert_eq!(before.phase, vft_manager_client::CutoverPhase::Active);
        assert!(admin
            .begin_cutover(destination, u64::MAX)
            .send_recv(manager)
            .await
            .is_err());
        admin
            .begin_cutover(destination, 101)
            .send_recv(manager)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_prepare_cutover_rejects_mismatched_fingerprint_and_used_destination() {
    for mismatch in 0..5 {
        let Fixture {
            remoting,
            vft_manager_program_id: manager,
            eth_supply_vft: token,
            gear_supply_vft,
            ..
        } = setup_for_test().await;
        let destination = upgrade_destination(&remoting, manager).await;
        configure_cutover_destination(&remoting, manager, destination).await;
        let mut admin = VftManagerC::new(remoting.clone());
        match mismatch {
            0 => {
                admin
                    .remove_vara_to_eth_address(token)
                    .send_recv(destination)
                    .await
                    .unwrap();
            }
            1 => {
                admin
                    .update_historical_proxy_address(999.into())
                    .send_recv(destination)
                    .await
                    .unwrap();
            }
            2 => {
                admin
                    .update_erc_20_manager_address(H160([9; 20]))
                    .send_recv(destination)
                    .await
                    .unwrap();
            }
            3 => {
                admin
                    .configure_native_wrapper(Some(held_wrapper_config(
                        &remoting,
                        gear_supply_vft,
                        ERC20_TOKEN_GEAR_SUPPLY,
                    )))
                    .send_recv(destination)
                    .await
                    .unwrap();
            }
            _ => {
                let mut config = admin.get_config().recv(destination).await.unwrap();
                config.fee_incoming += 1;
                admin
                    .update_config(config)
                    .send_recv(destination)
                    .await
                    .unwrap();
            }
        }
        admin
            .begin_cutover(destination, 32)
            .send_recv(manager)
            .await
            .unwrap();
        assert!(admin
            .prepare_cutover(manager, 32)
            .send_recv(destination)
            .await
            .is_err());
        assert_eq!(
            admin
                .cutover_status()
                .recv(destination)
                .await
                .unwrap()
                .phase,
            vft_manager_client::CutoverPhase::Active
        );
        admin.pause().send_recv(manager).await.unwrap();
        assert!(admin.upgrade(destination).send_recv(manager).await.is_err());
        assert!(admin
            .cutover_transfers(0, 10)
            .recv(manager)
            .await
            .unwrap()
            .is_empty());
    }
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        eth_supply_vft: token,
        ..
    } = setup_for_test().await;
    let destination = upgrade_destination(&remoting, manager).await;
    configure_cutover_destination(&remoting, manager, destination).await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin.unpause().send_recv(destination).await.unwrap();
    VftAdminC::new(remoting.clone())
        .set_minter(destination)
        .send_recv(token)
        .await
        .unwrap();
    let receipt = deposit_receipt(100_102.into(), ERC20_TOKEN_ETH_SUPPLY, U256::one());
    VftManagerC::new(remoting.clone().with_actor_id(HISTORICAL_PROXY_ID.into()))
        .submit_receipt(0, 0, receipt)
        .send_recv(destination)
        .await
        .unwrap()
        .unwrap();
    admin.pause().send_recv(destination).await.unwrap();
    admin
        .begin_cutover(destination, 32)
        .send_recv(manager)
        .await
        .unwrap();
    assert!(admin
        .prepare_cutover(manager, 32)
        .send_recv(destination)
        .await
        .is_err());
}

#[tokio::test]
async fn test_prepare_and_upgrade_recheck_concurrent_pause_and_revision() {
    for unpause in [true, false] {
        let Fixture {
            remoting,
            vft_manager_program_id: manager,
            ..
        } = setup_for_test().await;
        let destination = upgrade_destination(&remoting, manager).await;
        configure_cutover_destination(&remoting, manager, destination).await;
        let mut admin = VftManagerC::new(remoting.clone());
        admin
            .begin_cutover(destination, 32)
            .send_recv(manager)
            .await
            .unwrap();
        let mut manual =
            VftManagerC::new(remoting.clone().with_block_run_mode(BlockRunMode::Manual));
        let mut preparing_client =
            VftManagerC::new(remoting.clone().with_block_run_mode(BlockRunMode::Manual));
        let preparing = preparing_client
            .prepare_cutover(manager, 32)
            .send(destination)
            .await
            .unwrap();
        let competing = if unpause {
            manual
                .unpause()
                .send(destination)
                .await
                .unwrap()
                .recv()
                .left_future()
        } else {
            manual
                .update_historical_proxy_address(999.into())
                .send(destination)
                .await
                .unwrap()
                .recv()
                .right_future()
        };
        for _ in 0..6 {
            remoting.run_next_block();
        }
        competing.now_or_never().unwrap().unwrap();
        assert!(preparing.recv().now_or_never().unwrap().is_err());
        assert_eq!(
            admin
                .cutover_status()
                .recv(destination)
                .await
                .unwrap()
                .phase,
            vft_manager_client::CutoverPhase::Active
        );
    }
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        ..
    } = setup_for_test().await;
    let destination = upgrade_destination(&remoting, manager).await;
    configure_cutover_destination(&remoting, manager, destination).await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .begin_cutover(destination, 32)
        .send_recv(manager)
        .await
        .unwrap();
    admin
        .prepare_cutover(manager, 32)
        .send_recv(destination)
        .await
        .unwrap();
    admin.pause().send_recv(manager).await.unwrap();
    let mut manual = VftManagerC::new(remoting.clone().with_block_run_mode(BlockRunMode::Manual));
    let mut upgrading_client =
        VftManagerC::new(remoting.clone().with_block_run_mode(BlockRunMode::Manual));
    let upgrade = upgrading_client
        .upgrade(destination)
        .send(manager)
        .await
        .unwrap();
    let unpause = manual.unpause().send(manager).await.unwrap();
    for _ in 0..8 {
        remoting.run_next_block();
    }
    unpause.recv().now_or_never().unwrap().unwrap();
    assert!(upgrade.recv().now_or_never().unwrap().is_err());
    assert_eq!(
        admin.cutover_status().recv(manager).await.unwrap().phase,
        vft_manager_client::CutoverPhase::Draining
    );
    assert!(admin
        .cutover_transfers(0, 10)
        .recv(manager)
        .await
        .unwrap()
        .is_empty());
}

// Managers remain real WASM; this token endpoint is an adversarial user mailbox
// so the exact original balance/transfer replies can be delayed independently.
async fn custody_mailbox_fixture() -> (GTestRemoting, ActorId, ActorId, ActorId) {
    let Fixture {
        remoting,
        vft_manager_program_id: source,
        gear_supply_vft,
        eth_supply_vft,
    } = setup_for_test().await;
    let token: ActorId = 100_110.into();
    remoting.system().mint_to(token, 100_000_000_000_000);
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .remove_vara_to_eth_address(gear_supply_vft)
        .send_recv(source)
        .await
        .unwrap();
    admin
        .remove_vara_to_eth_address(eth_supply_vft)
        .send_recv(source)
        .await
        .unwrap();
    admin
        .map_vara_to_eth_address(token, H160([21; 20]), TokenSupply::Gear)
        .send_recv(source)
        .await
        .unwrap();
    let mut config = admin.get_config().recv(source).await.unwrap();
    config.reply_timeout = 10;
    admin.update_config(config).send_recv(source).await.unwrap();
    let destination = upgrade_destination(&remoting, source).await;
    configure_cutover_destination(&remoting, source, destination).await;
    admin
        .begin_cutover(destination, 32)
        .send_recv(source)
        .await
        .unwrap();
    admin.pause().send_recv(source).await.unwrap();
    admin
        .prepare_cutover(source, 32)
        .send_recv(destination)
        .await
        .unwrap();
    (remoting, source, destination, token)
}

fn start_custody_upgrade(
    remoting: &GTestRemoting,
    source: ActorId,
    destination: ActorId,
) -> MessageId {
    remoting
        .system()
        .get_program(source)
        .unwrap()
        .send_bytes_with_gas(
            REMOTING_ACTOR_ID,
            vft_manager_client::vft_manager::io::Upgrade::encode_call(destination),
            gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        )
}

fn reply_custody_balance(remoting: &GTestRemoting, source: ActorId, token: ActorId, amount: U256) {
    use vft_client::vft::io::BalanceOf;
    let mailbox = remoting.system().get_mailbox(token);
    let request = Log::builder()
        .source(source)
        .dest(token)
        .payload_bytes(BalanceOf::encode_call(source));
    for _ in 0..12 {
        if mailbox.contains(&request) {
            let mut bytes = BalanceOf::ROUTE.to_vec();
            amount.encode_to(&mut bytes);
            mailbox.reply_bytes(request, bytes, 0).unwrap();
            remoting.system().run_next_block();
            return;
        }
        remoting.system().run_next_block();
    }
    std::panic!("original custody balance query did not reach token mailbox");
}

fn custody_transfer_log(
    source: ActorId,
    destination: ActorId,
    token: ActorId,
    amount: U256,
) -> Log {
    Log::builder().source(source).dest(token).payload_bytes(
        vft_client::vft::io::Transfer::encode_call(destination, amount),
    )
}

#[tokio::test]
async fn test_custody_timeout_late_reply_and_concurrent_upgrade_keep_one_original_child() {
    use vft_manager_client::{CustodyTransferOutcome, CutoverPhase};
    let (remoting, source, destination, token) = custody_mailbox_fixture().await;
    let amount = U256::from(17u32);
    let first = start_custody_upgrade(&remoting, source, destination);
    let _concurrent = start_custody_upgrade(&remoting, source, destination);
    reply_custody_balance(&remoting, source, token, amount);
    reply_custody_balance(&remoting, source, token, amount);
    let admin = VftManagerC::new(remoting.clone());
    let before = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].outcome, CustodyTransferOutcome::InFlight);
    assert_eq!(before[0].token, token);
    assert_eq!(before[0].amount, amount);
    assert_eq!(
        admin
            .cutover_status()
            .recv(source)
            .await
            .unwrap()
            .in_flight_transfer_count,
        1
    );
    let mut original_parent_finished = false;
    for _ in 0..14 {
        let result = remoting.system().run_next_block();
        if let Some(reply) = result
            .log()
            .iter()
            .find(|entry| entry.reply_to() == Some(first))
        {
            vft_manager_client::vft_manager::io::Upgrade::decode_reply(reply.payload()).unwrap();
            original_parent_finished = true;
        }
    }
    assert!(
        original_parent_finished,
        "original Upgrade must finish at its configured reply deadline"
    );
    let ambiguous = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(ambiguous.len(), 1);
    assert_eq!(ambiguous[0].child, before[0].child);
    assert_eq!(ambiguous[0].outcome, CustodyTransferOutcome::Ambiguous);
    assert!(remoting.system().is_active_program(source));
    assert_eq!(
        admin.cutover_status().recv(source).await.unwrap().phase,
        CutoverPhase::Transferring
    );
    assert!(VftManagerC::new(remoting.clone())
        .unpause()
        .send_recv(source)
        .await
        .is_err());
    start_custody_upgrade(&remoting, source, destination);
    for _ in 0..6 {
        remoting.run_next_block();
    }
    assert_eq!(
        admin.cutover_transfers(0, 10).recv(source).await.unwrap(),
        ambiguous
    );
    let mailbox = remoting.system().get_mailbox(token);
    let transfer = custody_transfer_log(source, destination, token, amount);
    let mut success = vft_client::vft::io::Transfer::ROUTE.to_vec();
    true.encode_to(&mut success);
    mailbox
        .reply_bytes(transfer.clone(), success.clone(), 0)
        .unwrap();
    remoting.system().run_next_block();
    let completed = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(completed[0].child, before[0].child);
    assert_eq!(completed[0].outcome, CustodyTransferOutcome::Completed);
    assert!(mailbox.reply_bytes(transfer, success, 0).is_err());
    assert_eq!(
        admin
            .cutover_status()
            .recv(source)
            .await
            .unwrap()
            .in_flight_transfer_count,
        0
    );
    start_custody_upgrade(&remoting, source, destination);
    reply_custody_balance(&remoting, source, token, U256::zero());
    reply_custody_balance(&remoting, source, token, U256::zero());
    assert_eq!(
        admin.cutover_status().recv(source).await.unwrap().phase,
        CutoverPhase::Retired
    );
    assert!(remoting.system().is_active_program(source));
    // The saved child belongs to the original parent, not any continuation.
    assert_ne!(completed[0].child, first);
}

#[tokio::test]
async fn test_custody_donation_appends_residual_attempt_and_preserves_original() {
    use vft_manager_client::{CustodyTransferOutcome, CutoverPhase};
    let (remoting, source, destination, token) = custody_mailbox_fixture().await;
    let amount = U256::from(17u32);
    let donation = U256::from(3u32);
    let mailbox = remoting.system().get_mailbox(token);
    let mut success = vft_client::vft::io::Transfer::ROUTE.to_vec();
    true.encode_to(&mut success);
    start_custody_upgrade(&remoting, source, destination);
    reply_custody_balance(&remoting, source, token, amount);
    mailbox
        .reply_bytes(
            custody_transfer_log(source, destination, token, amount),
            success.clone(),
            0,
        )
        .unwrap();
    // The completed original has moved its custody, but an unsolicited balance
    // arrives before the final zero check. Retirement must remain blocked.
    reply_custody_balance(&remoting, source, token, donation);
    let admin = VftManagerC::new(remoting.clone());
    let original = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].outcome, CustodyTransferOutcome::Completed);
    assert_eq!(
        admin.cutover_status().recv(source).await.unwrap().phase,
        CutoverPhase::Transferring
    );

    start_custody_upgrade(&remoting, source, destination);
    reply_custody_balance(&remoting, source, token, donation);
    let residual = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(
        residual.len(),
        2,
        "completed custody must not permanently fence residual sweeps"
    );
    assert_eq!(residual[0], original[0]);
    assert_eq!(residual[1].token, token);
    assert_eq!(residual[1].amount, donation);
    assert_ne!(residual[1].child, original[0].child);
    assert_eq!(residual[1].outcome, CustodyTransferOutcome::InFlight);
    start_custody_upgrade(&remoting, source, destination);
    for _ in 0..3 {
        remoting.run_next_block();
    }
    assert_eq!(
        admin.cutover_transfers(0, 10).recv(source).await.unwrap(),
        residual
    );
    mailbox
        .reply_bytes(
            custody_transfer_log(source, destination, token, donation),
            success,
            0,
        )
        .unwrap();
    reply_custody_balance(&remoting, source, token, U256::zero());
    let completed = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(completed[0], original[0]);
    assert_eq!(completed[1].child, residual[1].child);
    assert_eq!(completed[1].outcome, CustodyTransferOutcome::Completed);
    assert_eq!(
        admin.cutover_status().recv(source).await.unwrap().phase,
        CutoverPhase::Retired
    );
}

#[tokio::test]
async fn test_custody_exact_false_archives_before_retry_and_malformed_true_stays_held() {
    for malformed in [false, true] {
        use vft_manager_client::CustodyTransferOutcome;
        let (remoting, source, destination, token) = custody_mailbox_fixture().await;
        let amount = U256::from(17u32);
        start_custody_upgrade(&remoting, source, destination);
        reply_custody_balance(&remoting, source, token, amount);
        let mailbox = remoting.system().get_mailbox(token);
        let mut reply = vft_client::vft::io::Transfer::ROUTE.to_vec();
        malformed.encode_to(&mut reply);
        if malformed {
            reply.push(0xff);
        }
        mailbox
            .reply_bytes(
                custody_transfer_log(source, destination, token, amount),
                reply,
                0,
            )
            .unwrap();
        for _ in 0..3 {
            remoting.run_next_block();
        }
        let admin = VftManagerC::new(remoting.clone());
        let original = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
        assert_eq!(original.len(), 1);
        assert_eq!(
            original[0].outcome,
            if malformed {
                CustodyTransferOutcome::Ambiguous
            } else {
                CustodyTransferOutcome::Rejected
            }
        );
        start_custody_upgrade(&remoting, source, destination);
        if malformed {
            for _ in 0..6 {
                remoting.run_next_block();
            }
            assert_eq!(
                admin.cutover_transfers(0, 10).recv(source).await.unwrap(),
                original
            );
        } else {
            reply_custody_balance(&remoting, source, token, amount);
            let retry = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
            assert_eq!(retry.len(), 2);
            assert_eq!(retry[0], original[0]);
            assert_ne!(retry[0].child, retry[1].child);
            let mut success = vft_client::vft::io::Transfer::ROUTE.to_vec();
            true.encode_to(&mut success);
            mailbox
                .reply_bytes(
                    custody_transfer_log(source, destination, token, amount),
                    success,
                    0,
                )
                .unwrap();
            reply_custody_balance(&remoting, source, token, U256::zero());
            assert_eq!(
                admin.cutover_status().recv(source).await.unwrap().phase,
                vft_manager_client::CutoverPhase::Retired
            );
            let records = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
            assert_eq!(records[0], original[0]);
            assert_eq!(records[1].outcome, CustodyTransferOutcome::Completed);
        }
    }
}

#[tokio::test]
async fn test_custody_explicit_token_error_retries_without_exit_or_losing_attempts() {
    let Fixture {
        remoting,
        vft_manager_program_id: source,
        gear_supply_vft: token,
        ..
    } = setup_for_test().await;
    let amount = U256::from(1_000_000_000_000u64);
    VftAdminC::new(remoting.clone())
        .mint(source, amount)
        .send_recv(token)
        .await
        .unwrap();
    let destination = upgrade_destination(&remoting, source).await;
    configure_cutover_destination(&remoting, source, destination).await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .begin_cutover(destination, 32)
        .send_recv(source)
        .await
        .unwrap();
    admin.pause().send_recv(source).await.unwrap();
    admin
        .prepare_cutover(source, 32)
        .send_recv(destination)
        .await
        .unwrap();
    let mut token_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    token_admin.pause().send_recv(token).await.unwrap();
    admin.upgrade(destination).send_recv(source).await.unwrap();
    let rejected = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(rejected.len(), 1);
    assert_eq!(
        rejected[0].outcome,
        vft_manager_client::CustodyTransferOutcome::Rejected
    );
    assert_eq!(balance_of(&remoting, token, source).await, amount);
    token_admin.resume().send_recv(token).await.unwrap();
    admin.upgrade(destination).send_recv(source).await.unwrap();
    let records = admin.cutover_transfers(0, 10).recv(source).await.unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0], rejected[0]);
    assert_eq!(
        records[1].outcome,
        vft_manager_client::CustodyTransferOutcome::Completed
    );
    assert_ne!(records[0].child, records[1].child);
    assert_eq!(balance_of(&remoting, token, source).await, U256::zero());
    assert_eq!(balance_of(&remoting, token, destination).await, amount);
    assert_eq!(
        admin.cutover_status().recv(source).await.unwrap().phase,
        vft_manager_client::CutoverPhase::Retired
    );
    assert!(remoting.system().is_active_program(source));
}

#[tokio::test]
async fn test_cutover_fingerprint_uses_sorted_mapping_and_fixed_config_encoding() {
    use ethereum_common::{hash_db::Hasher, keccak_hasher::KeccakHasher};
    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        ..
    } = setup_for_test().await;
    let destination = upgrade_destination(&remoting, manager).await;
    let mut admin = VftManagerC::new(remoting.clone());
    let mut mappings = admin.vara_to_eth_addresses().recv(manager).await.unwrap();
    mappings.sort_unstable_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
    let expected = KeccakHasher::hash(
        &(
            *b"vft-manager/cutover-config/v1",
            ActorId::from(BRIDGE_BUILTIN_ID),
            ActorId::from(HISTORICAL_PROXY_ID),
            Some(ERC20_MANAGER_ADDRESS),
            Option::<ActorId>::None,
            admin.get_config().recv(manager).await.unwrap(),
            &mappings,
        )
            .encode(),
    );
    admin
        .begin_cutover(destination, 32)
        .send_recv(manager)
        .await
        .unwrap();
    assert_eq!(
        admin
            .cutover_status()
            .recv(manager)
            .await
            .unwrap()
            .config_fingerprint,
        Some(expected)
    );
}

fn resolve_receipt_events(
    remoting: &GTestRemoting,
    manager: ActorId,
    slot: u64,
    index: u64,
    row: &vft_manager_client::ReceiptDepositState,
) -> (
    Result<vft_manager_client::ReceiptDepositOutcome, Error>,
    Vec<vft_manager_client::vft_manager::events::VftManagerEvents>,
) {
    use sails_rs::events::EventIo;
    use vft_manager_client::vft_manager::{events::VftManagerEvents, io::ResolveReceiptDeposit};
    let request = remoting
        .system()
        .get_program(manager)
        .unwrap()
        .send_bytes_with_gas(
            REMOTING_ACTOR_ID,
            ResolveReceiptDeposit::encode_call(
                slot,
                index,
                row.log_index,
                row.operation_id,
                row.child,
            ),
            gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        );
    let mut events = Vec::new();
    for _ in 0..20 {
        let result = remoting.system().run_next_block();
        let mut reply = None;
        for entry in result.log() {
            if entry.reply_to() == Some(request) {
                reply = Some(ResolveReceiptDeposit::decode_reply(entry.payload()).unwrap());
            }
            if entry.source() == manager && entry.destination().is_zero() {
                events.push(VftManagerEvents::decode_event(entry.payload()).unwrap());
            }
        }
        if let Some(reply) = reply {
            return (reply, events);
        }
    }
    std::panic!("receipt resolution did not reply");
}

#[tokio::test]
async fn test_native_held_and_delivered_first_observed_in_same_block() {
    use sails_rs::events::EventIo;
    use vft_manager_client::{
        vft_manager::{events::VftManagerEvents, io::ResolveReceiptDeposit},
        ReceiptDepositOutcome, ReceiptStatus,
    };
    use vft_vara_client::{traits::NativeEscrow as _, PayoutStatus};

    let Fixture {
        remoting,
        vft_manager_program_id: manager,
        gear_supply_vft: native,
        ..
    } = setup_for_test().await;
    let recipient: ActorId = 100_138.into();
    remoting.system().mint_to(recipient, 100_000_000_000_000);
    let recipient_before = remoting.system().balance_of(recipient);
    let mut admin = VftManagerC::new(remoting.clone());
    admin.pause().send_recv(manager).await.unwrap();
    admin
        .configure_native_wrapper(Some(held_wrapper_config(
            &remoting,
            native,
            ERC20_TOKEN_GEAR_SUPPLY,
        )))
        .send_recv(manager)
        .await
        .unwrap();
    let mut wrapper_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    wrapper_admin.pause().send_recv(native).await.unwrap();
    let mut escrow = vft_vara_client::NativeEscrow::new(remoting.clone());
    escrow
        .configure_manager(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin
        .set_burner(manager)
        .send_recv(native)
        .await
        .unwrap();
    wrapper_admin.resume().send_recv(native).await.unwrap();
    let raw = 2_000_000_000_000u128;
    let amount = U256::from(raw);
    vft_vara_client::VftNativeExchange::new(remoting.clone())
        .mint()
        .with_value(raw)
        .send_recv(native)
        .await
        .unwrap();
    assert!(VftC::new(remoting.clone())
        .transfer(manager, amount)
        .send_recv(native)
        .await
        .unwrap());
    admin.unpause().send_recv(manager).await.unwrap();
    let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, amount);
    let (reply, events) = submit_receipt_events(&remoting, manager, 42, 0, receipt);
    assert_eq!(reply, Err(Error::NativeSettlementPending));
    assert!(events.is_empty());
    let row = receipt_deposits(&admin, manager, 42, 0).await.remove(0);
    assert_eq!(row.outcome, ReceiptDepositOutcome::NativeQueued);
    let saved = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, PayoutStatus::Queued);
    assert_eq!(saved.returned_value, 0);
    assert_eq!(
        (saved.from, saved.to, saved.amount),
        (manager, recipient, amount)
    );
    assert!(row.child.is_some());
    assert_ne!(row.child, Some(saved.child));
    admin.pause().send_recv(manager).await.unwrap();
    wrapper_admin.pause().send_recv(native).await.unwrap();
    assert_eq!(
        escrow
            .hold_redemption(row.operation_id, saved.clone(), true)
            .send_recv(native)
            .await
            .unwrap(),
        saved
    );
    assert_eq!(
        escrow.outstanding_redemptions().recv(native).await.unwrap(),
        1
    );
    assert_eq!(escrow.blocking_redemptions().recv(native).await.unwrap(), 0);
    let wrapper_before = remoting.system().balance_of(native);
    let mailbox = remoting.system().get_mailbox(recipient);
    let payout = Log::builder().source(native).dest(recipient);
    assert!(mailbox.contains(&payout));

    // Drain prior administrative replies before admitting the resolution. The
    // default fixture's 100-block query timeout exceeds the scheduling bound.
    remoting.system().run_next_block();
    let request = remoting
        .system()
        .get_program(manager)
        .unwrap()
        .send_bytes_with_gas(
            REMOTING_ACTOR_ID,
            ResolveReceiptDeposit::encode_call(42, 0, 0, row.operation_id, row.child),
            gtest::constants::MAX_USER_GAS_LIMIT,
            0,
        );
    let mut admitted = false;
    for gas in (1..=50).map(|i| i * 1_000_000_000) {
        let block = remoting.system().run_next_block_with_allowance(gas);
        assert!(!block.failed.contains(&request));
        assert!(block
            .log()
            .iter()
            .all(|entry| entry.reply_to() != Some(request)));
        let height = remoting.system().block_height();
        assert_eq!(
            admin
                .receipt_deposits(42, 0)
                .query_with_message(false)
                .recv(manager)
                .await
                .unwrap(),
            vec![row.clone()]
        );
        assert_eq!(
            admin
                .receipt_status(42, 0)
                .query_with_message(false)
                .recv(manager)
                .await
                .unwrap(),
            ReceiptStatus::Reserved
        );
        assert_eq!(
            escrow
                .redemption(row.operation_id)
                .query_with_message(false)
                .recv(native)
                .await
                .unwrap(),
            Some(saved.clone())
        );
        assert_eq!(
            remoting.system().block_height(),
            height,
            "snapshots must not run a block"
        );
        assert_eq!(remoting.system().balance_of(recipient), recipient_before);
        if block.gas_burned.contains_key(&request) && !block.not_executed.contains(&request) {
            admitted = true;
            break;
        }
    }
    assert!(
        admitted,
        "resolution was not admitted before the terminal block"
    );
    let before = remoting.system().block_height();
    // Resolution has started, but every prior block still exposed NativeQueued
    // and Queued. Queue the original callback now: whether delivery precedes
    // or follows the Held commit, both must first appear in the next block.
    mailbox.claim_value(payout.clone()).unwrap();
    assert_eq!(remoting.system().block_height(), before);
    let terminal_block = remoting.system().run_next_block();
    let height = remoting.system().block_height();
    assert_eq!(height, before + 1);
    let reply = terminal_block
        .log()
        .iter()
        .find(|entry| entry.reply_to() == Some(request))
        .expect("hold resolution must finish in the delivery block");
    assert_eq!(
        ResolveReceiptDeposit::decode_reply(reply.payload()).unwrap(),
        Ok(ReceiptDepositOutcome::Held)
    );
    for entry in terminal_block.log() {
        if entry.source() == manager && entry.destination().is_zero() {
            assert!(!matches!(
                VftManagerEvents::decode_event(entry.payload()).unwrap(),
                VftManagerEvents::BridgingAccepted { .. }
                    | VftManagerEvents::ReceiptDepositSettled { .. }
                    | VftManagerEvents::ReceiptDepositRefunded { .. }
            ));
        }
    }
    let held = admin
        .receipt_deposits(42, 0)
        .query_with_message(false)
        .recv(manager)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        held,
        vft_manager_client::ReceiptDepositState {
            outcome: ReceiptDepositOutcome::Held,
            ..row
        }
    );
    assert_eq!(
        admin
            .receipt_status(42, 0)
            .query_with_message(false)
            .recv(manager)
            .await
            .unwrap(),
        ReceiptStatus::Resolved
    );
    let live = escrow
        .redemption(held.operation_id)
        .query_with_message(false)
        .recv(native)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        live,
        vft_vara_client::Redemption {
            status: PayoutStatus::Delivered,
            ..saved.clone()
        }
    );
    assert_eq!(
        escrow
            .held_redemption(held.operation_id)
            .query_with_message(false)
            .recv(native)
            .await
            .unwrap(),
        Some(saved.clone())
    );
    assert_eq!(
        escrow
            .outstanding_redemptions()
            .query_with_message(false)
            .recv(native)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        escrow
            .blocking_redemptions()
            .query_with_message(false)
            .recv(native)
            .await
            .unwrap(),
        0
    );
    assert_eq!(remoting.system().block_height(), height);
    assert_eq!(
        remoting.system().balance_of(recipient),
        recipient_before + raw
    );
    assert_eq!(remoting.system().balance_of(native), wrapper_before);
    assert_eq!(balance_of(&remoting, native, recipient).await, U256::zero());
    assert_eq!(
        VftC::new(remoting.clone())
            .total_supply()
            .recv(native)
            .await
            .unwrap(),
        U256::zero()
    );
    println!(
        "historical_held_delivery block={height} row=0x{} saved=0x{} live=0x{}",
        hex::encode(held.encode()),
        hex::encode(saved.encode()),
        hex::encode(live.encode())
    );
    // This is an immutable historical exception, never a financial-success
    // rewrite. No earlier observed block exposed Held with a live queued payout.
    for _ in 0..2 {
        let (resolved, events) = resolve_receipt_events(&remoting, manager, 42, 0, &held);
        assert_eq!(resolved, Ok(ReceiptDepositOutcome::Held));
        assert!(events.is_empty());
        assert_eq!(
            admin
                .reconcile_receipt(42, 0)
                .send_recv(manager)
                .await
                .unwrap(),
            Ok(ReceiptStatus::Resolved)
        );
    }
    assert_eq!(
        receipt_deposits(&admin, manager, 42, 0).await,
        vec![held.clone()]
    );
    assert_eq!(
        escrow
            .held_redemption(held.operation_id)
            .recv(native)
            .await
            .unwrap(),
        Some(saved)
    );
    assert_eq!(
        escrow
            .redemption(held.operation_id)
            .recv(native)
            .await
            .unwrap(),
        Some(live)
    );
    assert_eq!(
        remoting.system().balance_of(recipient),
        recipient_before + raw
    );
    assert_eq!(remoting.system().balance_of(native), wrapper_before);
    assert!(!mailbox.contains(&payout));
    assert!(mailbox.claim_value(payout).is_err());
}

#[tokio::test]
async fn test_native_cutover_requires_same_wrapper_handoff_before_ready() {
    use vft_manager_client::{CutoverPhase, ReceiptDepositOutcome, ReceiptStatus};
    use vft_vara_client::{traits::NativeEscrow as _, PayoutStatus};
    // Queued, partial, post-hold full return, and pre-hold full/excess backing.
    for mode in 0..5 {
        let Fixture {
            remoting,
            vft_manager_program_id: source,
            gear_supply_vft: wrapper,
            ..
        } = setup_for_test().await;
        let recipient: ActorId = 100_131.into();
        remoting.system().mint_to(recipient, 100_000_000_000_000);
        let mut admin = VftManagerC::new(remoting.clone());
        admin.pause().send_recv(source).await.unwrap();
        admin
            .configure_native_wrapper(Some(held_wrapper_config(
                &remoting,
                wrapper,
                ERC20_TOKEN_GEAR_SUPPLY,
            )))
            .send_recv(source)
            .await
            .unwrap();
        let mut wrapper_admin = vft_vara_client::VftAdmin::new(remoting.clone());
        wrapper_admin.pause().send_recv(wrapper).await.unwrap();
        let mut escrow = vft_vara_client::NativeEscrow::new(remoting.clone());
        escrow
            .configure_manager(source)
            .send_recv(wrapper)
            .await
            .unwrap();
        wrapper_admin
            .set_burner(source)
            .send_recv(wrapper)
            .await
            .unwrap();
        wrapper_admin.resume().send_recv(wrapper).await.unwrap();
        let raw = 2_000_000_000_000u128;
        let amount = U256::from(raw);
        vft_vara_client::VftNativeExchange::new(remoting.clone())
            .mint()
            .with_value(raw * 2)
            .send_recv(wrapper)
            .await
            .unwrap();
        assert!(VftC::new(remoting.clone())
            .transfer(source, amount * 2)
            .send_recv(wrapper)
            .await
            .unwrap());
        admin.unpause().send_recv(source).await.unwrap();
        let receipt = deposit_receipt(recipient, ERC20_TOKEN_GEAR_SUPPLY, amount);
        let (reply, events) = submit_receipt_events(&remoting, source, 31, 0, receipt.clone());
        assert_eq!(reply, Err(Error::NativeSettlementPending));
        assert!(events.is_empty());
        let row = receipt_deposits(&admin, source, 31, 0).await.remove(0);
        assert_eq!(row.outcome, ReceiptDepositOutcome::NativeQueued);
        let payout_log = Log::builder().source(wrapper).dest(recipient);
        let mailbox = remoting.system().get_mailbox(recipient);
        assert!(mailbox.contains(&payout_log));
        let returned_value = match mode {
            1 => raw / 2,
            3 => raw,
            4 => raw + 1,
            _ => 0,
        };
        if returned_value != 0 {
            mailbox
                .reply_bytes(payout_log.clone(), Vec::new(), returned_value)
                .unwrap();
            remoting.system().run_next_block();
        }
        let snapshot = escrow
            .redemption(row.operation_id)
            .recv(wrapper)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (snapshot.from, snapshot.to, snapshot.amount),
            (source, recipient, amount)
        );
        assert!(!snapshot.child.is_zero());
        assert_ne!(Some(snapshot.child), row.child);
        assert_eq!(snapshot.returned_value, returned_value);
        assert_eq!(
            snapshot.status,
            if mode == 1 {
                PayoutStatus::Ambiguous
            } else {
                PayoutStatus::Queued
            }
        );
        if mode == 1 {
            assert!(
                admin
                    .resolve_receipt_deposit(31, 0, 0, row.operation_id, row.child)
                    .send_recv(source)
                    .await
                    .is_err(),
                "unpaused resolution must fail"
            );
        }
        let destination = upgrade_destination(&remoting, source).await;
        configure_cutover_destination(&remoting, source, destination).await;
        admin
            .begin_cutover(destination, 32)
            .send_recv(source)
            .await
            .unwrap();
        admin.pause().send_recv(source).await.unwrap();
        admin
            .prepare_cutover(source, 32)
            .send_recv(destination)
            .await
            .unwrap();
        assert!(admin.upgrade(destination).send_recv(source).await.is_err());
        assert_eq!(
            escrow.blocking_redemptions().recv(wrapper).await.unwrap(),
            1
        );
        if mode == 1 {
            assert!(
                VftManagerC::new(remoting.clone().with_actor_id(recipient))
                    .resolve_receipt_deposit(31, 0, 0, row.operation_id, row.child)
                    .send_recv(source)
                    .await
                    .is_err(),
                "only manager admin may resolve"
            );
            for (slot, log, operation, child) in [
                (30, 0, row.operation_id, row.child),
                (31, 1, row.operation_id, row.child),
                (31, 0, H256([99; 32]), row.child),
                (31, 0, row.operation_id, Some(MessageId::from([99; 32]))),
                (31, 0, row.operation_id, None),
            ] {
                assert_eq!(
                    admin
                        .resolve_receipt_deposit(slot, 0, log, operation, child)
                        .send_recv(source)
                        .await
                        .unwrap(),
                    Err(Error::InvalidReconciliation)
                );
            }
            assert_eq!(
                admin
                    .resolve_receipt_deposit(31, 0, 0, row.operation_id, row.child)
                    .send_recv(source)
                    .await
                    .unwrap(),
                Err(Error::InvalidReconciliation),
                "an outstanding payout without an authenticated hold is not resolvable"
            );
        }
        wrapper_admin.pause().send_recv(wrapper).await.unwrap();
        assert!(
            vft_vara_client::NativeEscrow::new(remoting.clone().with_actor_id(recipient))
                .hold_redemption(row.operation_id, snapshot.clone(), true)
                .send_recv(wrapper)
                .await
                .is_err()
        );
        let mut changed = snapshot.clone();
        changed.amount += U256::one();
        assert!(escrow
            .hold_redemption(row.operation_id, changed, true)
            .send_recv(wrapper)
            .await
            .is_err());
        println!(
            "held_native operation=0x{} redemption=0x{}",
            hex::encode(row.operation_id),
            hex::encode(snapshot.encode())
        );
        for consent in [false, true] {
            use vft_vara_client::native_escrow::io::HoldRedemption;
            let payload = HoldRedemption::encode_call(row.operation_id, snapshot.clone(), consent);
            let mut arguments = payload.strip_prefix(HoldRedemption::ROUTE).unwrap();
            let decoded =
                <(H256, vft_vara_client::Redemption, bool)>::decode(&mut arguments).unwrap();
            assert!(arguments.is_empty());
            assert_eq!(decoded, (row.operation_id, snapshot.clone(), consent));
            println!(
                "hold_native consent={consent} payload=0x{}",
                hex::encode(payload)
            );
        }
        if mode != 1 {
            assert!(escrow
                .hold_redemption(row.operation_id, snapshot.clone(), false)
                .send_recv(wrapper)
                .await
                .is_err());
            assert_eq!(
                escrow.blocking_redemptions().recv(wrapper).await.unwrap(),
                1
            );
            assert_eq!(
                escrow
                    .held_redemption(row.operation_id)
                    .recv(wrapper)
                    .await
                    .unwrap(),
                None
            );
        }
        assert_eq!(
            escrow
                .hold_redemption(row.operation_id, snapshot.clone(), mode != 1)
                .send_recv(wrapper)
                .await
                .unwrap(),
            snapshot
        );
        assert_eq!(
            escrow
                .hold_redemption(row.operation_id, snapshot.clone(), false)
                .send_recv(wrapper)
                .await
                .unwrap(),
            snapshot
        );
        let wrapper_balance = remoting.system().balance_of(wrapper);
        assert_eq!(
            escrow
                .outstanding_redemptions()
                .recv(wrapper)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            escrow.blocking_redemptions().recv(wrapper).await.unwrap(),
            0
        );
        assert!(
            admin.upgrade(destination).send_recv(source).await.is_err(),
            "zero wrapper blockers does not release the receipt reservation"
        );
        for _ in 0..2 {
            let (resolved, events) = resolve_receipt_events(&remoting, source, 31, 0, &row);
            assert_eq!(resolved, Ok(ReceiptDepositOutcome::Held));
            assert!(events.is_empty(), "holding is not settlement/refund");
        }
        let held = receipt_deposits(&admin, source, 31, 0).await.remove(0);
        assert_eq!(held.outcome, ReceiptDepositOutcome::Held);
        assert_eq!(
            (held.operation_id, held.child),
            (row.operation_id, row.child)
        );
        assert_eq!(
            receipt_status(&admin, source, 31, 0).await,
            ReceiptStatus::Resolved
        );
        assert_eq!(
            admin
                .receipt_rejection(31, 0, 0)
                .recv(source)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            balance_of(&remoting, wrapper, recipient).await,
            U256::zero()
        );
        assert_eq!(
            VftC::new(remoting.clone())
                .total_supply()
                .recv(wrapper)
                .await
                .unwrap(),
            amount
        );
        assert_eq!(remoting.system().balance_of(wrapper), wrapper_balance);
        wrapper_admin.resume().send_recv(wrapper).await.unwrap();
        admin.upgrade(destination).send_recv(source).await.unwrap();
        assert_eq!(balance_of(&remoting, wrapper, destination).await, amount);
        if mode == 1 {
            assert!(admin
                .finalize_cutover()
                .send_recv(destination)
                .await
                .is_err());
        }
        wrapper_admin.pause().send_recv(wrapper).await.unwrap();
        escrow
            .configure_manager(destination)
            .send_recv(wrapper)
            .await
            .unwrap();
        if mode == 1 {
            assert!(admin
                .finalize_cutover()
                .send_recv(destination)
                .await
                .is_err());
        }
        wrapper_admin
            .set_burner(destination)
            .send_recv(wrapper)
            .await
            .unwrap();
        wrapper_admin.resume().send_recv(wrapper).await.unwrap();
        admin
            .finalize_cutover()
            .send_recv(destination)
            .await
            .unwrap();
        assert_eq!(
            admin.cutover_status().recv(source).await.unwrap().phase,
            CutoverPhase::Retired
        );
        assert_eq!(
            admin
                .cutover_status()
                .recv(destination)
                .await
                .unwrap()
                .phase,
            CutoverPhase::Ready
        );
        assert_eq!(
            escrow.manager().recv(wrapper).await.unwrap(),
            Some(destination)
        );
        if mode == 0 {
            let recipient_before = remoting.system().balance_of(recipient);
            mailbox.claim_value(payout_log.clone()).unwrap();
            remoting.system().run_next_block();
            assert_eq!(
                escrow
                    .redemption(row.operation_id)
                    .recv(wrapper)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                PayoutStatus::Delivered
            );
            assert_eq!(
                remoting.system().balance_of(recipient),
                recipient_before + raw
            );
            assert_eq!(
                escrow
                    .outstanding_redemptions()
                    .recv(wrapper)
                    .await
                    .unwrap(),
                0
            );
        } else if mode == 2 {
            mailbox
                .reply_bytes(payout_log.clone(), Vec::new(), raw)
                .unwrap();
            remoting.system().run_next_block();
            assert_eq!(
                escrow
                    .redemption(row.operation_id)
                    .recv(wrapper)
                    .await
                    .unwrap()
                    .unwrap()
                    .returned_value,
                raw
            );
            assert_eq!(
                escrow
                    .outstanding_redemptions()
                    .recv(wrapper)
                    .await
                    .unwrap(),
                1
            );
            assert_eq!(remoting.system().balance_of(wrapper), wrapper_balance + raw);
        }
        assert!(mailbox.reply_bytes(payout_log, Vec::new(), 0).is_err());
        assert_eq!(
            escrow.blocking_redemptions().recv(wrapper).await.unwrap(),
            0
        );
        assert_eq!(
            escrow
                .held_redemption(row.operation_id)
                .recv(wrapper)
                .await
                .unwrap(),
            Some(snapshot)
        );
        assert_eq!(receipt_deposits(&admin, source, 31, 0).await, vec![held]);
        assert_eq!(
            balance_of(&remoting, wrapper, recipient).await,
            U256::zero()
        );
        assert_eq!(
            VftC::new(remoting.clone())
                .total_supply()
                .recv(wrapper)
                .await
                .unwrap(),
            amount
        );
        assert!(admin
            .transactions(Order::Direct, 0, 10)
            .recv(source)
            .await
            .unwrap()
            .is_empty());
        let (replay, events) = submit_receipt_events(&remoting, source, 31, 0, receipt);
        assert!(replay.is_err());
        assert!(events.is_empty());
    }
}

#[tokio::test]
async fn test_awaiting_vft_map_update_cannot_commit_after_begin_cutover() {
    let Fixture {
        remoting,
        vft_manager_program_id: source,
        gear_supply_vft,
        ..
    } = setup_for_test().await;
    let destination = upgrade_destination(&remoting, source).await;
    let admin = VftManagerC::new(remoting.clone());
    let before = admin.vara_to_eth_addresses().recv(source).await.unwrap();
    let manual = remoting.clone().with_block_run_mode(BlockRunMode::Manual);
    let mut updating_client = VftManagerC::new(manual.clone());
    let mut beginning_client = VftManagerC::new(manual);
    // Queue the freeze before the real VFT can reply to the awaited balance query.
    let updating = updating_client
        .update_vfts(vec![(gear_supply_vft, 100_121.into())])
        .send(source)
        .await
        .unwrap();
    let beginning = beginning_client
        .begin_cutover(destination, 32)
        .send(source)
        .await
        .unwrap();
    for _ in 0..3 {
        remoting.run_next_block();
    }
    beginning
        .recv()
        .now_or_never()
        .expect("queued cutover must finish")
        .unwrap();
    assert!(updating
        .recv()
        .now_or_never()
        .expect("frozen map update must finish without a commit")
        .is_err());
    assert_eq!(
        admin.vara_to_eth_addresses().recv(source).await.unwrap(),
        before
    );
    let intent = admin.cutover_status().recv(source).await.unwrap();
    assert_eq!(intent.phase, vft_manager_client::CutoverPhase::Draining);
    assert_eq!(intent.successor, Some(destination));
}

#[tokio::test]
async fn test_mock_receipt_admission_preserves_typed_cutover_domain_errors() {
    let Fixture {
        remoting,
        vft_manager_program_id: source,
        ..
    } = setup_for_test().await;
    let destination = upgrade_destination(&remoting, source).await;
    configure_cutover_destination(&remoting, source, destination).await;
    let mut admin = VftManagerC::new(remoting.clone());
    admin
        .begin_cutover(destination, 32)
        .send_recv(source)
        .await
        .unwrap();
    assert_eq!(
        admin
            .calculate_gas_for_reply(33, 0, TokenSupply::Gear)
            .send_recv(source)
            .await
            .unwrap(),
        Err(Error::ReceiptOutsideDeployment)
    );
    admin
        .prepare_cutover(source, 32)
        .send_recv(destination)
        .await
        .unwrap();
    assert_eq!(
        admin
            .calculate_gas_for_reply(32, 0, TokenSupply::Gear)
            .send_recv(destination)
            .await
            .unwrap(),
        Err(Error::ReceiptOutsideDeployment)
    );
    assert_eq!(
        admin
            .calculate_gas_for_reply(33, 0, TokenSupply::Gear)
            .send_recv(destination)
            .await
            .unwrap(),
        Err(Error::CutoverNotReady)
    );
    admin.pause().send_recv(source).await.unwrap();
    admin.upgrade(destination).send_recv(source).await.unwrap();
    assert_eq!(
        admin
            .calculate_gas_for_reply(32, 0, TokenSupply::Gear)
            .send_recv(source)
            .await
            .unwrap(),
        Err(Error::Retired)
    );
    assert!(receipt_deposits(&admin, source, 32, 0).await.is_empty());
    assert!(receipt_deposits(&admin, destination, 33, 0)
        .await
        .is_empty());
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
