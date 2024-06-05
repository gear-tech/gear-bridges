use std::{
    collections::{btree_map::Entry, BTreeMap, HashMap},
    sync::mpsc::{channel, Receiver},
    thread,
    time::Duration,
};

use crate::{EthereumArgs, RelayArgs};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::{dto::Message, GearApi};

const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);
const GEAR_BLOCK_TIME_APPROX: Duration = Duration::from_secs(3);

type AuthoritySetId = u64;

struct MessagesInBlock {
    messages: Vec<Message>,
    block: u32,
}

struct RelayedMerkleRoot {
    gear_block: u32,
}

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

    let from_gear_block = if let Some(block) = args.from_block {
        block
    } else {
        let block = gear_api.latest_finalized_block().await?;
        gear_api.block_hash_to_number(block).await?
    };

    let from_eth_block = eth_api.block_number().await?;

    log::info!(
        "Starting gear event processing from block #{}",
        from_gear_block
    );
    let messages = run_event_processor(gear_api.clone(), from_gear_block);

    log::info!("Starting ethereum listener from block #{}", from_eth_block);
    let merkle_roots = run_merkle_root_listener(eth_api.clone(), gear_api, from_eth_block);

    log::info!("Starting message relayer");
    run_message_relayer(eth_api, messages, merkle_roots);

    Ok(())
}

fn run_event_processor(
    gear_api: GearApi,
    from_block: u32,
) -> Receiver<(AuthoritySetId, MessagesInBlock)> {
    let (sender, receiver) = channel();

    tokio::spawn(async move {
        let mut current_block = from_block;

        loop {
            let finalized_head = gear_api.latest_finalized_block().await.unwrap();
            let finalized_head = gear_api.block_hash_to_number(finalized_head).await.unwrap();

            if finalized_head >= current_block {
                for block in current_block..=finalized_head {
                    let block_hash = gear_api.block_number_to_hash(current_block).await.unwrap();
                    let messages = gear_api.message_queued_events(block_hash).await.unwrap();

                    if !messages.is_empty() {
                        log::info!("Found {} messages", messages.len());
                    } else {
                        continue;
                    }

                    let authority_set_id = gear_api
                        .signed_by_authority_set_id(block_hash)
                        .await
                        .unwrap();

                    sender
                        .send((authority_set_id, MessagesInBlock { messages, block }))
                        .unwrap();
                }

                current_block = finalized_head + 1;
            } else {
                thread::sleep(GEAR_BLOCK_TIME_APPROX);
            }
        }
    });

    receiver
}

fn run_merkle_root_listener(
    eth_api: EthApi,
    gear_api: GearApi,
    from_block: u64,
) -> Receiver<(AuthoritySetId, RelayedMerkleRoot)> {
    let (sender, receiver) = channel();

    tokio::spawn(async move {
        let mut current_block = from_block;

        loop {
            let latest = eth_api.block_number().await.unwrap();
            if latest >= current_block {
                let merkle_roots = eth_api
                    .fetch_merkle_roots_in_range(current_block, latest)
                    .await
                    .unwrap();

                for merkle_root in merkle_roots {
                    let block_hash = gear_api
                        .block_number_to_hash(merkle_root.block_number as u32)
                        .await
                        .unwrap();

                    let authority_set_id = gear_api
                        .signed_by_authority_set_id(block_hash)
                        .await
                        .unwrap();

                    sender
                        .send((
                            authority_set_id,
                            RelayedMerkleRoot {
                                gear_block: merkle_root.block_number as u32,
                            },
                        ))
                        .unwrap();
                }

                current_block = latest + 1;
            } else {
                thread::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2)
            }
        }
    });

    receiver
}

struct Era {
    merkle_roots: Vec<RelayedMerkleRoot>,
    messages: Vec<MessagesInBlock>,
}

fn run_message_relayer(
    eth_api: EthApi,
    messages: Receiver<(AuthoritySetId, MessagesInBlock)>,
    merkle_roots: Receiver<(AuthoritySetId, RelayedMerkleRoot)>,
) {
    let mut eras: BTreeMap<AuthoritySetId, Era> = BTreeMap::new();

    loop {
        for (authority_set_id, messages) in messages.try_iter() {
            match eras.entry(authority_set_id) {
                Entry::Occupied(mut entry) => {
                    entry.get_mut().messages.push(messages);
                }
                Entry::Vacant(entry) => {
                    entry.insert(Era {
                        merkle_roots: vec![],
                        messages: vec![messages],
                    });
                }
            }
        }

        for (authority_set_id, merkle_root) in merkle_roots.try_iter() {
            match eras.entry(authority_set_id) {
                Entry::Occupied(mut entry) => {
                    entry.get_mut().merkle_roots.push(merkle_root);
                }
                Entry::Vacant(entry) => {
                    entry.insert(Era {
                        merkle_roots: vec![merkle_root],
                        messages: vec![],
                    });
                }
            }
        }

        //for message in messages {
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
        //}
    }
}
