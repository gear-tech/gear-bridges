use crate::prelude::*;
use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, VerifierCircuitTarget},
        proof::ProofWithPublicInputs,
    },
};

pub struct ProofWithCircuitData {
    pub(crate) proof: ProofWithPublicInputs<F, C, D>,
    pub(crate) circuit_data: CircuitData<F, C, D>,
}

pub struct ProofCompositionTargets {
    pub first_proof_public_input_targets: Vec<Target>,
    pub second_proof_public_input_targets: Vec<Target>,
}

impl ProofWithCircuitData {
    pub fn compose<O>(
        first: &ProofWithCircuitData,
        second: &ProofWithCircuitData,
        operation_with_targets: O,
    ) -> ProofWithCircuitData
    where
        O: Fn(&mut CircuitBuilder<F, D>, ProofCompositionTargets),
    {
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
        pw.set_proof_with_pis_target(&proof_with_pis_target_1, &first.proof);
        pw.set_proof_with_pis_target(&proof_with_pis_target_2, &second.proof);
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

        operation_with_targets(
            &mut builder,
            ProofCompositionTargets {
                first_proof_public_input_targets: proof_with_pis_target_1.public_inputs.clone(),
                second_proof_public_input_targets: proof_with_pis_target_2.public_inputs.clone(),
            },
        );

        let circuit_data = builder.build::<C>();
        let proof = circuit_data.prove(pw).unwrap();

        ProofWithCircuitData {
            proof,
            circuit_data,
        }
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
