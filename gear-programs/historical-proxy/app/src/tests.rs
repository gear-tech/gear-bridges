use gclient::{Event, EventProcessor, GearApi, GearEvent};
use sails_rs::{calls::*, gclient::calls::*, prelude::*};
use historical_proxy_client::traits::HistoricalProxyService;

async fn spin_up_node() -> (GClientRemoting, GearApi, CodeId, GasUnit) {
    let api = GearApi::dev().await.unwrap();
    let gas_limit = api.block_gas_limit().unwrap();
    let remoting = GClientRemoting::new(api.clone());
    let (code_id, _) = api.upload_code(historical_proxy::WASM_BINARY).await.unwrap();

    (remoting, api, code_id, gas_limit)
}
