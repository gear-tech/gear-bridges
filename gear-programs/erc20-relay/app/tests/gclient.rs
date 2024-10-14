// Incorporate code generated based on the IDL file
#[allow(dead_code)]
mod vft {
    include!(concat!(env!("OUT_DIR"), "/vft-gateway.rs"));
}

use ethereum_common::{utils as eth_utils, memory_db, trie_db::{TrieMut, Trie, Recorder}, patricia_trie::{TrieDB, TrieDBMut}};
use erc20_relay_client::{EthToVaraEvent, BlockInclusionProof, traits::{Erc20Relay, Erc20RelayFactory}};
use gclient::{Event, EventProcessor, GearApi, GearEvent};
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use vft::vft_gateway;
use alloy_rlp::Encodable;
use alloy::rpc::types::{Log, TransactionReceipt};
use alloy_primitives::Log as PrimitiveLog;
use alloy_consensus::{TxType, Receipt, ReceiptWithBloom, ReceiptEnvelope};
use serde::Deserialize;

const HOLESKY_RECEIPTS_2_498_456: &[u8; 160_144] =
    include_bytes!("./holesky-receipts-2_498_456.json");
const HOLESKY_BLOCK_2_498_456: &[u8; 235_397] = include_bytes!("./holesky-block-2_498_456.json");
const HOLESKY_HEADER_2_498_457: &[u8; 670] = include_bytes!("./holesky-header-2_498_457.json");
const HOLESKY_HEADER_2_498_458: &[u8; 669] = include_bytes!("./holesky-header-2_498_458.json");
const HOLESKY_HEADER_2_498_459: &[u8; 670] = include_bytes!("./holesky-header-2_498_459.json");
const HOLESKY_HEADER_2_498_460: &[u8; 670] = include_bytes!("./holesky-header-2_498_460.json");
const HOLESKY_HEADER_2_498_461: &[u8; 670] = include_bytes!("./holesky-header-2_498_461.json");
const HOLESKY_HEADER_2_498_462: &[u8; 669] = include_bytes!("./holesky-header-2_498_462.json");
const HOLESKY_HEADER_2_498_463: &[u8; 670] = include_bytes!("./holesky-header-2_498_463.json");
const HOLESKY_HEADER_2_498_464: &[u8; 669] = include_bytes!("./holesky-header-2_498_464.json");

#[derive(Deserialize)]
pub struct Receipts {
    result: Vec<TransactionReceipt>,
}

fn map_receipt_envelope(receipt: &ReceiptEnvelope<Log>) -> ReceiptEnvelope<PrimitiveLog> {
    let logs = receipt
        .logs()
        .iter()
        .map(AsRef::as_ref)
        .cloned()
        .collect();

    let result = ReceiptWithBloom::new(
        Receipt {
            status: receipt.status().into(),
            cumulative_gas_used: receipt.cumulative_gas_used(),
            logs,
        },
        *receipt.logs_bloom(),
    );

    match receipt.tx_type() {
        TxType::Legacy => ReceiptEnvelope::Legacy(result),
        TxType::Eip1559 => ReceiptEnvelope::Eip1559(result),
        TxType::Eip2930 => ReceiptEnvelope::Eip2930(result),
        TxType::Eip4844 => ReceiptEnvelope::Eip4844(result),
    }
}

async fn spin_up_node() -> (GClientRemoting, CodeId, GasUnit) {
    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();
    let remoting = GClientRemoting::new(api);
    let (code_id, _) = remoting
        .api()
        .upload_code(erc20_relay::WASM_BINARY)
        .await
        .unwrap();

    (remoting, code_id, gas_limit)
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn gas_for_reply() {
    use erc20_relay_client::{traits::Erc20Relay as _, Erc20Relay, Erc20RelayFactory};

    let route = <vft_gateway::io::MintTokens as ActionIo>::ROUTE;

    let (remoting, code_id, gas_limit) = spin_up_node().await;
    let account_id: ActorId = <[u8; 32]>::from(remoting.api().account_id().clone()).into();

    let factory = Erc20RelayFactory::new(remoting.clone());

    let program_id = factory
        .gas_calculation(1_000, 5_500_000_000)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .unwrap();

    let mut client = Erc20Relay::new(remoting.clone());
    while client
        .fill_transactions()
        .send_recv(program_id)
        .await
        .unwrap()
    {}

    println!("prepared");

    for i in 5..10 {
        let mut listener = remoting.api().subscribe().await.unwrap();

        client
            .calculate_gas_for_reply(i, i)
            .with_gas_limit(10_000_000_000)
            .send(program_id)
            .await
            .unwrap();

        let message_id = listener
            .proc(|e| match e {
                Event::Gear(GearEvent::UserMessageSent { message, .. })
                    if message.destination == account_id.into() && message.details.is_none() =>
                {
                    message.payload.0.starts_with(route).then_some(message.id)
                }
                _ => None,
            })
            .await
            .unwrap();

        println!("message_id = {}", hex::encode(message_id.0.as_ref()));

        let reply: <vft_gateway::io::MintTokens as ActionIo>::Reply = Ok(());
        let payload = {
            let mut result = Vec::with_capacity(route.len() + reply.encoded_size());
            result.extend_from_slice(route);
            reply.encode_to(&mut result);

            result
        };
        let gas_info = remoting
            .api()
            .calculate_reply_gas(None, message_id.into(), payload, 0, true)
            .await
            .unwrap();

        println!("gas_info = {gas_info:?}");
    }
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn set_vft_gateway() {
    use erc20_relay_client::Config;

    let (remoting, code_id, gas_limit) = spin_up_node().await;

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());

    let program_id = factory
        .new(
            Default::default(),
            Default::default(),
            Config {
                reply_timeout: 10_000,
                reply_deposit: 1_000_000_000,
            },
        )
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .unwrap();

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());

    // by default address of the VFT gateway is not set
    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, Default::default());

    let vft_gateway_new = ActorId::from([1u8; 32]);

    // admin should be able to set the VFT gateway address
    client
        .set_vft_gateway(vft_gateway_new)
        .send_recv(program_id)
        .await
        .unwrap();

    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, vft_gateway_new);

    // and reset it
    client
        .set_vft_gateway(Default::default())
        .send_recv(program_id)
        .await
        .unwrap();

    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, Default::default());

    // another account isn't permitted to change the VFT gateway address
    let api = GearApi::dev().await.unwrap().with("//Bob").unwrap();
    let remoting = GClientRemoting::new(api);

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());
    let result = client
        .set_vft_gateway(Default::default())
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    let result = client
        .set_vft_gateway(vft_gateway_new)
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    // anyone should be able to read the address
    let vft_gateway = client.vft_gateway().recv(program_id).await.unwrap();
    assert_eq!(vft_gateway, Default::default());
}

#[tokio::test]
#[ignore = "Requires running node"]
async fn update_config() {
    use erc20_relay_client::Config;

    let (remoting, code_id, gas_limit) = spin_up_node().await;

    let factory = erc20_relay_client::Erc20RelayFactory::new(remoting.clone());

    let checkpoints = ActorId::from([1u8; 32]);
    let address = H160::from([2u8; 20]);
    let reply_timeout = 10_000;
    let reply_deposit = 1_000_000_000;
    let program_id = factory
        .new(
            checkpoints,
            address,
            Config {
                reply_timeout,
                reply_deposit,
            },
        )
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .unwrap();

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());

    assert_eq!(
        Config {
            reply_timeout,
            reply_deposit,
        },
        client.config().recv(program_id).await.unwrap()
    );

    let reply_timeout = 4_000;
    let reply_deposit = 1_222_000_000;
    client
        .update_config(Config {
            reply_timeout,
            reply_deposit,
        })
        .send_recv(program_id)
        .await
        .unwrap();

    assert_eq!(
        Config {
            reply_timeout,
            reply_deposit,
        },
        client.config().recv(program_id).await.unwrap()
    );

    // another account isn't permitted to update the config
    let api = GearApi::dev().await.unwrap().with("//Bob").unwrap();
    let remoting = GClientRemoting::new(api);

    let mut client = erc20_relay_client::Erc20Relay::new(remoting.clone());
    let result = client
        .update_config(Config {
            reply_timeout: 111,
            reply_deposit: 222,
        })
        .send_recv(program_id)
        .await;
    assert!(result.is_err());

    // anyone is able to get the config
    assert_eq!(
        Config {
            reply_timeout,
            reply_deposit,
        },
        client.config().recv(program_id).await.unwrap()
    );
}

// block = 2_498_456 (number = 2_301_317),
// checkpoint = 2_498_464 (0xb89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1)
// tx 0x180cd2328df9c4356adc77e19e33c5aa2d5395f1b52e70d22c25070a04f16691

#[tokio::test]
#[ignore = "Requires running node"]
async fn test_relay_erc20() {
    let receipts: Receipts = serde_json::from_slice(HOLESKY_RECEIPTS_2_498_456.as_ref()).unwrap();
    let receipts = receipts.result.iter().map(|tx_receipt| {
        let receipt = tx_receipt.as_ref();

        tx_receipt
            .transaction_index
            .map(|i| (i, map_receipt_envelope(receipt)))
    }).collect::<Option<Vec<_>>>()
    .unwrap_or_default();

    let mut memory_db = memory_db::new();
    let key_value_tuples = eth_utils::rlp_encode_receipts_and_nibble_tuples(&receipts[..]);
    let root = {
        let mut root = H256::zero();
        let mut triedbmut = TrieDBMut::new(&mut memory_db, &mut root);
        for (key, value) in &key_value_tuples {
            triedbmut.insert(key, value).unwrap();
        }

        *triedbmut.root()
    };

    let tx_index = 15;
    let (tx_index, receipt) = receipts
        .iter()
        .find(|(index, _)| index == &tx_index)
        .unwrap();

    let trie = TrieDB::new(&memory_db, &root).unwrap();
    let (key, _expected_value) = eth_utils::rlp_encode_index_and_receipt(tx_index, receipt);

    let mut recorder = Recorder::new();
    let _value = trie.get_with(&key, &mut recorder);

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(receipt));
    Encodable::encode(receipt, &mut receipt_rlp);
    // let message = EthToVaraEvent {
    //     proof_block,
    //     proof: recorder
    //         .drain()
    //         .into_iter()
    //         .map(|r| r.data)
    //         .collect::<Vec<_>>(),
    //     transaction_index: *tx_index,
    //     receipt_rlp,
    // };
}
