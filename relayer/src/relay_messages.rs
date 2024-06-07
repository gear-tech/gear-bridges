use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::Duration,
};

use ethereum_client::Contracts as EthApi;
use gear_rpc_client::{dto::Message, GearApi};
use keccak_hash::keccak_256;

const ETHEREUM_BLOCK_TIME_APPROX: Duration = Duration::from_secs(12);
const GEAR_BLOCK_TIME_APPROX: Duration = Duration::from_secs(3);

type AuthoritySetId = u64;
type BlockNumber = u32;

struct MessagesInBlock {
    messages: Vec<Message>,
    block: u32,
}

#[derive(Clone, Copy)]
struct RelayedMerkleRoot {
    gear_block: u32,
}

pub async fn run(
    gear_api: GearApi,
    eth_api: EthApi,
    from_block: Option<u32>,
) -> anyhow::Result<()> {
    let from_gear_block = if let Some(block) = from_block {
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
    let merkle_roots = run_merkle_root_listener(eth_api.clone(), gear_api.clone(), from_eth_block);

    log::info!("Starting message relayer");
    run_message_relayer(eth_api, gear_api, messages, merkle_roots).await;

    Ok(())
}

fn run_event_processor(
    gear_api: GearApi,
    from_block: u32,
) -> Receiver<(AuthoritySetId, MessagesInBlock)> {
    let (sender, receiver) = channel();

    tokio::spawn(async move {
        loop {
            let res = event_processor_inner(&gear_api, from_block, &sender).await;
            if let Err(err) = res {
                log::error!("Event processor failed: {}", err);
            }
        }
    });

    receiver
}

async fn event_processor_inner(
    gear_api: &GearApi,
    from_block: u32,
    sender: &Sender<(AuthoritySetId, MessagesInBlock)>,
) -> anyhow::Result<()> {
    let mut current_block = from_block;

    loop {
        let finalized_head = gear_api.latest_finalized_block().await?;
        let finalized_head = gear_api.block_hash_to_number(finalized_head).await?;

        if finalized_head >= current_block {
            for block in current_block..=finalized_head {
                log::info!("Processing gear block #{}", block);

                let block_hash = gear_api.block_number_to_hash(block).await?;
                let messages = gear_api.message_queued_events(block_hash).await?;

                if !messages.is_empty() {
                    log::info!("Found {} messages", messages.len());
                } else {
                    continue;
                }

                let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;

                sender.send((authority_set_id, MessagesInBlock { messages, block }))?;
            }

            current_block = finalized_head + 1;
        } else {
            thread::sleep(GEAR_BLOCK_TIME_APPROX);
        }
    }
}

fn run_merkle_root_listener(
    eth_api: EthApi,
    gear_api: GearApi,
    from_block: u64,
) -> Receiver<(AuthoritySetId, RelayedMerkleRoot)> {
    let (sender, receiver) = channel();

    tokio::spawn(async move {
        loop {
            let res = merkle_root_listener_inner(&eth_api, &gear_api, from_block, &sender).await;
            if let Err(err) = res {
                log::error!("Merkle root listener failed: {}", err);
            }
        }
    });

    receiver
}

async fn merkle_root_listener_inner(
    eth_api: &EthApi,
    gear_api: &GearApi,
    from_block: u64,
    sender: &Sender<(AuthoritySetId, RelayedMerkleRoot)>,
) -> anyhow::Result<()> {
    let mut current_block = from_block;

    loop {
        let latest = eth_api.block_number().await?;
        if latest >= current_block {
            log::info!("Processing ethereum blocks #{}..#{}", current_block, latest);
            let merkle_roots = eth_api
                .fetch_merkle_roots_in_range(current_block, latest)
                .await?;

            if !merkle_roots.is_empty() {
                log::info!("Found {} merkle roots", merkle_roots.len());
            }

            for merkle_root in merkle_roots {
                let block_hash = gear_api
                    .block_number_to_hash(merkle_root.block_number as u32)
                    .await?;

                let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;

                sender.send((
                    authority_set_id,
                    RelayedMerkleRoot {
                        gear_block: merkle_root.block_number as u32,
                    },
                ))?;
            }

            current_block = latest + 1;
        } else {
            thread::sleep(ETHEREUM_BLOCK_TIME_APPROX / 2)
        }
    }
}

struct Era {
    latest_merkle_root: Option<RelayedMerkleRoot>,
    messages: BTreeMap<BlockNumber, Vec<Message>>,
}

async fn run_message_relayer(
    eth_api: EthApi,
    gear_api: GearApi,
    messages: Receiver<(AuthoritySetId, MessagesInBlock)>,
    merkle_roots: Receiver<(AuthoritySetId, RelayedMerkleRoot)>,
) {
    let mut eras: BTreeMap<AuthoritySetId, Era> = BTreeMap::new();

    loop {
        for (authority_set_id, new_messages) in messages.try_iter() {
            match eras.entry(authority_set_id) {
                Entry::Occupied(mut entry) => {
                    // TODO: Check that = None
                    entry
                        .get_mut()
                        .messages
                        .insert(new_messages.block, new_messages.messages);
                }
                Entry::Vacant(entry) => {
                    let mut messages = BTreeMap::new();
                    messages.insert(new_messages.block, new_messages.messages);

                    entry.insert(Era {
                        latest_merkle_root: None,
                        messages,
                    });
                }
            }
        }

        for (authority_set_id, new_merkle_root) in merkle_roots.try_iter() {
            match eras.entry(authority_set_id) {
                Entry::Occupied(mut entry) => {
                    let era = entry.get_mut();

                    if let Some(mr) = era.latest_merkle_root.as_ref() {
                        if mr.gear_block < new_merkle_root.gear_block {
                            era.latest_merkle_root = Some(new_merkle_root);
                        }
                    } else {
                        era.latest_merkle_root = Some(new_merkle_root);
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert(Era {
                        latest_merkle_root: Some(new_merkle_root),
                        messages: BTreeMap::new(),
                    });
                }
            }
        }

        // TODO
        let latest_era = eras.last_key_value().map(|(k, _)| *k);
        match latest_era {
            Some(_) => {
                for (era_id, era) in eras.iter_mut() {
                    let res = process_era(&gear_api, &eth_api, era).await;
                    if let Err(err) = res {
                        log::error!("Failed to process era #{}: {}", era_id, err);
                    }
                }
            }
            None => {}
        }
    }
}

async fn process_era(gear_api: &GearApi, eth_api: &EthApi, era: &mut Era) -> anyhow::Result<()> {
    let Some(latest_merkle_root) = era.latest_merkle_root else {
        return Ok(());
    };

    let mut processed_blocks = vec![];

    for (&block, messages) in era.messages.iter() {
        if block > latest_merkle_root.gear_block {
            break;
        }

        let block_hash = gear_api
            .block_number_to_hash(latest_merkle_root.gear_block)
            .await?;

        for message in messages {
            let message_hash = message_hash(message);

            log::info!("Relaying message with hash {}", hex::encode(&message_hash));

            let proof = gear_api
                .fetch_message_inclusion_merkle_proof(block_hash, message_hash.into())
                .await?;

            // TODO: Fully decode
            let nonce_bytes = &message.nonce_le[..16];
            let nonce = u128::from_le_bytes(nonce_bytes.try_into()?);

            eth_api
                .provide_content_message(
                    latest_merkle_root.gear_block,
                    proof.num_leaves as u32,
                    proof.leaf_index as u32,
                    nonce,
                    message.source,
                    message.destination,
                    &message.payload[..],
                    proof.proof,
                )
                .await?;

            log::info!("Message #{} successfully relayed", nonce);
        }

        processed_blocks.push(block);
    }

    // TODO: Clean only when ethereum block is finalized
    for block in processed_blocks {
        era.messages.remove_entry(&block);
    }

    Ok(())
}

fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_le.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_256(&data, &mut hash);

    hash
}
