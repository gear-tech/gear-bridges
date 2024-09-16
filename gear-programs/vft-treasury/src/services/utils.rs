use super::vft::vft::io as vft_io;
use sails_rs::prelude::*;

pub async fn transfer_tokens(token_id: ActorId, sender: ActorId, receiver: ActorId, amount: U256) {
    let bytes: Vec<u8> = vft_io::TransferFrom::encode_call(sender, receiver, amount);

    gstd::msg::send_bytes(token_id, bytes, 0).expect("failed to transfer tokens");
}
