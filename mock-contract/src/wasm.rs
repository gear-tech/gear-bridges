extern crate alloc;

use checkpoint_light_client_client::service_checkpoint_for;
use gstd::{debug, msg, prelude::*};
use hex_literal::hex;
use primitive_types::H256;
use sails_rs::{
    calls::ActionIo,
    prelude::{Decode, Encode},
};

/* associative list of slot -> (checkpoint, blockRoot) */
static CHECKPOINTS: &[(u64, (u64, H256))] = &[
    (
        4_534_630,
        (
            4_534_655,
            H256(hex!(
                "ca88b75653941bd709d239f9cf44aa0327d9532ce265db37f692de1df104a090"
            )),
        ),
    ),
    (
        4_534_633,
        (
            4_534_655,
            H256(hex!(
                "ca88b75653941bd709d239f9cf44aa0327d9532ce265db37f692de1df104a090"
            )),
        ),
    ),
    (
        4_534_682,
        (
            4_534_685,
            H256(hex!(
                "307fe9634ea7e127807faa1c89935c3e170f1b5cc449b37eef7334ca42ffdfd7"
            )),
        ),
    ),
    (
        4_534_773,
        (
            4_534_783,
            H256(hex!(
                "1efc6ddcf4c9757610b896634275254c1edc522c97e36baaee736dbd4f949da8"
            )),
        ),
    ),
    (
        4_535_105,
        (
            4_535_134,
            H256(hex!(
                "20367847517c0fe3b51ba8399546755967838b2f3fe5c459f574771dc8067e62"
            )),
        ),
    ),
    (
        4_535_108,
        (
            4_535_134,
            H256(hex!(
                "20367847517c0fe3b51ba8399546755967838b2f3fe5c459f574771dc8067e62"
            )),
        ),
    ),
    (
        4_536_096,
        (
            4_536_124,
            H256(hex!(
                "2d3ff114a232763e2cd5e886806a0ce0a582d83e3f0c917e1be2dff1eea898ea"
            )),
        ),
    ),
    (
        4_537_829,
        (
            4_537_855,
            H256(hex!(
                "6543f3e6ed9474fed0b030757c0b4d4cb96b87ab9f964ed7fc1c4c82b3938e48"
            )),
        ),
    ),
    (
        4_540_604,
        (
            4_540_607,
            H256(hex!(
                "f9b0c82bf9c16a54f31056e088d51c2bde920c23a200b415cd781fb2182667b1"
            )),
        ),
    ),
];

const GET_CHECKPOINT: &[u8] = <service_checkpoint_for::io::Get as ActionIo>::ROUTE;

#[unsafe(no_mangle)]
extern "C" fn init() {}

#[unsafe(no_mangle)]
extern "C" fn handle() {
    let payload = msg::load_bytes().expect("unable to load payload");

    if !payload.starts_with(GET_CHECKPOINT) {
        panic!("Unknown action: {payload:?}");
    }
    let params = &payload[GET_CHECKPOINT.len()..];
    let slot = <service_checkpoint_for::io::Get as ActionIo>::Params::decode(&mut &params[..])
        .expect("unable to decode params");
    debug!("Received request for checkpoint for slot: {slot}");

    let checkpoint = CHECKPOINTS
        .iter()
        .find(|(s, _)| *s == slot)
        .map(|(_, (checkpoint, block_root))| (*checkpoint, sails_rs::H256::from(block_root.0)))
        .expect("checkpoint not found");

    debug!("Found checkpoint: {checkpoint:?}");

    let mut bytes = Vec::with_capacity(GET_CHECKPOINT.len() + Encode::size_hint(&checkpoint));
    bytes.extend_from_slice(GET_CHECKPOINT);
    let result =
        Result::<(u64, sails_rs::H256), checkpoint_light_client_client::CheckpointError>::Ok((
            checkpoint.0,
            checkpoint.1 .0.into(),
        ));
    <service_checkpoint_for::io::Get as ActionIo>::Reply::encode_to(&result, &mut bytes);

    msg::reply_bytes(bytes, 0).expect("unable to reply with checkpoint");
}
