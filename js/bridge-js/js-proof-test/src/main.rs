use ethereum_beacon_client::BeaconClient;
use ethereum_client::PollingEthApi;
use gclient::GearApi;
use relayer::message_relayer::eth_to_gear::proof_composer::compose;

#[tokio::main]
async fn main() {
    let gear_api = GearApi::builder()
        .build(gclient::WSAddress::new("wss://testnet.vara.network", None))
        .await
        .expect("GearApi client should be created");

    let beacon_client = BeaconClient::new("http://unstable.hoodi.beacon-api.nimbus.team/", None)
        .await
        .expect("Failed to create beacon client");

    let eth_api =
        PollingEthApi::new("https://eth-hoodi.g.alchemy.com/v2/g4iWV409zGS5az4LouyDVRd6S2WWwCLk")
            .await
            .expect("Failed to create Ethereum API");

    let tx_hash = "0xddbcd9191bfa11e040afe87d476381066f2aefa7287f846da4eb7c35f5a0a704";
    let historical_proxy_id = "0x5d2a0dcfc30301ad5eda002481e6d0b283f81a1221bef8ba2a3fa65fd56c8e0f";

    let compose_result = compose(
        &beacon_client,
        &gear_api,
        &eth_api,
        tx_hash,
        &historical_proxy_id,
    )
    .await;

    println!("Compose result: {:?}", compose_result);
}
