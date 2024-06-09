use super::*;
use gstd::{msg, prelude::*, ActorId};

static mut ADMIN_ADDRESS: ActorId = ActorId::new([0u8; 32]);
static mut GRC20_GATEWAY_ADDRESS: ActorId = ActorId::new([0u8; 32]);
static mut FEE: u128 = 0;

#[no_mangle]
extern "C" fn init() {
    let init_msg: InitMessage = msg::load().expect("Failed to load request");
    let admin = msg::source();

    unsafe {
        ADMIN_ADDRESS = admin;
        GRC20_GATEWAY_ADDRESS = init_msg.grc20_gateway;
        FEE = init_msg.fee;
    }
}

#[gstd::async_main]
async fn main() {
    if msg::source() == unsafe { ADMIN_ADDRESS } {
        let admin_msg: AdminMessage = msg::load().expect("Failed to load admin message");

        match admin_msg {
            AdminMessage::SetFee(fee) => unsafe { FEE = fee },
        }
    } else {
        let payload = msg::load_bytes().expect("Failed to load payload");
        msg::send_bytes_for_reply(unsafe { GRC20_GATEWAY_ADDRESS }, payload, 0, 0)
            .expect("Failed to send bytes")
            .await
            .expect("Error requesting bridging");

        if msg::value() < unsafe { FEE } {
            panic!("Insufficient fee paid");
        }
    }
}

#[no_mangle]
extern "C" fn state() {
    msg::reply(State { fee: unsafe { FEE } }, 0).expect("Failed to read state");
}