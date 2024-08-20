use gear_rpc_client::GearApi;

pub async fn fetch(gear_api: GearApi, block: Option<u32>) -> anyhow::Result<()> {
    let block = match block {
        Some(bn) => Some(
            gear_api
                .block_number_to_hash(bn)
                .await
                .expect("Failed to fetch block hash by number"),
        ),
        None => None,
    };

    let state = gear_api.authority_set_state(block).await.unwrap();

    println!("Authority set id: {}", state.authority_set_id);
    println!(
        "Authority set hash: {}",
        hex::encode(&state.authority_set_hash)
    );

    Ok(())
}
