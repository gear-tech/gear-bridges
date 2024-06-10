use gstd::{msg, prelude::*};

#[no_mangle]
extern "C" fn init() {
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
