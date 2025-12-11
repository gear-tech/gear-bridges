#![allow(incomplete_features)]

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

mod block_finality;
pub(crate) mod common;
mod final_proof;
pub mod header_chain;
mod latest_validator_set;
pub mod serialization;
mod storage_inclusion;
pub mod utils;

pub type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

pub(crate) mod prelude {
    use plonky2::{
        field::goldilocks_field::GoldilocksField, plonk::config::PoseidonGoldilocksConfig,
    };

    pub type F = GoldilocksField;
    pub type C = PoseidonGoldilocksConfig;
    pub const D: usize = 2;

    pub use super::{consts, GearHeader};
}

pub mod consts {
    pub const CIRCUIT_DIGEST_SIZE: usize = 4;

    // For now we send a single Keccak256 hash.
    pub const MESSAGE_SIZE: usize = 32;
    pub const MESSAGE_SIZE_IN_BITS: usize = MESSAGE_SIZE * 8;
    pub const MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS: usize = 8;

    pub const BLAKE2_DIGEST_SIZE: usize = 32;
    pub const BLAKE2_DIGEST_SIZE_IN_BITS: usize = BLAKE2_DIGEST_SIZE * 8;
    /// If we pack `BoolTargets` into `Targets` by groups of 32 then
    /// 8 Goldilocks field elements are required.
    pub const BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS: usize =
        BLAKE2_DIGEST_SIZE_IN_BITS / 32;

    pub const ED25519_PUBLIC_KEY_SIZE: usize = 32;
    pub const ED25519_PUBLIC_KEY_SIZE_IN_BITS: usize = ED25519_PUBLIC_KEY_SIZE * 8;

    pub const ED25519_SIGNATURE_SIZE: usize = 64;
    pub const ED25519_SIGNATURE_SIZE_IN_BITS: usize = ED25519_SIGNATURE_SIZE * 8;

    pub const GRANDPA_VOTE_LENGTH: usize = 53;

    pub const MAX_VALIDATOR_COUNT: usize = 64;

    // 4MiB
    pub const SIZE_THREAD_STACK_MIN: usize = 4_194_304;
}

pub mod proving {
    use crate::{
        common::{self, targets::TargetSet},
        final_proof::{message_sent::MessageSent, FinalProof},
        latest_validator_set::{
            next_validator_set::{NextValidatorSet, NextValidatorSetTarget},
            LatestValidatorSet,
        },
        prelude::*,
    };
    use consts::BLAKE2_DIGEST_SIZE;
    use plonky2::{
        plonk::{
            circuit_data::{CommonCircuitData, VerifierCircuitData},
            proof::ProofWithPublicInputs,
        },
        util::serialization::DefaultGateSerializer,
    };
    use serde::{Deserialize, Serialize};

    pub use crate::{
        block_finality::{BlockFinality, PreCommit},
        storage_inclusion::{BranchNodeData, StorageInclusion},
    };

    use self::consts::BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS;

    /// Represents proof along with public inputs. Can be verified using `CircuitData`.
    #[derive(Clone)]
    pub struct Proof(pub Vec<u8>);

    impl Proof {
        fn from_plonky2_repr(proof: &ProofWithPublicInputs<F, C, D>) -> Self {
            Self(proof.to_bytes())
        }

        fn into_plonky2_repr(
            self,
            common_data: &CommonCircuitData<F, D>,
        ) -> ProofWithPublicInputs<F, C, D> {
            ProofWithPublicInputs::from_bytes(self.0, common_data)
                .expect("Failed to deserialize proof")
        }

        pub fn from_bytes(data: Vec<u8>) -> Self {
            Self(data)
        }

        pub fn into_bytes(self) -> Vec<u8> {
            self.0
        }
    }

    /// Represents circuit data that's sufficient to verify a `Proof`.
    #[derive(Clone)]
    pub struct CircuitData(pub Vec<u8>);

    impl CircuitData {
        fn from_plonky2_repr(circuit_data: &VerifierCircuitData<F, C, D>) -> Self {
            Self(
                circuit_data
                    .to_bytes(&DefaultGateSerializer)
                    .expect("Failed to serialize circuit data"),
            )
        }

        fn into_plonky2_repr(self) -> VerifierCircuitData<F, C, D> {
            VerifierCircuitData::from_bytes(self.0, &DefaultGateSerializer)
                .expect("Failed to deserialize circuit data")
        }

        pub fn from_bytes(data: Vec<u8>) -> Self {
            Self(data)
        }

        pub fn into_bytes(self) -> Vec<u8> {
            self.0
        }
    }

    #[derive(Clone)]
    pub struct ProofWithCircuitData {
        pub proof: Proof,
        pub circuit_data: CircuitData,
    }

    impl ProofWithCircuitData {
        fn from_plonky2_repr<TS: TargetSet>(
            proof_with_circuit_data: &common::ProofWithCircuitData<TS>,
        ) -> Self {
            Self {
                proof: Proof::from_plonky2_repr(&proof_with_circuit_data.proof()),
                circuit_data: CircuitData::from_plonky2_repr(
                    proof_with_circuit_data.circuit_data(),
                ),
            }
        }

        fn into_plonky2_repr<TS: TargetSet>(self) -> common::ProofWithCircuitData<TS> {
            let circuit_data = self.circuit_data.into_plonky2_repr();

            common::ProofWithCircuitData::from_proof_and_circuit_data(
                self.proof.into_plonky2_repr(&circuit_data.common),
                circuit_data,
            )
        }
    }

    /// All the data that's exported to `gnark-wrapper` as `JSON` strings.
    #[derive(Clone, Serialize, Deserialize)]
    pub struct ExportedProofWithCircuitData {
        pub proof_with_public_inputs: String,
        pub common_circuit_data: String,
        pub verifier_only_circuit_data: String,
    }

    /// Genesis config of a bridge. Note that any change in genesis config renders all the subsequent
    /// proofs invalid(they will NOT get verified on ethereum) as circuit digest will change.
    #[derive(Clone, Copy)]
    pub struct GenesisConfig {
        pub authority_set_id: u64,
        pub authority_set_hash: [u8; BLAKE2_DIGEST_SIZE],
    }

    impl GenesisConfig {
        pub fn authority_set_hash_goldilocks(
            &self,
        ) -> [u64; BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS] {
            self.authority_set_hash
                .chunks(4)
                .map(|limb_bytes| {
                    u32::from_be_bytes(limb_bytes.try_into().expect("Expected 4 bytes per limb"))
                        as u64
                })
                .collect::<Vec<_>>()
                .try_into()
                .expect("Incorrect amount of limbs")
        }
    }

    /// Prove very first transition from genesis authority set to the subsequent.
    /// # Arguments
    ///
    /// * `current_epoch_block_finality` - Finality proof of some block present in the last epoch of
    ///   the first bridge era. The first bridge era is defined in `genesis_config`
    /// * `genesis_config` - Defines starting point for all the bridge operation.
    /// * `next_validator_set_inclusion_proof` - Proof of inclusion of next authority set into
    ///   storage of the block that proven final by `current_epoch_block_finality`.
    /// * `next_validator_set_data` - Hash of next validator set(as it's stored in the
    ///   `pallet_gear_bridge` storage).
    pub fn prove_genesis(
        current_epoch_block_finality: BlockFinality,
        genesis_config: GenesisConfig,
        next_validator_set_inclusion_proof: StorageInclusion,
        next_validator_set_data: Vec<u8>,
    ) -> ProofWithCircuitData {
        let change_from_genesis = NextValidatorSet {
            current_epoch_block_finality,
            next_validator_set_inclusion_proof,
            next_validator_set_storage_data: next_validator_set_data,
        };

        let proof = LatestValidatorSet {
            change_proof: change_from_genesis,
        }
        .prove_genesis(genesis_config);

        ProofWithCircuitData::from_plonky2_repr(&proof)
    }

    /// Add one more layer to the proof of authority set changes.
    ///
    /// # Arguments
    ///
    /// * `previous_proof` - Proof that we previously composed. Proves transition of authority set
    ///   from genesis to current.
    /// * `current_epoch_block_finality` - Finality proof of some block present in the last epoch of
    ///   the current era.
    /// * `next_validator_set_inclusion_proof` - Proof of inclusion of next authority set into
    ///   storage of block that's proven final by `current_epoch_block_finality`.
    /// * `next_validator_set_data` - Hash of next validator set(as it's stored in the
    ///   `pallet_gear_bridge` storage).
    pub fn prove_validator_set_change(
        previous_proof: ProofWithCircuitData,
        current_epoch_block_finality: BlockFinality,
        next_validator_set_inclusion_proof: StorageInclusion,
        next_validator_set_data: Vec<u8>,
    ) -> ProofWithCircuitData {
        let next_change = NextValidatorSet {
            current_epoch_block_finality,
            next_validator_set_inclusion_proof,
            next_validator_set_storage_data: next_validator_set_data,
        };

        let previous_proof: common::ProofWithCircuitData<NextValidatorSetTarget> =
            previous_proof.into_plonky2_repr();

        let proof = LatestValidatorSet {
            change_proof: next_change,
        }
        .prove_recursive(previous_proof.proof());

        ProofWithCircuitData::from_plonky2_repr(&proof)
    }

    /// Add one layer on top of authority set change proof chain that will prove that merkle trie
    /// root of queued messages is present in storage of some finalized block.
    ///
    /// # Arguments
    ///
    /// * `previous_proof` - Proof that proves transition of authority set from genesis to current.
    ///   It can be obtained by calling `prove_genesis` once and then `prove_validator_set_change`
    ///   `N` times where `N` means number of eras passes from bridge genesis to the current era.
    /// * `block_finality_proof` - Finality proof of block that contain merkle trie root(in the
    ///   storage of `pallet-gear-bridge`) that we want to relay.
    /// * `genesis_config` - `GenesisConfig` that was used in the `prove_genesis` call.
    /// * `message_inclusion_proof` - Proof of inclusion of merkle trie root of queued messages into
    ///   storage of `pallet-gear-bridge`.
    /// * `message_contents` - Root of merkle trie root as it's stored in `pallet-gear-bridge`
    ///   storage.
    pub fn prove_message_sent(
        previous_proof: ProofWithCircuitData,
        block_finality_proof: BlockFinality,
        headers: Vec<GearHeader>,
        genesis_config: GenesisConfig,
        message_inclusion_proof: StorageInclusion,
        message_contents: Vec<u8>,
    ) -> ExportedProofWithCircuitData {
        let message_sent = MessageSent {
            block_finality: block_finality_proof,
            headers,
            inclusion_proof: message_inclusion_proof,
            message_storage_data: message_contents,
        };

        let previous_proof: common::ProofWithCircuitData<NextValidatorSetTarget> =
            previous_proof.into_plonky2_repr();

        let proof = FinalProof {
            current_validator_set_verifier_data: previous_proof.circuit_data().clone(),
            current_validator_set_proof: previous_proof.proof(),
            message_sent,
        }
        .prove(genesis_config);

        proof.export_wrapped()
    }
}
