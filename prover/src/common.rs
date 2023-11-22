use crate::prelude::*;
use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, VerifierCircuitTarget},
        proof::{Proof, ProofWithPublicInputs},
    },
};

pub struct ProofWithCircuitData {
    proof: Proof<F, C, D>,
    public_inputs: Vec<F>,
    circuit_data: CircuitData<F, C, D>,
}

impl ProofWithCircuitData {
    pub fn from_builder(
        builder: CircuitBuilder<F, D>,
        witness: PartialWitness<F>,
    ) -> ProofWithCircuitData {
        let circuit_data = builder.build::<C>();
        let ProofWithPublicInputs {
            proof,
            public_inputs,
        } = circuit_data.prove(witness).unwrap();
        ProofWithCircuitData {
            proof,
            public_inputs,
            circuit_data,
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
}

pub struct ProofCompositionTargets {
    pub first_proof_public_input_targets: Vec<Target>,
    pub second_proof_public_input_targets: Vec<Target>,
}

pub struct ProofCompositionBuilder {
    circuit_builder: CircuitBuilder<F, D>,
    witness: PartialWitness<F>,
    first_public_inputs: Vec<Target>,
    second_public_inputs: Vec<Target>,
}

impl ProofCompositionBuilder {
    pub fn new(
        first: ProofWithCircuitData,
        second: ProofWithCircuitData,
    ) -> ProofCompositionBuilder {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_recursion_config());
        let proof_with_pis_target_1 =
            builder.add_virtual_proof_with_pis(&first.circuit_data.common);
        let proof_with_pis_target_2 =
            builder.add_virtual_proof_with_pis(&second.circuit_data.common);

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

        ProofCompositionBuilder {
            circuit_builder: builder,
            witness: pw,
            first_public_inputs: proof_with_pis_target_1.public_inputs,
            second_public_inputs: proof_with_pis_target_2.public_inputs,
        }
    }

    pub fn operation_with_targets<O>(mut self, op: O) -> ProofCompositionBuilder
    where
        O: Fn(&mut CircuitBuilder<F, D>, ProofCompositionTargets),
    {
        op(
            &mut self.circuit_builder,
            ProofCompositionTargets {
                first_proof_public_input_targets: self.first_public_inputs.clone(),
                second_proof_public_input_targets: self.second_public_inputs.clone(),
            },
        );

        self
    }

    pub fn build(self) -> ProofWithCircuitData {
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
