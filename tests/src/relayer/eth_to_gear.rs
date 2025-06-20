use alloy::primitives::{fixed_bytes, FixedBytes};
use alloy::rpc::types::TransactionReceipt;
use alloy_rlp::Encodable;
use eth_events_electra_client::{BlockGenericForBlockBody, BlockInclusionProof, EthToVaraEvent};
use ethereum_common::utils::{BeaconBlockHeaderResponse, MerkleProof};
use ethereum_common::{
    beacon::electra::Block,
    beacon::BlockHeader,
    utils::{self as eth_utils, BeaconBlockResponse},
};
use gstd::Encode;
use relayer::message_relayer::common::{EthereumSlotNumber, TxHashWithSlot};
use relayer::message_relayer::eth_to_gear::storage::Storage;
use relayer::message_relayer::eth_to_gear::{
    message_sender::{self, MessageSenderIo},
    proof_composer::{self, ProofComposerIo},
    tx_manager::*,
};
use ruzstd::{self, StreamingDecoder};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::AtomicUsize;
use std::sync::LazyLock;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
/*const BLOCK_4011537: &[u8] = include_bytes!("./blocks/block_4011537.json.zst");
const RECEIPTS_4011537: &[u8] = include_bytes!("./blocks/receipts_4011537.json.zst");
const HEADER_4011538: &[u8] = include_bytes!("./blocks/headers_4011538.json.zst");

fn holesky_block(bytes: &[u8]) -> Block {
    let mut source = bytes;
    let mut decoder = StreamingDecoder::new(&mut source).unwrap();
    let mut result = Vec::new();

    decoder.read_to_end(&mut result).unwrap();

    let block: Block = {
        let response: BeaconBlockResponse<Block> = serde_json::from_slice(&result).unwrap();
        response.data.message
    };

    block
}

fn holesky_header(bytes: &[u8]) -> BlockHeader {
    let mut source = bytes;
    let mut decoder = StreamingDecoder::new(&mut source).unwrap();
    let mut result = Vec::new();

    decoder.read_to_end(&mut result).unwrap();

    serde_json::from_slice::<BeaconBlockHeaderResponse>(&result)
        .unwrap()
        .data
        .header
        .message
}

fn holesky_receipt(bytes: &[u8]) -> Vec<TransactionReceipt> {
    let mut source = bytes;
    let mut decoder = StreamingDecoder::new(&mut source).unwrap();
    let mut result = Vec::new();

    decoder.read_to_end(&mut result).unwrap();
    let receipts: Receipts = serde_json::from_slice(&result).unwrap();
    let receipts: Vec<TransactionReceipt> = receipts.result;
    receipts
}

const TX_HASH: FixedBytes<32> =
    fixed_bytes!("0xe0ea78116b2f8318c50d7eebb120244dbd4225f259170a7e68d4da333f3b7757");
const SLOT_NUMBER: u64 = 4513181;
const TX_INDEX: u64 = 44;

fn event() -> EthToVaraEvent {
    let block = holesky_block(BLOCK_4011537);
    let receipts = holesky_receipt(RECEIPTS_4011537)
        .into_iter()
        .map(|tx_receipt| {
            let receipt = tx_receipt.as_ref();

            tx_receipt
                .transaction_index
                .map(|i| (i, eth_utils::map_receipt_envelope(receipt)))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();

    let headers = vec![holesky_header(HEADER_4011538)];

    let MerkleProof { proof, receipt } =
        eth_utils::generate_merkle_proof(TX_INDEX, &receipts[..]).unwrap();

    println!("Proof: {proof:?}\nReceipt: {receipt:?}");

    let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
    Encodable::encode(&receipt, &mut receipt_rlp);
    let block = BlockGenericForBlockBody {
        slot: block.slot,
        proposer_index: block.proposer_index,
        parent_root: block.parent_root,
        state_root: block.state_root,
        body: block.body.into(),
    };

    EthToVaraEvent {
        proof_block: BlockInclusionProof { block, headers },
        proof: proof.clone(),
        transaction_index: TX_INDEX,
        receipt_rlp,
    }
}

struct MockProofComposer;

impl MockProofComposer {
    async fn run(
        mut requests: UnboundedReceiver<proof_composer::Request>,
        response: UnboundedSender<proof_composer::Response>,
    ) {
        let req = requests.recv().await.expect("expected TX");
        assert_eq!(req.tx.tx_hash, TX_HASH);
        assert_eq!(req.tx.slot_number.0, SLOT_NUMBER);

        let event = event();

        response
            .send(proof_composer::Response {
                payload: event,
                tx_uuid: req.tx_uuid,
            })
            .expect("response channel dropped");
    }
}

struct MockMessageSender;

impl MockMessageSender {
    async fn run(
        mut requests: UnboundedReceiver<message_sender::Request>,
        responses: UnboundedSender<message_sender::Response>,
    ) {
        let req = requests.recv().await.expect("expected message send req");

        assert_eq!(req.payload, event());

        responses
            .send(message_sender::Response {
                tx_uuid: req.tx_uuid,
                status: message_sender::MessageStatus::Success,
            })
            .unwrap();
    }
}

struct MockStorage {
    transition_count: AtomicUsize,
}

impl MockStorage {
    fn new() -> Self {
        Self {
            transition_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl Storage for MockStorage {
    async fn load(&self, _tx_manager: &TransactionManager) -> anyhow::Result<()> {
        /* no-op for tests */
        Ok(())
    }

    async fn save(&self, tx_manager: &TransactionManager) -> anyhow::Result<()> {
        match self
            .transition_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        {
            0 => {
                let txns = tx_manager.transactions.read().await;
                assert!(txns.len() == 1);
                let tx = txns.iter().next().unwrap().1;
                assert!(matches!(tx.status, TxStatus::ComposeProof));
            }

            1 => {
                let txns = tx_manager.transactions.read().await;
                assert!(txns.len() == 1);
                let tx = txns.iter().next().unwrap().1;

                let TxStatus::SubmitMessage { ref payload } = tx.status else {
                    panic!("Tx status should transition to SubmitMessage from ComposeProof");
                };

                assert_eq!(&event().encode(), payload);
            }

            2 => {
                let txns = tx_manager.transactions.read().await;
                assert!(txns.len() == 1);
                let tx = txns.iter().next().unwrap().1;
                assert!(matches!(tx.status, TxStatus::Completed));
            }

            _ => unreachable!(),
        }

        Ok(())
    }
}

#[tokio::test]
async fn test_relayer() {
    let (proof_req_tx, proof_req_rx) = unbounded_channel();
    let (proof_res_tx, proof_res_rx) = unbounded_channel();

    let proof_composer_io = ProofComposerIo::new(proof_req_tx, proof_res_rx);

    let (message_req_tx, message_req_rx) = unbounded_channel();
    let (message_res_tx, message_res_rx) = unbounded_channel();

    let message_sender_io = MessageSenderIo::new(message_req_tx, message_res_rx);

    tokio::spawn(MockProofComposer::run(proof_req_rx, proof_res_tx));
    tokio::spawn(MockMessageSender::run(message_req_rx, message_res_tx));

    let storage = MockStorage::new();

    let tx_manager = TransactionManager::new(Some(Box::new(storage)));

    let (deposit_event_tx, deposit_event_rx) = unbounded_channel();

    deposit_event_tx
        .send(TxHashWithSlot {
            slot_number: EthereumSlotNumber(SLOT_NUMBER),
            tx_hash: TX_HASH,
        })
        .unwrap();

    assert!(tx_manager
        .run(deposit_event_rx, proof_composer_io, message_sender_io)
        .await
        .is_ok());
    drop(deposit_event_tx);
}
*/

#[derive(Deserialize, Debug)]
struct Receipts {
    result: Vec<TransactionReceipt>,
}

#[derive(Deserialize, Debug)]
pub struct Tx {
    pub tx_hash: FixedBytes<32>,
    pub tx_index: u64,
    pub block_number: u64,
    pub slot_number: u64,
    pub epoch_number: u64,
    pub checkpoint: u64,
    pub receipts: Receipts,
    pub block: BeaconBlockResponse<Block>,
    pub headers: Vec<BeaconBlockHeaderResponse>,
}

impl Tx {
    pub fn event(&self) -> EthToVaraEvent {
        let receipts = self.receipts
            .result
            .iter()
            .map(|tx_receipt| {
                tx_receipt
                    .transaction_index
                    .map(|i| (i, eth_utils::map_receipt_envelope(tx_receipt)))
            }).collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        let headers = self.headers.clone();

        let MerkleProof {
            proof, receipt
        } = eth_utils::generate_merkle_proof(self.tx_index,
            &receipts
        ).unwrap();
        i
        println!("Proof for tx #{}:\n{:#?}", self.tx_index, proof);

        let mut receipt_rlp = Vec::with_capacity(Encodable::length(&receipt));
        Encodable::encode(&receipt, &mut receipt_rlp);

        let block = BlockGenericForBlockBody {
            slot: self.block.data.message.slot,
            proposer_index: self.block.data.message.proposer_index,
            parent_root: self.block.data.message.parent_root,
            state_root: self.block.data.message.state_root,
            body: self.block.data.message.body  
        };

        EthToVaraEvent {
            proof_block: BlockInclusionProof {
                block: self.block.data.message.clone(),
                headers,
                proof: proof.clone(),
                transaction
            }
        }
    }
}

static TRANSACTIONS_BYTES: &[u8] = include_bytes!("./transactions.json.zst");
static TRANSACTIONS: LazyLock<HashMap<FixedBytes<32>, Tx>> = LazyLock::new(|| {
    let mut txs = TRANSACTIONS_BYTES;
    let mut decoder = StreamingDecoder::new(&mut txs).unwrap();
    let mut result = Vec::new();
    decoder.read_to_end(&mut result).unwrap();

    let txs: Vec<Tx> = serde_json::from_slice(&result).unwrap();

    txs.into_iter().map(|tx| (tx.tx_hash, tx)).collect()
});

struct MockProofComposer;

#[tokio::test]
async fn test_relayer() {
    let txs = &*TRANSACTIONS;
}
