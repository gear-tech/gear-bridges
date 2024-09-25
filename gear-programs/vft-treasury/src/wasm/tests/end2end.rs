use gclient::{GearApi, Result};
mod utils_gclient;
use sails_rs::{ActorId, H160, U256};
use utils_gclient::*;

#[tokio::test]
#[ignore]
async fn test_treasury() -> Result<()> {
    // It will not work on local node as vft-treasury logic relies on pallet-gear-eth-bridge
    // which will be initialized only in ~12 hrs from start on local node.
    let mut client = GearApi::vara_testnet().await?;

    let actor: ActorId = client.get_actor_id();

    // Subscribe to events
    let mut listener = client.subscribe().await?;

    // Check that blocks are still running
    assert!(listener.blocks_running().await?);

    let vft = Vft::new(&client, &mut listener).await?;

    let amount = U256::from(10_000_000_000_u64);

    let result = vft
        .mint(&client, &mut listener, actor, 10_000_000_000u64.into())
        .await?;
    assert!(result, "failed to mint to {}", actor);
    let balance = vft.balance_of(&client, &mut listener, actor).await?;
    assert_eq!(balance, amount);
    let treasury = VftTreasury::new(&client, &mut listener).await?;

    let success = vft
        .approve(&client, &mut listener, treasury.program_id(), amount)
        .await?;
    assert!(
        success,
        "failed to approve {:?} spending {} tokens from {:?}",
        treasury.program_id(),
        amount,
        actor
    );
    let allowance = vft
        .allowance(&client, &mut listener, actor, treasury.program_id())
        .await?;
    assert_eq!(allowance, amount);

    treasury
        .map_vara_to_eth_address(&client, &mut listener, [3; 20].into(), vft.program_id())
        .await?
        .expect("failed to map address");

    let reply = treasury
        .deposit_tokens(
            &client,
            &mut listener,
            100_000_000_000,
            vft.program_id(),
            actor,
            amount,
            [3; 20].into(),
        )
        .await
        .unwrap_or_else(|e| {
            eprintln!("error: {:?}", e);
            client.print_node_logs();
            panic!()
        });

    let expected = H160::from([3; 20]);
    assert_eq!(reply.expect("failed to deposit").1, expected);
    treasury
        .update_ethereum_event_client_address(&client, &mut listener, actor)
        .await?
        .expect("failed to update ETH event client address");

    treasury
        .withdraw_tokens(
            &client,
            &mut listener,
            100_000_000_000,
            [3; 20].into(),
            actor,
            amount,
        )
        .await?
        .expect("failed to withdraw tokens");

    let balance = vft.balance_of(&client, &mut listener, actor).await?;
    assert_eq!(balance, amount);

    Ok(())
}
