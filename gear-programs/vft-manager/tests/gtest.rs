use alloy_consensus::{Receipt, ReceiptEnvelope, ReceiptWithBloom};
use gtest::{Program, System, WasmProgram};
use sails_rs::{calls::*, gtest::calls::*, prelude::*};
use vft_client::{traits::*, Vft as VftC, VftFactory as VftFactoryC};
use vft_manager_app::services::abi::ERC20_MANAGER;
use vft_manager_client::{
    traits::*, Config, Error, InitConfig, TokenSupply, VftManager as VftManagerC,
    VftManagerFactory as VftManagerFactoryC,
};

const ADMIN_ID: u64 = 1_000;
const ETH_CLIENT_ID: u64 = 500;
const BRIDGE_BUILTIN_ID: u64 = 300;
const ERC20_MANAGER_ADDRESS: H160 = H160([1; 20]);

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

struct Fixture {
    remoting: GTestRemoting,
    vft_manager_program_id: ActorId,
    vft_program_id: ActorId,
}

async fn setup_for_test() -> Fixture {
    let system = System::new();
    system.init_logger();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);
    system.mint_to(ETH_CLIENT_ID, 100_000_000_000_000);

    let remoting = GTestRemoting::new(system, ADMIN_ID.into());

    // Bridge Builtin
    let gear_bridge_builtin =
        Program::mock_with_id(remoting.system(), BRIDGE_BUILTIN_ID, GearBridgeBuiltinMock);
    let _ = gear_bridge_builtin.send_bytes(ADMIN_ID, b"INIT");

    // Vft Manager
    let vft_manager_code_id = remoting.system().submit_code(vft_manager::WASM_BINARY);
    let init_config = InitConfig {
        erc20_manager_address: ERC20_MANAGER_ADDRESS,
        gear_bridge_builtin: BRIDGE_BUILTIN_ID.into(),
        eth_client: ETH_CLIENT_ID.into(),
        config: Config {
            gas_to_burn_tokens: 15_000_000_000,
            gas_for_reply_deposit: 15_000_000_000,
            gas_to_mint_tokens: 15_000_000_000,
            gas_to_transfer_tokens: 15_000_000_000,
            gas_to_process_mint_request: 15_000_000_000,
            gas_to_send_request_to_builtin: 15_000_000_000,
            reply_timeout: 100,
            gas_for_transfer_to_eth_msg: 20_000_000_000,
            gas_for_event_sending: 15_000_000_000,
        },
    };
    let vft_manager_program_id = VftManagerFactoryC::new(remoting.clone())
        .new(init_config)
        .send_recv(vft_manager_code_id, b"salt")
        .await
        .unwrap();

    // VFT
    let vft_code_id = remoting
        .system()
        .submit_code(extended_vft_wasm::WASM_BINARY);
    let vft_program_id = VftFactoryC::new(remoting.clone())
        .new("Token".into(), "Token".into(), 18)
        .send_recv(vft_code_id, b"salt")
        .await
        .unwrap();

    Fixture {
        remoting,
        vft_manager_program_id,
        vft_program_id,
    }
}

#[tokio::test]
async fn test_vft_manager() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let mut vft_manager = VftManagerC::new(remoting.clone());
    vft_manager
        .map_vara_to_eth_address(vft_program_id, [2; 20].into(), TokenSupply::Gear)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let account_id: ActorId = 100_000.into();
    remoting.system().mint_to(account_id, 100_000_000_000_000);

    let amount = U256::from(10_000_000_000_u64);

    let mut vft = VftC::new(remoting.clone());

    let ok = vft
        .mint(account_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(vft_manager_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let reply = vft_manager
        .request_bridging(account_id, vft_program_id, amount, [3; 20].into())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let expected = Ok((U256::from(1), H160::from([2; 20])));
    assert_eq!(reply, expected);

    let account_balance = balance_of(&remoting, vft_program_id, account_id).await;
    assert!(account_balance.is_zero());

    let vft_manager_balance = balance_of(&remoting, vft_program_id, vft_manager_program_id).await;
    assert_eq!(vft_manager_balance, amount);

    let receipt_rlp = create_receipt_rlp(account_id, [2; 20].into(), amount);
    VftManagerC::new(remoting.clone().with_actor_id(ETH_CLIENT_ID.into()))
        .submit_receipt(receipt_rlp)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap()
        .unwrap();

    let account_balance = balance_of(&remoting, vft_program_id, account_id).await;
    assert_eq!(account_balance, amount);

    let vft_manager_balance = balance_of(&remoting, vft_program_id, vft_manager_program_id).await;
    assert!(vft_manager_balance.is_zero());
}

#[tokio::test]
async fn test_mapping_does_not_exists() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let account_id: ActorId = 100_000.into();
    remoting.system().mint_to(account_id, 100_000_000_000_000);

    let amount = U256::from(10_000_000_000_u64);

    let ok = VftC::new(remoting.clone())
        .mint(account_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let ok = VftC::new(remoting.clone().with_actor_id(account_id))
        .approve(vft_manager_program_id, amount)
        .send_recv(vft_program_id)
        .await
        .unwrap();
    assert!(ok);

    let reply = VftManagerC::new(remoting.clone())
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    assert!(reply.is_err());
    assert_eq!(reply.unwrap_err(), Error::NoCorrespondingEthAddress);
}

#[tokio::test]
async fn test_withdraw_fails_with_bad_origin() {
    let Fixture {
        remoting,
        vft_manager_program_id,
        vft_program_id,
    } = setup_for_test().await;

    let mut vft_manager = VftManagerC::new(remoting.clone());
    vft_manager
        .map_vara_to_eth_address(vft_program_id, [2; 20].into(), TokenSupply::Gear)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    let account_id: ActorId = 100000.into();

    let receipt_rlp = create_receipt_rlp(account_id, [2; 20].into(), U256::from(42));
    let result = vft_manager
        .submit_receipt(receipt_rlp)
        .send_recv(vft_manager_program_id)
        .await
        .unwrap();

    assert_eq!(result.unwrap_err(), Error::NotEthClient);
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

fn create_receipt_rlp(receiver: ActorId, token: H160, amount: U256) -> Vec<u8> {
    let event = ERC20_MANAGER::BridgingRequested {
        from: [3u8; 20].into(),
        to: receiver.into_bytes().into(),
        token: token.0.into(),
        amount: {
            let mut bytes = [0u8; 32];
            amount.to_little_endian(&mut bytes[..]);

            alloy_primitives::U256::from_le_bytes(bytes)
        },
    };

    let receipt = ReceiptWithBloom::from(Receipt {
        status: true.into(),
        cumulative_gas_used: 100_000u128,
        logs: vec![alloy_primitives::Log {
            address: ERC20_MANAGER_ADDRESS.0.into(),
            data: Into::into(&event),
        }],
    });

    let receipt = ReceiptEnvelope::Eip2930(receipt);

    let mut receipt_rlp = vec![];
    alloy_rlp::Encodable::encode(&receipt, &mut receipt_rlp);

    receipt_rlp
}
