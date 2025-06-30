use alloy::primitives::fixed_bytes;

use ethereum_client::EthApi;

use relayer::hex_utils;
use relayer::message_relayer::common::EthereumSlotNumber;
use relayer::message_relayer::common::TxHashWithSlot;
use relayer::message_relayer::common::{
    ethereum::message_paid_event_extractor::MessagePaidEventExtractor, EthereumBlockNumber,
};
use relayer::message_relayer::eth_to_gear::storage::BlockStorage;
use relayer::message_relayer::eth_to_gear::storage::Storage;
use relayer::message_relayer::eth_to_gear::tx_manager::TransactionManager;
use std::sync::Arc;

const BLOCKS: [EthereumBlockNumber; 4] = [
    EthereumBlockNumber(4045807),
    EthereumBlockNumber(4044981),
    EthereumBlockNumber(4044973),
    EthereumBlockNumber(4044967),
];

const TRANSACTIONS: [TxHashWithSlot; 4] = [
    TxHashWithSlot {
        tx_hash: fixed_bytes!("0x5bcfe532e9fb06edae8627c97ba681949de8e95cda1c7315c4cec29655002b3b"),
        slot_number: EthereumSlotNumber(4560525),
    },
    TxHashWithSlot {
        tx_hash: fixed_bytes!("0x09a972afe1684367592d7d82eac3193723b2b8d27aba88b62edc5e47fb420f99"),
        slot_number: EthereumSlotNumber(4559462),
    },
    TxHashWithSlot {
        tx_hash: fixed_bytes!("0x0d07b7911052b00dbb2096e86c79b15cee24a15cb756deff898dbd522c643df3"),
        slot_number: EthereumSlotNumber(4559451),
    },
    TxHashWithSlot {
        tx_hash: fixed_bytes!("0x376f557154bd1272cba0ac2220cf894f6d1209e0945d26720467eb162778aa56"),
        slot_number: EthereumSlotNumber(4559445),
    },
];

const BRIDING_PAYMENT_ADDRESS: &str = "0x94f7dc06314Efc22a8Cc16dC78DA9Ba5A20D1544";
const GENESIS_TIME: u64 = 1695877200; // 2023-09-27T00:00:00Z - Holesky genesis time

struct MockStorage(BlockStorage);

#[async_trait::async_trait]
impl Storage for MockStorage {
    fn block_storage(&self) -> &BlockStorage {
        &self.0
    }

    async fn save_blocks(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn save(&self, _: &TransactionManager) -> anyhow::Result<()> {
        Ok(())
    }

    async fn load(&self, _: &TransactionManager) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "Requires a valid Ethereum RPC endpoint for Holesky testnet"]
async fn test_eth_event_extractor() {
    let rpc_url = std::env::var("ETH_RPC").expect("ETH_RPC environment variable not set");

    let bridging_payment_address = hex_utils::decode_h160(&BRIDING_PAYMENT_ADDRESS);

    let rpc = EthApi::new(
        &rpc_url,
        "0x0000000000000000000000000000000000000000",
        "0x0000000000000000000000000000000000000000",
        None,
    )
    .await
    .expect("Failed to create EthApi instance");

    let storage = Arc::new(MockStorage(BlockStorage::new()));

    let extractor = MessagePaidEventExtractor::new(
        rpc.clone(),
        bridging_payment_address.unwrap(),
        storage.clone(),
        GENESIS_TIME,
    );

    let (blocks_tx, blocks_rx) = tokio::sync::mpsc::unbounded_channel();
    for block in BLOCKS {
        blocks_tx.send(block).expect("Failed to send block");
    }

    let mut receiver = extractor.run(blocks_rx).await;

    for i in 0..TRANSACTIONS.len() {
        let tx = receiver
            .recv()
            .await
            .expect("Failed to receive transaction from extractor");

        let expected_tx = TRANSACTIONS
            .iter()
            .find(|&t| t.slot_number == tx.slot_number && t.tx_hash == tx.tx_hash);
        assert!(
            expected_tx.is_some(),
            "Received transaction not found in expected transactions"
        );

        let block_storage = storage.block_storage();
        let blocks = block_storage.blocks_raw();
        let blocks = blocks.read().await;

        let block = blocks.get(&tx.slot_number);
        assert!(
            block.is_some(),
            "Block for transaction not found in storage"
        );
        let block = block.unwrap();
        assert!(
            block.transactions.contains(&tx.tx_hash),
            "Transaction hash not found in block"
        );
    }

    drop(blocks_tx);
}
