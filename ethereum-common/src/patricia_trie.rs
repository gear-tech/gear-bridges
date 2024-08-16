// Inspired by the Parity Ethereum project.

pub use crate::{keccak_hasher::KeccakHasher, rlp_node_codec::RlpNodeCodec};

/// Convenience type alias to instantiate a Keccak-flavoured `RlpNodeCodec`
pub type RlpCodec = RlpNodeCodec<KeccakHasher>;

/// Defines the working of a particular flavour of trie:
/// how keys are hashed, how values are encoded, does it use extension nodes or not.
#[derive(Clone, Default)]
pub struct Layout;

impl trie_db::TrieLayout for Layout {
    const USE_EXTENSION: bool = true;
    type Hash = KeccakHasher;
    type Codec = RlpNodeCodec<KeccakHasher>;
}

/// Convenience type alias to instantiate a Keccak/Rlp-flavoured `TrieDB`
///
/// Use it as a `Trie` trait object. You can use `db()` to get the backing database object.
/// Use `get` and `contains` to query values associated with keys in the trie.
///
/// # Example
/// ```
/// use hash_db::*;
/// use memory_db::*;
/// use trie_db::*;
/// use ethereum_types::H256;
/// use ethereum_common::{patricia_trie::{TrieDB, TrieDBMut}, memory_db};
///
///   let mut memdb = memory_db::new();
///   let mut root = H256::zero();
///   TrieDBMut::new(&mut memdb, &mut root).insert(b"foo", b"bar").unwrap();
///   let t = TrieDB::new(&memdb, &root).unwrap();
///   assert!(t.contains(b"foo").unwrap());
///   assert_eq!(t.get(b"foo").unwrap().unwrap(), b"bar".to_vec());
/// ```
pub type TrieDB<'db> = trie_db::TrieDB<'db, Layout>;

/// Convenience type alias to instantiate a Keccak/Rlp-flavoured `TrieDBMut`
///
/// Use it as a `TrieMut` trait object. You can use `db()` to get the backing database object.
/// Note that changes are not committed to the database until `commit` is called.
/// Querying the root or dropping the trie will commit automatically.

/// # Example
/// ```
/// use hash_db::*;
/// use memory_db::*;
/// use trie_db::*;
/// use ethereum_types::H256;
/// use ethereum_common::{patricia_trie::{TrieDB, TrieDBMut}, rlp_node_codec, memory_db};
///
///   let mut memdb = memory_db::new();
///   let mut root = H256::zero();
///   let mut t = TrieDBMut::new(&mut memdb, &mut root);
///   assert!(t.is_empty());
///   assert_eq!(*t.root(), rlp_node_codec::HASHED_NULL_NODE);
///   t.insert(b"foo", b"bar").unwrap();
///   assert!(t.contains(b"foo").unwrap());
///   assert_eq!(t.get(b"foo").unwrap().unwrap(), b"bar".to_vec());
///   t.remove(b"foo").unwrap();
///   assert!(!t.contains(b"foo").unwrap());
/// ```
pub type TrieDBMut<'db> = trie_db::TrieDBMut<'db, Layout>;

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use memory_db::{HashKey, MemoryDB};
    use trie_db::{Trie, TrieMut};

    #[test]
    fn test_inline_encoding_branch() {
        let mut memdb = MemoryDB::<KeccakHasher, HashKey<_>, Vec<u8>>::from_null_node(
            &rlp::NULL_RLP,
            rlp::NULL_RLP.as_ref().into(),
        );
        let mut root = H256::zero();
        {
            let mut triedbmut = TrieDBMut::new(&mut memdb, &mut root);
            triedbmut.insert(b"foo", b"bar").unwrap();
            triedbmut.insert(b"fog", b"b").unwrap();
            triedbmut.insert(b"fot", &vec![0u8; 33][..]).unwrap();
        }
        let t = TrieDB::new(&memdb, &root).unwrap();
        assert!(t.contains(b"foo").unwrap());
        assert!(t.contains(b"fog").unwrap());
        assert_eq!(t.get(b"foo").unwrap().unwrap(), b"bar".to_vec());
        assert_eq!(t.get(b"fog").unwrap().unwrap(), b"b".to_vec());
        assert_eq!(t.get(b"fot").unwrap().unwrap(), vec![0u8; 33]);
    }

    #[test]
    fn test_inline_encoding_extension() {
        let mut memdb = MemoryDB::<KeccakHasher, HashKey<_>, Vec<u8>>::from_null_node(
            &rlp::NULL_RLP,
            rlp::NULL_RLP.as_ref().into(),
        );
        let mut root = H256::zero();
        {
            let mut triedbmut = TrieDBMut::new(&mut memdb, &mut root);
            triedbmut.insert(b"foo", b"b").unwrap();
            triedbmut.insert(b"fog", b"a").unwrap();
        }
        let t = TrieDB::new(&memdb, &root).unwrap();
        assert!(t.contains(b"foo").unwrap());
        assert!(t.contains(b"fog").unwrap());
        assert_eq!(t.get(b"foo").unwrap().unwrap(), b"b".to_vec());
        assert_eq!(t.get(b"fog").unwrap().unwrap(), b"a".to_vec());
    }
}
