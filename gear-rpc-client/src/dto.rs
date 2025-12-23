use crate::GearHeader;
use parity_scale_codec::{Decode, Encode};
use primitive_types::H256;
use serde::{
    de::{self, MapAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Serialize,
};
use sp_consensus_grandpa::{self, Precommit};
use std::fmt;

const ED25519_PUBLIC_KEY_SIZE: usize = 32;
const ED25519_SIGNATURE_SIZE: usize = 64;
const KECCAK_HASH_SIZE: usize = 32;
const BLAKE2_HASH_SIZE: usize = 32;
const VOTE_LENGTH_IN_BITS: usize = 424;

#[derive(Debug, Encode, Decode, Clone)]
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

#[derive(Clone, Debug)]
pub struct RawBlockInclusionProof {
    pub justification_round: u64,
    pub required_authority_set_id: u64,
    pub validator_set: Vec<[u8; ED25519_PUBLIC_KEY_SIZE]>,
    pub block_hash: H256,
    pub block_number: u32,
    pub pre_commits: Vec<PreCommit>,
}

/* manual serialization to hex encode some fields */
impl Serialize for RawBlockInclusionProof {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("RawBlockInclusionProof", 7)?;
        state.serialize_field("justification_round", &self.justification_round)?;
        state.serialize_field("required_authority_set_id", &self.required_authority_set_id)?;

        let validator_set: Vec<String> = self.validator_set.iter().map(hex::encode).collect();
        state.serialize_field("validator_set", &validator_set)?;

        state.serialize_field("block_hash", &hex::encode(self.block_hash.as_bytes()))?;
        state.serialize_field("block_number", &self.block_number)?;
        state.serialize_field("pre_commits", &self.pre_commits)?;
        state.end()
    }
}

/* manual deserialization because some fields are hex encoded */
impl<'de> Deserialize<'de> for RawBlockInclusionProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            justification_round: u64,
            required_authority_set_id: u64,

            validator_set: Vec<String>,
            block_hash: String,
            block_number: u32,
            pre_commits: Vec<PreCommit>,
        }

        let helper = Helper::deserialize(deserializer)?;

        let mut validator_set = Vec::with_capacity(helper.validator_set.len());
        for s in helper.validator_set {
            let pk = hex::decode(s).map_err(serde::de::Error::custom)?;
            if pk.len() != ED25519_PUBLIC_KEY_SIZE {
                return Err(serde::de::Error::custom(
                    "validator_set entry has wrong length",
                ));
            }
            let mut arr = [0u8; ED25519_PUBLIC_KEY_SIZE];
            arr.copy_from_slice(&pk);
            validator_set.push(arr);
        }

        let block_hash_bytes = hex::decode(helper.block_hash).map_err(serde::de::Error::custom)?;
        if block_hash_bytes.len() != 32 {
            return Err(serde::de::Error::custom("block_hash must be 32 bytes"));
        }
        let block_hash = H256::from_slice(&block_hash_bytes);

        Ok(Self {
            justification_round: helper.justification_round,
            required_authority_set_id: helper.required_authority_set_id,

            validator_set,
            block_hash,
            block_number: helper.block_number,
            pre_commits: helper.pre_commits,
        })
    }
}

impl From<RawBlockInclusionProof> for (H256, BlockFinalityProof) {
    fn from(this: RawBlockInclusionProof) -> (H256, BlockFinalityProof) {
        let signed_data = sp_consensus_grandpa::localized_payload(
            this.justification_round,
            this.required_authority_set_id,
            &sp_consensus_grandpa::Message::<GearHeader>::Precommit(Precommit::<GearHeader>::new(
                this.block_hash,
                this.block_number,
            )),
        );

        if signed_data.len() * 8 != VOTE_LENGTH_IN_BITS {
            log::error!(
                "Signed data has incorrect length: expected {}, got {}",
                VOTE_LENGTH_IN_BITS,
                signed_data.len() * 8
            );
        }

        (
            this.block_hash,
            BlockFinalityProof {
                validator_set: this.validator_set,
                message: signed_data,
                pre_commits: this.pre_commits,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_block_inclusion_proof_hex_serde_roundtrip() {
        let p = RawBlockInclusionProof {
            justification_round: 42,
            required_authority_set_id: 7,

            validator_set: vec![
                [0xAB; ED25519_PUBLIC_KEY_SIZE],
                [0xCD; ED25519_PUBLIC_KEY_SIZE],
            ],
            block_hash: H256::from_low_u64_be(0x22),
            block_number: 999,
            pre_commits: vec![PreCommit {
                public_key: [0xEF; ED25519_PUBLIC_KEY_SIZE],
                signature: [0x01; ED25519_SIGNATURE_SIZE],
            }],
        };

        let json = serde_json::to_string(&p).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("json value");

        // Ensure these are encoded as hex strings (not arrays of ints).
        assert!(v.get("block_hash").unwrap().as_str().is_some());
        assert!(v.get("validator_set").unwrap().as_array().unwrap()[0]
            .as_str()
            .is_some());
        assert!(v.get("precommit").unwrap().as_array().unwrap()[0]
            .as_str()
            .is_some());

        let decoded: RawBlockInclusionProof = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.justification_round, p.justification_round);
        assert_eq!(
            decoded.required_authority_set_id,
            p.required_authority_set_id
        );

        assert_eq!(decoded.validator_set, p.validator_set);
        assert_eq!(decoded.block_hash, p.block_hash);
        assert_eq!(decoded.block_number, p.block_number);
        assert_eq!(decoded.pre_commits.len(), p.pre_commits.len());
    }
}
