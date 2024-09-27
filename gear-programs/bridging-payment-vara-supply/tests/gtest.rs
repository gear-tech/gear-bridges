use bridging_payment_vara_supply_client::{
    traits::*, BridgingPayment as BridgingPaymentC,
    BridgingPaymentVaraSupplyFactory as BridgingPaymentVaraSupplyFactoryC, Config, InitConfig,
};
use gtest::{Log, Program, WasmProgram};
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use vft_treasury_app::services::vft::{traits::*, Vft as VftC, VftFactory as VftFactoryC};
use vft_treasury_client::{
    traits::*, Config as VftTreasuryConfig, InitConfig as VftTreasuryInitConfig,
    VftTreasury as VftTreasuryC, VftTreasuryFactory as VftTreasuryFactoryC,
};

const ADMIN_ID: u64 = 1000;
const FEE: u128 = 10_000_000_000_000;
const ETH_CLIENT_ID: u64 = 500;
const BRIDGE_BUILTIN_ID: u64 = 300;

#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
enum Response {
    MessageSent { nonce: U256, hash: H256 },
}

#[derive(Debug)]
struct GearBridgeBuiltinMock;

impl WasmProgram for GearBridgeBuiltinMock {
    fn init(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(None)
    }

    fn handle(&mut self, _payload: Vec<u8>) -> Result<Option<Vec<u8>>, &'static str> {
        Ok(Some(
            Response::MessageSent {
                nonce: U256::from(1),
                hash: [1; 32].into(),
            }
            .encode(),
        ))
    }

    fn handle_reply(&mut self, _payload: Vec<u8>) -> Result<(), &'static str> {
        unimplemented!()
    }

    fn handle_signal(&mut self, _payload: Vec<u8>) -> Result<(), &'static str> {
        unimplemented!()
    }

    fn state(&mut self) -> Result<Vec<u8>, &'static str> {
        unimplemented!()
    }
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

struct Fixture {
    remoting: GTestRemoting,
    bridging_payment_program_id: ActorId,
    treasury_program_id: ActorId,
    vft_program_id: ActorId,
}

async fn setup_for_test() -> Fixture {
    let remoting = GTestRemoting::new(ADMIN_ID.into());
    remoting.system().init_logger();

    // Bridge Builtin
    let gear_bridge_builtin =
        Program::mock_with_id(remoting.system(), BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    let _ = gear_bridge_builtin.send_bytes(ADMIN_ID, b"INIT");

    // Treasury
    let treasury_code_id = remoting.system().submit_code(vft_treasury::WASM_BINARY);
    let init_config = VftTreasuryInitConfig {
        receiver_contract_address: [1; 20].into(),
        gear_bridge_builtin: BRIDGE_BUILTIN_ID.into(),
        ethereum_event_client: ETH_CLIENT_ID.into(),
        config: VftTreasuryConfig {
            gas_for_transfer_tokens: 15_000_000_000,
            gas_for_reply_deposit: 15_000_000_000,
            gas_to_send_request_to_builtin: 15_000_000_000,
            reply_timeout: 100,
            gas_for_transfer_to_eth_msg: 15_000_000_000,
        },
    };
    let treasury_program_id = VftTreasuryFactoryC::new(remoting.clone())
        .new(init_config)
        .send_recv(treasury_code_id, b"salt")
        .await
        .unwrap();

    // VFT
    let vft_code_id = remoting
        .system()
        .submit_code(extended_vft_wasm::WASM_BINARY_OPT);
    let vft_program_id = VftFactoryC::new(remoting.clone())
        .new("Token".into(), "Token".into(), 18)
        .send_recv(vft_code_id, b"salt")
        .await
        .unwrap();

    // Bridging payment with Vara supply
    let bridging_payment_code_id = remoting
        .system()
        .submit_code(bridging_payment_vara_supply::WASM_BINARY);
    let init_config = InitConfig {
        admin_address: ADMIN_ID.into(),
        vft_treasury_address: treasury_program_id,
        config: Config {
            fee: FEE,
            gas_for_reply_deposit: 15_000_000_000,
            gas_to_send_request_to_treasury: 100_000_000_000,
            reply_timeout: 1000,
            gas_for_request_to_treasury_msg: 50_000_000_000,
        },
    };
    let bridging_payment_program_id = BridgingPaymentVaraSupplyFactoryC::new(remoting.clone())
        .new(init_config)
        .send_recv(bridging_payment_code_id, b"salt")
        .await
        .unwrap();

    Fixture {
        remoting,
        bridging_payment_program_id,
        treasury_program_id,
        vft_program_id,
    }
}

#[tokio::test]
async fn deposit_to_treasury() {
    let Fixture {
        remoting,
        bridging_payment_program_id,
        treasury_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let account_id: ActorId = 10000.into();
    let amount = U256::from(10_000_000_000_u64);
    let eth_token_id: H160 = [2; 20].into();

    let ok = VftC::new(remoting.clone())
        .mint(account_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(treasury_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    VftTreasuryC::new(remoting.clone())
        .map_vara_to_eth_address(eth_token_id, vft_program_id)
        .send_recv(treasury_program_id)
        .await
        .unwrap()
        .unwrap();

    remoting.system().mint_to(account_id, FEE);
    BridgingPaymentC::new(remoting.clone().with_actor_id(account_id))
        .request(amount, [1; 20].into(), vft_program_id)
        .with_value(FEE)
        .send_recv(bridging_payment_program_id)
        .await
        .unwrap();

    assert_eq!(
        balance_of(&remoting, vft_program_id, account_id).await,
        U256::zero()
    );
    assert_eq!(
        balance_of(&remoting, vft_program_id, treasury_program_id).await,
        amount
    );

    // Claim fee
    BridgingPaymentC::new(remoting.clone())
        .reclaim_fee()
        .send_recv(bridging_payment_program_id)
        .await
        .unwrap();

    remoting
        .system()
        .get_mailbox(ADMIN_ID)
        .claim_value(Log::builder().dest(ADMIN_ID))
        .unwrap();
    assert_eq!(remoting.system().balance_of(ADMIN_ID), FEE);
}
