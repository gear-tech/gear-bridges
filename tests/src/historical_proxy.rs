use crate::connect_to_node;

#[tokio::test]
async fn update_admin() {
    let conn = connect_to_node(
        &[500_000_000_000_000],
        "historical_proxy",
        &[historical_proxy::WASM_BINARY],
    )
    .await;
}
