use ethereum_types::H256;
use hash256_std_hasher::Hash256StdHasher;
use hash_db::Hasher;
use tiny_keccak::{Hasher as _, Keccak};

/// Concrete `Hasher` implementation for the Keccak-256 hash
#[derive(Default, Debug, Clone, PartialEq)]
pub struct KeccakHasher;

impl Hasher for KeccakHasher {
    type Out = H256;
    type StdHasher = Hash256StdHasher;
    const LENGTH: usize = 32;

    fn hash(x: &[u8]) -> Self::Out {
        let mut out = [0; 32];

        let mut hasher = Keccak::v256();
        hasher.update(x);
        hasher.finalize(&mut out);

        out.into()
    }
}
