use super::{Error, HandleMessage, InitMessage, Reply, State};
use gstd::{collections::BTreeMap, exec, msg, prelude::*, ActorId};

static mut ADMIN_ADDRESS: ActorId = ActorId::new([0u8; 32]);
static mut STATE: Option<State> = None;

#[no_mangle]
extern "C" fn init() {
    let admin = msg::source();
    unsafe {
        ADMIN_ADDRESS = admin;
    }

    let msg: InitMessage = msg::load().unwrap();

    let mut proof_blocks = BTreeMap::new();
    let block = exec::block_height();
    proof_blocks.insert(msg.genesis_proof.authority_set_id, block);

    unsafe {
        STATE = Some(State {
            latest_proof: msg.genesis_proof,
            proof_blocks,
        });
    }

    reply_ok();
}

#[no_mangle]
extern "C" fn handle() {
    if msg::source() != unsafe { ADMIN_ADDRESS } {
        panic!("Access forbidden");
    }

    let state = unsafe { STATE.as_mut().unwrap() };
    let msg: HandleMessage = msg::load().unwrap();

    if msg.proof.authority_set_id != state.latest_proof.authority_set_id + 1 {
        reply_err(Error::AuthoritySetIdNotSequential);
        return;
    }

    let block = exec::block_height();

    if matches!(state.proof_blocks.last_key_value(), Some((_, &lst_block)) if lst_block == block) {
        reply_err(Error::ManyProofsSubmittedInSameBlock);
        return;
    }

    if state
        .proof_blocks
        .insert(msg.proof.authority_set_id, block)
        .is_some()
    {
        unreachable!("Due to the check that new authority set id == previous + 1");
    }

    state.latest_proof = msg.proof;

    reply_ok();
}

#[no_mangle]
extern "C" fn state() {
    let state = unsafe { STATE.take().expect("State is not set") };
    msg::reply(state, 0).expect("Failed to read state");
}

fn reply_err(err: Error) {
    msg::reply(Reply(Err(err)), 0).expect("Failed to send reply");
}

fn reply_ok() {
    msg::reply(Reply(Ok(())), 0).expect("Failed to send reply");
}
