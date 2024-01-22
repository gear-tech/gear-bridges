use std::marker::PhantomData;

use crate::prelude::*;
use circom_verifier::CircomVerifierFilePaths;
use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, VerifierCircuitTarget},
        proof::{Proof, ProofWithPublicInputs},
    },
};

#[macro_use]
pub mod targets;

use targets::TargetSet;

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
        Self::new_with_config(first, second, CircuitConfig::standard_recursion_config())
    }

    pub fn new_with_config(
        first: ProofWithCircuitData<TS1>,
        second: ProofWithCircuitData<TS2>,
        config: CircuitConfig,
    ) -> ProofCompositionBuilder<TS1, TS2> {
        let mut builder = CircuitBuilder::<F, D>::new(config);
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

    pub fn build<O, TS>(mut self, op: O) -> ProofWithCircuitData<TS>
    where
        TS: TargetSet,
        O: Fn(&mut CircuitBuilder<F, D>, ProofCompositionTargets<TS1, TS2>) -> TS,
    {
        let target_set = op(
            &mut self.circuit_builder,
            ProofCompositionTargets {
                first_proof_public_inputs: self.first_public_inputs.clone(),
                second_proof_public_inputs: self.second_public_inputs.clone(),
            },
        );

        for target in target_set.into_targets_iter() {
            self.circuit_builder.register_public_input(target);
        }

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
