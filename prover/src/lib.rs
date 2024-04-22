#![allow(incomplete_features)]

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

mod block_finality;
pub(crate) mod common;
mod final_proof;
mod latest_validator_set;
mod storage_inclusion;

pub(crate) mod prelude {
    use plonky2::field::goldilocks_field::GoldilocksField;
    use plonky2::plonk::config::PoseidonGoldilocksConfig;

    pub type F = GoldilocksField;
    pub type C = PoseidonGoldilocksConfig;
    pub const D: usize = 2;

    pub use super::consts;
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

    pub const MAX_VALIDATOR_COUNT: usize = 6;

    pub const GENESIS_AUTHORITY_SET_ID: u64 = 272;
    pub const GENESIS_VALIDATOR_SET_HASH: [u64; BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS] = [
        0x3E453535, 0x39A1043B, 0x8D1265E7, 0xB47E1B16, 0x3E263D42, 0x9EF6888C, 0x137A20F1,
        0x6696DBA1,
    ];
}

pub mod proving {
    use crate::prelude::*;
    use crate::{
        common::{self, targets::TargetSet},
        final_proof::{message_sent::MessageSent, FinalProof},
        latest_validator_set::{
            next_validator_set::{NextValidatorSet, NextValidatorSetTarget},
            LatestValidatorSet,
        },
    };
    use plonky2::{
        plonk::{
            circuit_data::{CommonCircuitData, VerifierCircuitData},
            proof::ProofWithPublicInputs,
        },
        util::serialization::DefaultGateSerializer,
    };
    use serde::{Deserialize, Serialize};

    pub use crate::block_finality::{BlockFinality, PreCommit};
    pub use crate::storage_inclusion::{BranchNodeData, StorageInclusion};

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

    #[derive(Clone, Serialize, Deserialize)]
    pub struct ExportedProofWithCircuitData {
        pub proof_with_public_inputs: String,
        pub common_circuit_data: String,
        pub verifier_only_circuit_data: String,
    }

    pub fn prove_genesis(
        current_epoch_block_finality: BlockFinality,
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
        .prove_genesis();

        ProofWithCircuitData::from_plonky2_repr(&proof)
    }

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

    pub fn prove_message_sent(
        previous_proof: ProofWithCircuitData,
        block_finality_proof: BlockFinality,
        message_inclusion_proof: StorageInclusion,
        message_contents: Vec<u8>,
    ) -> ExportedProofWithCircuitData {
        let message_sent = MessageSent {
            block_finality: block_finality_proof,
            inclusion_proof: message_inclusion_proof,
            message_storage_data: message_contents,
        };

        let previous_proof: common::ProofWithCircuitData<NextValidatorSetTarget> =
            previous_proof.into_plonky2_repr();

        FinalProof {
            current_validator_set_verifier_data: previous_proof.circuit_data().clone(),
            current_validator_set_proof: previous_proof.proof(),
            message_sent,
        }
        .prove();

        todo!()
    }
}
