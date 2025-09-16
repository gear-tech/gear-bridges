use crate::{eth_to_vara::eth_to_vara, vara_to_eth::vara_to_eth};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::PollingEthApi;
use gear_rpc_client::GearApi;
use primitive_types::U256;
use std::env;

mod eth_to_vara;
mod vara_to_eth;

pub fn get_var(name: &str) -> String {
    env::var(name)
        .map_err(|_| format!("{name} env variable should be set"))
        .unwrap()
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv().ok();
    let gear_rpc_url = get_var("VARA_WS_RPC");
    let beacon_rpc_url = get_var("BEACON_RPC_URL");
    let eth_rpc_url = get_var("ETH_RPC_URL");

    let api = gsdk::Api::new(gear_rpc_url.as_str())
        .await
        .expect("Failed to create Gear API");

    let mut args = std::env::args().skip(1);
    let scope = args.next().expect("Scope should be provided");

    match scope.as_str() {
        "vara-to-eth" => {
            let gear_api = GearApi::from(api);

            let nonce: U256 = get_var("VARA_TO_ETH_NONCE").parse().expect("Invalid nonce");
            println!("{nonce:?}");

            let block_number: u32 = get_var("VARA_TO_ETH_BLOCK_NUMBER")
                .parse()
                .expect("Block number should be a valid integer");

            vara_to_eth(gear_api, nonce, block_number).await;
        }
        "eth-to-vara" => {
            let gear_api = gclient::GearApi::from(api);
            let polling_eth_api = PollingEthApi::new(&eth_rpc_url)
                .await
                .expect("Failed to create Ethereum API");
            let beacon_client = BeaconClient::new(beacon_rpc_url, None)
                .await
                .expect("Failed to create beacon client");
            let historical_proxy_id_str = get_var("HISTORICAL_PROXY_ID")
                .strip_prefix("0x")
                .unwrap()
                .to_string();
            let tx_hash_str = args
                .next()
                .expect("Missing transaction hash in the args")
                .strip_prefix("0x")
                .unwrap()
                .to_string();

            eth_to_vara(
                tx_hash_str,
                historical_proxy_id_str,
                &beacon_client,
                &gear_api,
                &polling_eth_api,
            )
            .await;
        }
        _ => panic!("Unknown scope"),
    };
}
