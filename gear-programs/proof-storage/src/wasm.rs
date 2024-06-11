use super::AuthoritySetId;
use gstd::{msg, prelude::*, BlockNumber, collections::BTreeMap};

static mut ADMIN_ADDRESS: ActorId = ActorId::new([0u8; 32]);
static mut PROOF_BLOCKS: BTreeMap<AuthoritySetId, BlockNumber> = BTreeMap::new();

#[no_mangle]
extern "C" fn init() {
    let admin = msg::source();
    unsafe {
        ADMIN_ADDRESS = admin;
    }

    // proof + circuit data + genesis set id
}

#[no_mangle]
extern "C" fn handle() {
    // new proof + authority set id of this proof
}

#[no_mangle]
extern "C" fn state() {
    // circuit data + latest authority set id + mapping(authority_set_id -> block_number)
}
