use crate::{EthereumArgs, RelayArgs};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::GearApi;

pub async fn relay(args: RelayArgs) -> anyhow::Result<()> {
    let gear_api = GearApi::new(&args.vara_endpoint.vara_endpoint)
        .await
        .unwrap();

    let eth_api = {
        let EthereumArgs {
            eth_endpoint,
            fee_payer,
            relayer_address,
            mq_address,
        } = args.ethereum_args;

        EthApi::new(
            &eth_endpoint,
            &mq_address,
            &relayer_address,
            fee_payer.as_deref(),
        )
        .unwrap_or_else(|err| panic!("Error while creating ethereum client: {}", err))
    };

    let mut current_block = if let Some(block) = args.from_block {
        block
    } else {
        let block = gear_api.latest_finalized_block().await?;
        gear_api.block_hash_to_number(block).await?
    };

    loop {
        log::info!("Processing block #{}", current_block);
        let res = process_block(&gear_api, &eth_api, current_block).await;

        if let Err(res) = res {
            log::error!("{}", res);
            continue;
        }

        current_block += 1;
    }
}

async fn process_block(gear_api: &GearApi, eth_api: &EthApi, block: u32) -> anyhow::Result<()> {
    let block = gear_api.block_number_to_hash(block).await?;
    let messages = gear_api.message_queued_events(block).await?;

    if !messages.is_empty() {
        log::info!("Found {} messages", messages.len());
    }

    // eth_api
    //     .provide_content_message(
    //         block.0,
    //         proof.num_leaves as u32,
    //         proof.leaf_index as u32,
    //         1u128,
    //         sender,
    //         [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1],
    //         &[0x11][..],
    //         proof.proof,
    //     )
    //     .await?;

    Ok(())
}
