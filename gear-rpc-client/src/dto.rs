use parity_scale_codec::{Decode, Encode};
use serde::{
    de::{self, MapAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Serialize,
};
use std::fmt;

const ED25519_PUBLIC_KEY_SIZE: usize = 32;
const ED25519_SIGNATURE_SIZE: usize = 64;
const KECCAK_HASH_SIZE: usize = 32;
const BLAKE2_HASH_SIZE: usize = 32;

#[derive(Encode, Decode, Clone)]
pub struct PreCommit {
    pub public_key: [u8; ED25519_PUBLIC_KEY_SIZE],
    pub signature: [u8; ED25519_SIGNATURE_SIZE],
}

impl Serialize for PreCommit {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("PreCommit", 2)?;
        state.serialize_field("public_key", &hex::encode(self.public_key))?;
        state.serialize_field("signature", &hex::encode(self.signature))?;

        state.end()
    }
}

impl<'de> Deserialize<'de> for PreCommit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PreCommitVisitor;

        impl<'de> Visitor<'de> for PreCommitVisitor {
            type Value = PreCommit;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a PreCommit struct")
            }

            fn visit_map<V>(self, mut map: V) -> Result<PreCommit, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut public_key = None;
                let mut signature = None;
                let mut id = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        "public_key" => {
                            let hex_str: String = map.next_value()?;
                            let bytes = hex::decode(hex_str).map_err(de::Error::custom)?;
                            if bytes.len() != ED25519_PUBLIC_KEY_SIZE {
                                return Err(de::Error::custom("Invalid public key length"));
                            }
                            let mut arr = [0u8; ED25519_PUBLIC_KEY_SIZE];
                            arr.copy_from_slice(&bytes);
                            public_key = Some(arr);
                        }
                        "signature" => {
                            let hex_str: String = map.next_value()?;
                            let bytes = hex::decode(hex_str).map_err(de::Error::custom)?;
                            if bytes.len() != ED25519_SIGNATURE_SIZE {
                                return Err(de::Error::custom("Invalid signature length"));
                            }
                            let mut arr = [0u8; ED25519_SIGNATURE_SIZE];
                            arr.copy_from_slice(&bytes);
                            signature = Some(arr);
                        }
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                let public_key =
                    public_key.ok_or_else(|| de::Error::missing_field("public_key"))?;
                let signature = signature.ok_or_else(|| de::Error::missing_field("signature"))?;

                Ok(PreCommit {
                    public_key,
                    signature,
                })
            }
        }

        deserializer.deserialize_struct("PreCommit", &["public_key", "signature"], PreCommitVisitor)
    }
}

#[derive(Encode, Decode, Serialize, Deserialize, Clone)]
pub struct BlockFinalityProof {
    pub validator_set: Vec<[u8; ED25519_PUBLIC_KEY_SIZE]>,
    pub pre_commits: Vec<PreCommit>,
    pub message: Vec<u8>,
}

pub struct BranchNodeData {
    pub data: Vec<u8>,
    pub target_child: u8,
}

pub struct StorageInclusionProof {
    pub address: Vec<u8>,

    pub block_header: Vec<u8>,
    /// Arranged from leaf to root.
    pub branch_nodes_data: Vec<BranchNodeData>,
    pub leaf_node_data: Vec<u8>,

    pub stored_data: Vec<u8>,
}

pub struct ValidatorSetChangeProof {
    pub current_epoch_block_finality: BlockFinalityProof,
    pub queued_keys_inclusion_proof: StorageInclusionProof,
}

pub struct MessageSentProof {
    pub block_finality_proof: BlockFinalityProof,
    pub storage_inclusion_proof: StorageInclusionProof,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MerkleProof {
    pub root: [u8; KECCAK_HASH_SIZE],
    pub proof: Vec<[u8; KECCAK_HASH_SIZE]>,
    pub num_leaves: u64,
    pub leaf_index: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub nonce_be: [u8; 32],
    pub source: [u8; 32],
    pub destination: [u8; 20],
    pub payload: Vec<u8>,
}

pub struct UserMessageSent {
    pub payload: Vec<u8>,
}

pub struct AuthoritySetState {
    pub authority_set_id: u64,
    pub authority_set_hash: [u8; BLAKE2_HASH_SIZE],
}
