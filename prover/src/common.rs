use std::marker::PhantomData;

use crate::{
    consts::{
        BLAKE2_DIGEST_SIZE_IN_BITS, ED25519_PUBLIC_KEY_SIZE_IN_BITS,
        ED25519_SIGNATURE_SIZE_IN_BITS, SHA256_DIGEST_SIZE_IN_BITS,
    },
    prelude::*,
};
use circom_verifier::CircomVerifierFilePaths;
use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, VerifierCircuitTarget},
        config::PoseidonGoldilocksConfig,
        proof::{Proof, ProofWithPublicInputs},
    },
};

pub use targets::TargetSet;

pub mod targets {
    use std::fmt::Debug;
    use std::ops::Deref;

    use super::*;

    pub trait TargetSet: Clone {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self;
    }

    pub trait TargetSetOperations {
        fn register_as_public_inputs(&self, builder: &mut CircuitBuilder<F, D>);
        fn connect(&self, other: &Self, builder: &mut CircuitBuilder<F, D>);
        fn set_partial_witness(&self, data: &[u8], witness: &mut PartialWitness<F>);
    }

    pub type CompositeTarget<T, const N: usize> = [T; N];

    impl<const N: usize> TargetSet for CompositeTarget<BoolTarget, N> {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            parse_composite_target(raw, BoolTarget::new_unsafe)
        }
    }

    impl<const N: usize> TargetSet for CompositeTarget<Target, N> {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            parse_composite_target(raw, |t| t)
        }
    }

    fn parse_composite_target<T: Debug, const N: usize>(
        raw: &mut impl Iterator<Item = Target>,
        mapping: impl Fn(Target) -> T,
    ) -> CompositeTarget<T, N> {
        raw.take(N)
            .map(mapping)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    impl<T, const N: usize> TargetSetOperations for T
    where
        T: Deref<Target = CompositeTarget<BoolTarget, N>>,
    {
        fn connect(&self, other: &Self, builder: &mut CircuitBuilder<F, D>) {
            for (target_1, target_2) in self.iter().zip(other.iter()) {
                builder.connect(target_1.target, target_2.target);
            }
        }

        fn register_as_public_inputs(&self, builder: &mut CircuitBuilder<F, D>) {
            for target in self.iter() {
                builder.register_public_input(target.target);
            }
        }

        fn set_partial_witness(&self, data: &[u8], witness: &mut PartialWitness<F>) {
            let data = array_to_bits(data);
            for (target, bit) in self.iter().zip(data.into_iter()) {
                witness.set_bool_target(*target, bit);
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct SingleTarget(Target);

    impl SingleTarget {
        pub fn to_target(&self) -> Target {
            self.0
        }
    }

    #[derive(Clone, Debug)]
    pub struct Sha256Target(CompositeTarget<BoolTarget, SHA256_DIGEST_SIZE_IN_BITS>);

    #[derive(Clone, Debug)]
    pub struct Blake2Target(CompositeTarget<BoolTarget, BLAKE2_DIGEST_SIZE_IN_BITS>);

    #[derive(Clone, Debug)]
    pub struct Ed25519PublicKeyTarget(CompositeTarget<BoolTarget, ED25519_PUBLIC_KEY_SIZE_IN_BITS>);

    #[derive(Clone, Debug)]
    pub struct Ed25519SignatreTarget(CompositeTarget<BoolTarget, ED25519_SIGNATURE_SIZE_IN_BITS>);

    #[derive(Clone, Debug)]
    pub struct BitArrayTarget<const N: usize>(CompositeTarget<BoolTarget, N>);

    impl Deref for Sha256Target {
        type Target = CompositeTarget<BoolTarget, SHA256_DIGEST_SIZE_IN_BITS>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Deref for Blake2Target {
        type Target = CompositeTarget<BoolTarget, BLAKE2_DIGEST_SIZE_IN_BITS>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Deref for Ed25519PublicKeyTarget {
        type Target = CompositeTarget<BoolTarget, ED25519_PUBLIC_KEY_SIZE_IN_BITS>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Deref for Ed25519SignatreTarget {
        type Target = CompositeTarget<BoolTarget, ED25519_SIGNATURE_SIZE_IN_BITS>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<const N: usize> Deref for BitArrayTarget<N> {
        type Target = CompositeTarget<BoolTarget, N>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl TargetSetOperations for SingleTarget {
        fn connect(&self, other: &Self, builder: &mut CircuitBuilder<F, D>) {
            builder.connect(self.0, other.0)
        }

        fn register_as_public_inputs(&self, builder: &mut CircuitBuilder<F, D>) {
            builder.register_public_input(self.0)
        }

        fn set_partial_witness(&self, data: &[u8], witness: &mut PartialWitness<F>) {
            unimplemented!("Set SingleTarget manually by calling .to_target()");
        }
    }

    impl TargetSet for SingleTarget {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            Self(raw.next().unwrap())
        }
    }

    impl TargetSet for Sha256Target {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            Self(TargetSet::parse(raw))
        }
    }

    impl TargetSet for Blake2Target {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            Self(TargetSet::parse(raw))
        }
    }

    impl TargetSet for Ed25519PublicKeyTarget {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            Self(TargetSet::parse(raw))
        }
    }

    impl TargetSet for Ed25519SignatreTarget {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            Self(TargetSet::parse(raw))
        }
    }

    impl<const N: usize> TargetSet for BitArrayTarget<N> {
        fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
            Self(TargetSet::parse(raw))
        }
    }

    impl From<Target> for SingleTarget {
        fn from(value: Target) -> Self {
            Self(value)
        }
    }

    impl From<[BoolTarget; SHA256_DIGEST_SIZE_IN_BITS]> for Sha256Target {
        fn from(value: [BoolTarget; SHA256_DIGEST_SIZE_IN_BITS]) -> Self {
            Self(value)
        }
    }

    impl From<[BoolTarget; BLAKE2_DIGEST_SIZE_IN_BITS]> for Blake2Target {
        fn from(value: [BoolTarget; BLAKE2_DIGEST_SIZE_IN_BITS]) -> Self {
            Self(value)
        }
    }

    impl From<[BoolTarget; ED25519_PUBLIC_KEY_SIZE_IN_BITS]> for Ed25519PublicKeyTarget {
        fn from(value: [BoolTarget; ED25519_PUBLIC_KEY_SIZE_IN_BITS]) -> Self {
            Self(value)
        }
    }

    impl From<[BoolTarget; ED25519_SIGNATURE_SIZE_IN_BITS]> for Ed25519SignatreTarget {
        fn from(value: [BoolTarget; ED25519_SIGNATURE_SIZE_IN_BITS]) -> Self {
            Self(value)
        }
    }
}

pub struct ProofWithCircuitData<TS>
where
    TS: TargetSet,
{
    proof: Proof<F, C, D>,
    circuit_data: CircuitData<F, C, D>,

    public_inputs: Vec<F>,
    public_inputs_parser: PhantomData<TS>,
}

impl<TS> ProofWithCircuitData<TS>
where
    TS: TargetSet,
{
    pub fn from_builder(
        builder: CircuitBuilder<F, D>,
        witness: PartialWitness<F>,
    ) -> ProofWithCircuitData<TS> {
        let circuit_data = builder.build::<C>();
        let ProofWithPublicInputs {
            proof,
            public_inputs,
        } = circuit_data.prove(witness).unwrap();

        ProofWithCircuitData {
            proof,
            circuit_data,
            public_inputs,
            public_inputs_parser: PhantomData,
        }
    }

    pub fn verify(&self) -> bool {
        self.circuit_data
            .verify(ProofWithPublicInputs {
                proof: self.proof.clone(),
                public_inputs: self.public_inputs.clone(),
            })
            .is_ok()
    }

    pub fn generate_circom_verifier(self, paths: CircomVerifierFilePaths) {
        circom_verifier::write_circom_verifier_files(
            paths,
            self.circuit_data.common,
            self.circuit_data.verifier_only,
            ProofWithPublicInputs {
                proof: self.proof,
                public_inputs: self.public_inputs,
            },
        )
    }
}

pub struct ProofCompositionTargets<TS1, TS2>
where
    TS1: TargetSet,
    TS2: TargetSet,
{
    pub first_proof_public_inputs: TS1,
    pub second_proof_public_inputs: TS2,
}

pub struct ProofCompositionBuilder<TS1, TS2>
where
    TS1: TargetSet,
    TS2: TargetSet,
{
    circuit_builder: CircuitBuilder<F, D>,
    witness: PartialWitness<F>,

    first_public_inputs: TS1,
    second_public_inputs: TS2,
}

impl<TS1, TS2> ProofCompositionBuilder<TS1, TS2>
where
    TS1: TargetSet,
    TS2: TargetSet,
{
    pub fn new(
        first: ProofWithCircuitData<TS1>,
        second: ProofWithCircuitData<TS2>,
    ) -> ProofCompositionBuilder<TS1, TS2> {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_recursion_config());
        let proof_with_pis_target_1 =
            builder.add_virtual_proof_with_pis::<C>(&first.circuit_data.common);
        let proof_with_pis_target_2 =
            builder.add_virtual_proof_with_pis::<C>(&second.circuit_data.common);

        let verifier_circuit_target_1 = VerifierCircuitTarget {
            constants_sigmas_cap: builder
                .add_virtual_cap(first.circuit_data.common.config.fri_config.cap_height),
            circuit_digest: builder.add_virtual_hash(),
        };
        let verifier_circuit_target_2 = VerifierCircuitTarget {
            constants_sigmas_cap: builder
                .add_virtual_cap(second.circuit_data.common.config.fri_config.cap_height),
            circuit_digest: builder.add_virtual_hash(),
        };

        let first_hash_target =
            builder.constant_hash(first.circuit_data.verifier_only.circuit_digest);
        builder.connect_hashes(first_hash_target, verifier_circuit_target_1.circuit_digest);

        let second_hash_target =
            builder.constant_hash(second.circuit_data.verifier_only.circuit_digest);
        builder.connect_hashes(second_hash_target, verifier_circuit_target_2.circuit_digest);

        let mut pw = PartialWitness::new();
        pw.set_proof_with_pis_target(
            &proof_with_pis_target_1,
            &ProofWithPublicInputs {
                proof: first.proof,
                public_inputs: first.public_inputs,
            },
        );
        pw.set_proof_with_pis_target(
            &proof_with_pis_target_2,
            &ProofWithPublicInputs {
                proof: second.proof,
                public_inputs: second.public_inputs,
            },
        );
        pw.set_cap_target(
            &verifier_circuit_target_1.constants_sigmas_cap,
            &first.circuit_data.verifier_only.constants_sigmas_cap,
        );
        pw.set_cap_target(
            &verifier_circuit_target_2.constants_sigmas_cap,
            &second.circuit_data.verifier_only.constants_sigmas_cap,
        );
        pw.set_hash_target(
            verifier_circuit_target_1.circuit_digest,
            first.circuit_data.verifier_only.circuit_digest,
        );
        pw.set_hash_target(
            verifier_circuit_target_2.circuit_digest,
            second.circuit_data.verifier_only.circuit_digest,
        );

        builder.verify_proof::<C>(
            &proof_with_pis_target_1,
            &verifier_circuit_target_1,
            &first.circuit_data.common,
        );
        builder.verify_proof::<C>(
            &proof_with_pis_target_2,
            &verifier_circuit_target_2,
            &second.circuit_data.common,
        );

        ProofCompositionBuilder {
            circuit_builder: builder,
            witness: pw,

            first_public_inputs: TS1::parse(&mut proof_with_pis_target_1.public_inputs.into_iter()),
            second_public_inputs: TS2::parse(
                &mut proof_with_pis_target_2.public_inputs.into_iter(),
            ),
        }
    }

    pub fn operation_with_targets<O>(mut self, op: O) -> ProofCompositionBuilder<TS1, TS2>
    where
        O: Fn(&mut CircuitBuilder<F, D>, ProofCompositionTargets<TS1, TS2>),
    {
        op(
            &mut self.circuit_builder,
            ProofCompositionTargets {
                first_proof_public_inputs: self.first_public_inputs.clone(),
                second_proof_public_inputs: self.second_public_inputs.clone(),
            },
        );

        self
    }

    pub fn build<TS>(self) -> ProofWithCircuitData<TS>
    where
        TS: TargetSet,
    {
        ProofWithCircuitData::from_builder(self.circuit_builder, self.witness)
    }
}

pub fn array_to_bits(data: &[u8]) -> Vec<bool> {
    data.iter().copied().flat_map(byte_to_bits).collect()
}

fn byte_to_bits(byte: u8) -> [bool; 8] {
    (0..8)
        .rev()
        .map(move |bit_idx| (byte >> bit_idx) % 2 == 1)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}
