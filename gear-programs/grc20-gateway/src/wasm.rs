use super::*;
use gstd::{msg, prelude::*};
use primitive_types::U256;

#[no_mangle]
extern "C" fn init() {}

#[no_mangle]
extern "C" fn handle() {
    msg::reply(
        vara2eth::Response {
            nonce: U256::zero(),
        },
        0,
    );
}
