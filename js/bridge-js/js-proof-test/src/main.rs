use ethereum_beacon_client::BeaconClient;
use ethereum_client::PollingEthApi;
use gclient::GearApi;
use relayer::message_relayer::eth_to_gear::proof_composer::compose;
use sails_rs::Encode;
use std::{env, path::PathBuf};

fn get_var(name: &str) -> String {
    env::var(name).expect("{name} env variable should be set")
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv().ok();
    let gear_rpc_url = get_var("VARA_WS_RPC");
    let beacon_rpc_url = get_var("BEACON_RPC_URL");
    let eth_rpc_url = get_var("ETH_RPC_URL");
    let historical_proxy_id_str = get_var("HISTORICAL_PROXY_ID")
        .strip_prefix("0x")
        .unwrap()
        .to_string();
    let tx_hash_str = std::env::args()
        .nth(1)
        .expect("Transaction hash should be provided")
        .strip_prefix("0x")
        .unwrap()
        .to_string();

    let gear_api = GearApi::builder()
        .build(gclient::WSAddress::new(gear_rpc_url, None))
        .await
        .expect("GearApi client should be created");

    let beacon_client = BeaconClient::new(beacon_rpc_url, None)
        .await
        .expect("Failed to create beacon client");

    let eth_api = PollingEthApi::new(&eth_rpc_url)
        .await
        .expect("Failed to create Ethereum API");

    let historical_proxy_id: [u8; 32] = hex::decode(historical_proxy_id_str)
        .unwrap()
        .as_slice()
        .try_into()
        .unwrap();
    let tx_hash: [u8; 32] = hex::decode(tx_hash_str)
        .unwrap()
        .as_slice()
        .try_into()
        .unwrap();

    let compose_result = compose(
        &beacon_client,
        &gear_api,
        &eth_api,
        tx_hash.into(),
        historical_proxy_id.into(),
    )
    .await
    .expect("Failed to compose proof");

    let payload = hex::encode(compose_result.encode());

    let receipt_rlp_hex = hex::encode(&compose_result.receipt_rlp);
    let proof_hex = compose_result
        .proof
        .iter()
        .map(hex::encode)
        .collect::<Vec<String>>()
        .join("");

    println!("proof: {proof_hex}");
    println!("receipt_rlp: {receipt_rlp_hex}");

    let target_dir = PathBuf::from("test/tmp");

    if !target_dir.exists() {
        std::fs::create_dir_all(&target_dir).expect("Failed to create target directory");
    }

    std::fs::write(target_dir.join("receipt_rlp"), receipt_rlp_hex)
        .expect("Failed to write receipt_rlp");
    std::fs::write(target_dir.join("proof"), proof_hex).expect("Failed to write proof");
    std::fs::write(target_dir.join("eth_to_vara_scale"), payload)
        .expect("Failed to write eth_to_vara");
}
