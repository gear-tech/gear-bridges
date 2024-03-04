use std::{marker::PhantomData, path::Path, sync::Arc};

use crate::prelude::*;
use plonky2::{
    hash::hash_types::HashOutTarget,
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, VerifierCircuitData, VerifierCircuitTarget},
        config::{GenericConfig, Hasher},
        proof::{Proof, ProofWithPublicInputs},
    },
};

#[macro_use]
pub mod targets;
pub mod bn128_wrapper;
pub mod poseidon_bn128;

use plonky2_field::goldilocks_field::GoldilocksField;
use targets::TargetSet;

type CircuitDigest = <<C as GenericConfig<D>>::Hasher as Hasher<F>>::Hash;

#[derive(Clone)]
pub struct ProofWithCircuitData<TS>
where
    TS: TargetSet,
{
    proof: Proof<F, C, D>,
    circuit_data: Arc<VerifierCircuitData<F, C, D>>,

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
            circuit_data: Arc::from(circuit_data.verifier_data()),
            public_inputs,
            public_inputs_parser: PhantomData,
        }
    }

    pub fn from_circuit_data(
        circuit_data: CircuitData<F, C, D>,
        witness: PartialWitness<F>,
    ) -> ProofWithCircuitData<TS> {
        let ProofWithPublicInputs {
            proof,
            public_inputs,
        } = circuit_data.prove(witness).unwrap();

        ProofWithCircuitData {
            proof,
            circuit_data: Arc::from(circuit_data.verifier_data()),
            public_inputs,
            public_inputs_parser: PhantomData,
        }
    }

    // TODO: REMOVE
    pub fn circuit_digest(&self) -> CircuitDigest {
        self.circuit_data.verifier_only.circuit_digest
    }

    // TODO: REMOVE
    pub fn circuit_data(&self) -> &VerifierCircuitData<F, C, D> {
        &self.circuit_data
    }

    // TODO: REMOVE
    pub fn pis(&self) -> Vec<GoldilocksField> {
        self.public_inputs.clone()
    }

    // TODO: REMOVE
    pub fn proof(&self) -> ProofWithPublicInputs<F, C, D> {
        ProofWithPublicInputs {
            proof: self.proof.clone(),
            public_inputs: self.public_inputs.clone(),
        }
    }

    pub fn wrap_bn128(self) {
        let pwpi = ProofWithPublicInputs {
            proof: self.proof,
            public_inputs: self.public_inputs,
        };
        bn128_wrapper::wrap(&self.circuit_data, pwpi);
    }

    pub fn verify(&self) -> bool {
        self.circuit_data
            .verify(ProofWithPublicInputs {
                proof: self.proof.clone(),
                public_inputs: self.public_inputs.clone(),
            })
            .is_ok()
    }
}

pub struct ProofComposition<TS1, TS2>
where
    TS1: TargetSet,
    TS2: TargetSet,
{
    circuit_builder: CircuitBuilder<F, D>,
    witness: PartialWitness<F>,

    first_public_inputs: TS1,
    second_public_inputs: TS2,

    first_circuit_digest: CircuitDigest,
    second_circuit_digest: CircuitDigest,

    first_circuit_digest_target: HashOutTarget,
    second_circuit_digest_target: HashOutTarget,
}

pub struct ExtendedComposeArgs<TS1, TS2>
where
    TS1: TargetSet,
    TS2: TargetSet,
{
    pub first_target_set: TS1,
    pub second_target_set: TS2,

    pub first_circuit_digest: HashOutTarget,
    pub second_circuit_digest: HashOutTarget,
}

impl<TS1, TS2> ProofComposition<TS1, TS2>
where
    TS1: TargetSet,
    TS2: TargetSet,
{
    pub fn new(
        first: ProofWithCircuitData<TS1>,
        second: ProofWithCircuitData<TS2>,
    ) -> ProofComposition<TS1, TS2> {
        Self::new_with_config(first, second, CircuitConfig::standard_recursion_config())
    }

    pub fn new_with_config(
        first: ProofWithCircuitData<TS1>,
        second: ProofWithCircuitData<TS2>,
        config: CircuitConfig,
    ) -> ProofComposition<TS1, TS2> {
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let proof_with_pis_target_1 =
            builder.add_virtual_proof_with_pis(&first.circuit_data.common);
        let proof_with_pis_target_2 =
            builder.add_virtual_proof_with_pis(&second.circuit_data.common);

        let first_circuit_digest = first.circuit_digest();
        let second_circuit_digest = second.circuit_digest();

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

        let first_public_inputs =
            TS1::parse_exact(&mut proof_with_pis_target_1.public_inputs.into_iter());

        let second_public_inputs =
            TS2::parse_exact(&mut proof_with_pis_target_2.public_inputs.into_iter());

        ProofComposition {
            circuit_builder: builder,
            witness: pw,

            first_public_inputs,
            second_public_inputs,

            first_circuit_digest,
            second_circuit_digest,

            first_circuit_digest_target: verifier_circuit_target_1.circuit_digest,
            second_circuit_digest_target: verifier_circuit_target_2.circuit_digest,
        }
    }

    // TODO: assert merkle caps too.
    pub fn assert_both_circuit_digests(self) -> Self {
        self.assert_first_circuit_digest()
            .assert_second_circuit_digest()
    }

    pub fn assert_first_circuit_digest(mut self) -> Self {
        let value_target = self
            .circuit_builder
            .constant_hash(self.first_circuit_digest);
        self.circuit_builder
            .connect_hashes(value_target, self.first_circuit_digest_target);
        self
    }

    pub fn assert_second_circuit_digest(mut self) -> Self {
        let value_target = self
            .circuit_builder
            .constant_hash(self.second_circuit_digest);
        self.circuit_builder
            .connect_hashes(value_target, self.second_circuit_digest_target);
        self
    }

    pub fn compose<O, TS>(mut self, op: O) -> ProofWithCircuitData<TS>
    where
        TS: TargetSet,
        O: Fn(&mut CircuitBuilder<F, D>, TS1, TS2) -> TS,
    {
        let target_set = op(
            &mut self.circuit_builder,
            self.first_public_inputs.clone(),
            self.second_public_inputs.clone(),
        );

        target_set.register_as_public_inputs(&mut self.circuit_builder);

        ProofWithCircuitData::from_builder(self.circuit_builder, self.witness)
    }

    pub fn extended_compose<O, TS>(mut self, op: O) -> ProofWithCircuitData<TS>
    where
        TS: TargetSet,
        O: Fn(&mut CircuitBuilder<F, D>, ExtendedComposeArgs<TS1, TS2>) -> TS,
    {
        let target_set = op(
            &mut self.circuit_builder,
            ExtendedComposeArgs {
                first_target_set: self.first_public_inputs.clone(),
                second_target_set: self.second_public_inputs.clone(),

                first_circuit_digest: self.first_circuit_digest_target,
                second_circuit_digest: self.second_circuit_digest_target,
            },
        );

        target_set.register_as_public_inputs(&mut self.circuit_builder);

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
