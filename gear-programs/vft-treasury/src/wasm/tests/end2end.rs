use gclient::{GearApi, Result};
mod utils_gclient;
use sails_rs::U256;
use utils_gclient::*;

#[tokio::test]
#[ignore]
async fn test_treasury() -> Result<()> {
    let client = GearApi::dev().await?;

    // Subscribe to events
    let mut listener = client.subscribe().await?;

    // Check that blocks are still running
    assert!(listener.blocks_running().await?);

    let vft = Vft::new(&client, &mut listener).await?;
    let actor = client.get_specific_actor_id(USERS_STR[0]);
    let amount = U256::from(10_000_000_000_u64);

    vft.mint(&client, &mut listener, actor, 10_000_000_000u64.into())
        .await?;

    let balance = vft.balance_of(&client, &mut listener, actor).await?;
    println!("balance = {}", balance);

    let treasury = VftTreasury::new(&client, &mut listener).await?;

    vft.approve(&client, &mut listener, actor, treasury.program_id(), amount)
        .await?;
    println!("approved");

    let reply = treasury
        .deposit_tokens(
            &client,
            &mut listener,
            vft.program_id(),
            actor,
            amount,
            [3; 20].into(),
        )
        .await?;

    println!("{:?}", reply);

    Ok(())
}
