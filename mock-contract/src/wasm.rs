extern crate alloc;

use gstd::{ActorId, debug, exec, msg, prelude::*};
use primitive_types::H256;
use sails_rs::calls::ActionIo;
use sails_rs::prelude::{Decode, Encode};
#[unsafe(no_mangle)]
extern "C" fn init() {}

/* associative list of slot -> (checkpoint, blockRoot) */
static CHECKPOINTS: &[(u64, (u64, H256))] = &[
    (
        4534630,
        (
            4534655,
            H256([
                202, 136, 183, 86, 83, 148, 27, 215, 9, 210, 57, 249, 207, 68, 170, 3, 39, 217, 83,
                44, 226, 101, 219, 55, 246, 146, 222, 29, 241, 4, 160, 144,
            ]),
        ),
    ),
    (
        4534633,
        (
            4534655,
            H256([
                202, 136, 183, 86, 83, 148, 27, 215, 9, 210, 57, 249, 207, 68, 170, 3, 39, 217, 83,
                44, 226, 101, 219, 55, 246, 146, 222, 29, 241, 4, 160, 144,
            ]),
        ),
    ),
    (
        4534682,
        (
            4534685,
            H256([
                48, 127, 233, 99, 78, 167, 225, 39, 128, 127, 170, 28, 137, 147, 92, 62, 23, 15,
                27, 92, 196, 73, 179, 126, 239, 115, 52, 202, 66, 255, 223, 215,
            ]),
        ),
    ),
    (
        4534773,
        (
            4534783,
            H256([
                30, 252, 109, 220, 244, 201, 117, 118, 16, 184, 150, 99, 66, 117, 37, 76, 30, 220,
                82, 44, 151, 227, 107, 170, 238, 115, 109, 189, 79, 148, 157, 168,
            ]),
        ),
    ),
    (
        4535105,
        (
            4535134,
            H256([
                32, 54, 120, 71, 81, 124, 15, 227, 181, 27, 168, 57, 149, 70, 117, 89, 103, 131,
                139, 47, 63, 229, 196, 89, 245, 116, 119, 29, 200, 6, 126, 98,
            ]),
        ),
    ),
    (
        4535108,
        (
            4535134,
            H256([
                32, 54, 120, 71, 81, 124, 15, 227, 181, 27, 168, 57, 149, 70, 117, 89, 103, 131,
                139, 47, 63, 229, 196, 89, 245, 116, 119, 29, 200, 6, 126, 98,
            ]),
        ),
    ),
    (
        4536096,
        (
            4536124,
            H256([
                45, 63, 241, 20, 162, 50, 118, 62, 44, 213, 232, 134, 128, 106, 12, 224, 165, 130,
                216, 62, 63, 12, 145, 126, 27, 226, 223, 241, 238, 168, 152, 234,
            ]),
        ),
    ),
    (
        4537829,
        (
            4537855,
            H256([
                101, 67, 243, 230, 237, 148, 116, 254, 208, 176, 48, 117, 124, 11, 77, 76, 185,
                107, 135, 171, 159, 150, 78, 215, 252, 28, 76, 130, 179, 147, 142, 72,
            ]),
        ),
    ),
    (
        4540604,
        (
            4540607,
            H256([
                249, 176, 200, 43, 249, 193, 106, 84, 243, 16, 86, 224, 136, 213, 28, 43, 222, 146,
                12, 35, 162, 0, 180, 21, 205, 120, 31, 178, 24, 38, 103, 177,
            ]),
        ),
    ),
];
use checkpoint_light_client_client::service_checkpoint_for;
const GET_CHECKPOINT: &[u8] = <service_checkpoint_for::io::Get as ActionIo>::ROUTE;

#[unsafe(no_mangle)]
extern "C" fn handle() {
    let id = msg::source();
    let payload = msg::load_bytes().expect("unable to load payload");

    if payload.starts_with(GET_CHECKPOINT) {
        let params = &payload[GET_CHECKPOINT.len()..];
        let slot = <service_checkpoint_for::io::Get as ActionIo>::Params::decode(&mut &params[..])
            .expect("unable to decode params");
        debug!("Received request for checkpoint for slot: {}", slot);

        let checkpoint = CHECKPOINTS
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, (checkpoint, block_root))| (*checkpoint, *block_root))
            .expect("checkpoint not found");

        debug!("Found checkpoint: {:?}", checkpoint);

        let mut bytes = Vec::with_capacity(GET_CHECKPOINT.len() + Encode::size_hint(&checkpoint));
        bytes.extend_from_slice(GET_CHECKPOINT);
        let result =
            Result::<(u64, H256), checkpoint_light_client_client::CheckpointError>::Ok(checkpoint);
        <service_checkpoint_for::io::Get as ActionIo>::Reply::encode_to(&result, &mut bytes);

        msg::reply_bytes(bytes, 0).expect("unable to reply with checkpoint");
    } else {
        panic!("Unknown action: {:?}", payload);
    }
}
