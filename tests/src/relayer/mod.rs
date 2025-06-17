use super::historical_proxy::shared;
use alloy_primitives::{fixed_bytes, FixedBytes};
use eth_events_electra_client::*;
use gclient::metadata::runtime_types::frame_system::extensions::check_tx_version;
use gstd::{Decode, Encode};
use relayer::message_relayer::{
    common::{EthereumSlotNumber, TxHashWithSlot},
    eth_to_gear::paid_token_transfers::{
        message_sender::{MessageSenderIo, Response, SendStatus},
        proof_composer::{ComposeProof, ComposedProof, ProofComposerIo},
        storage::*,
        tx_manager::{Transaction, TransactionManager, TxStatus},
    },
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

const TX_HASH: FixedBytes<32> =
    fixed_bytes!("0x180cd2328df9c4356adc77e19e33c5aa2d5395f1b52e70d22c25070a04f16691");
const SLOT_NUMBER: u64 = 2_498_456;

const BLOCK_4513181: &str = include_str!("./block_4513181.json");

struct MockProofComposer;
impl MockProofComposer {
    async fn run() -> ProofComposerIo {
        let (mut proof_tx, mut proof_rx) = unbounded_channel();
        let (mut proof_response_tx, mut proof_response_rx) = unbounded_channel();

        let proof_composer_io = ProofComposerIo::new(proof_tx, proof_response_rx);

        tokio::task::spawn(async move {
            let req = proof_rx.recv().await.unwrap();

            assert_eq!(req.tx.tx_hash, TX_HASH);

            proof_response_tx.send(ComposedProof {
                payload: event(),
                tx_uuid: req.tx_uuid,
            });
        });

        proof_composer_io
    }
}

struct MockMessageSender;

impl MockMessageSender {
    async fn run() -> MessageSenderIo {
        let (message_tx, mut message_rx) = unbounded_channel();
        let (mut message_response_tx, message_response_rx) = unbounded_channel();

        let message_sender_io = MessageSenderIo::new(message_tx, message_response_rx);

        tokio::task::spawn(async move {
            let req = message_rx.recv().await.unwrap();

            assert_eq!(req.payload, event());
            assert_eq!(req.tx_hash, TX_HASH);

            message_response_tx.send(Response {
                tx_uuid: req.tx_uuid,
                status: SendStatus::Success,
            })
        });

        message_sender_io
    }
}

#[tokio::test]
pub async fn test_relayer() {
    /*let message = event();
    let tx_ = TxHashWithSlot {
        slot_number: EthereumSlotNumber(SLOT_NUMBER),
        tx_hash: TX_HASH,
    };
    let (mut deposit_events_tx, deposit_events_rx) = unbounded_channel();

    deposit_events_tx.send(tx_);

    let mut tx_manager = TransactionManager::new(false, false, None);

    let message_sender = MockMessageSender::run().await;
    let proof_composer = MockProofComposer::run().await;

    let _ = tx_manager
        .run(deposit_events_rx, proof_composer, message_sender)
        .await;

    drop(deposit_events_tx);*/
}
