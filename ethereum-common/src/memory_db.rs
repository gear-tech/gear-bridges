use super::{keccak_hasher::KeccakHasher, *};
use ::memory_db::{HashKey, NoopTracker};

pub type MemoryDB = ::memory_db::MemoryDB<KeccakHasher, HashKey<KeccakHasher>, Vec<u8>, NoopTracker<Vec<u8>>>;

pub fn new() -> MemoryDB {
    memory_db::MemoryDB::from_null_node(&rlp::NULL_RLP, rlp::NULL_RLP.as_ref().into())
}
