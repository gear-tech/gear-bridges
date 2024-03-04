use plonky2::field::goldilocks_field::GoldilocksField;
use plonky2::field::types::Field;
use plonky2::iop::witness::{PartialWitness, WitnessWrite};
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::plonk::circuit_data::{CircuitConfig, VerifierCircuitTarget};
use plonky2::plonk::circuit_data::{CircuitData, VerifierCircuitData};
use plonky2::plonk::config::PoseidonGoldilocksConfig;
use plonky2::plonk::proof::ProofWithPublicInputs;
use plonky2::util::serialization::DefaultGateSerializer;

use super::poseidon_bn128::config::PoseidonBN128GoldilocksConfig;

use crate::prelude::{C, D, F};

pub fn wrap(
    inner_cd: &VerifierCircuitData<F, C, D>,
    proof_with_pis: ProofWithPublicInputs<F, C, D>,
) {
    type F = GoldilocksField;
    type C = PoseidonBN128GoldilocksConfig;
    const D: usize = 2;

    let mut builder: CircuitBuilder<F, D> =
        CircuitBuilder::new(CircuitConfig::standard_recursion_config());

    let proof_with_pis_target = builder.add_virtual_proof_with_pis(&inner_cd.common);
    let circuit_digest = inner_cd.verifier_only.circuit_digest;
    let verifier_circuit_target = VerifierCircuitTarget {
        constants_sigmas_cap: builder.add_virtual_cap(inner_cd.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };

    builder.register_public_inputs(&proof_with_pis_target.public_inputs);

    let mut witness = PartialWitness::new();
    witness.set_proof_with_pis_target(&proof_with_pis_target, &proof_with_pis);
    witness.set_cap_target(
        &verifier_circuit_target.constants_sigmas_cap,
        &inner_cd.verifier_only.constants_sigmas_cap,
    );
    witness.set_hash_target(
        verifier_circuit_target.circuit_digest,
        inner_cd.verifier_only.circuit_digest,
    );

    builder.verify_proof::<PoseidonGoldilocksConfig>(
        &proof_with_pis_target,
        &verifier_circuit_target,
        &inner_cd.common,
    );

    let circuit_data = builder.build::<C>();
    let proof = circuit_data.prove(witness).unwrap();

    let pwpi_ser = serde_json::to_string(&proof).unwrap();
    std::fs::write(
        "../gnark-plonky2-verifier/testdata/own/proof_with_public_inputs.json",
        pwpi_ser,
    )
    .unwrap();

    let common_cd = serde_json::to_string(&circuit_data.common).unwrap();
    std::fs::write(
        "../gnark-plonky2-verifier/testdata/own/common_circuit_data.json",
        common_cd,
    )
    .unwrap();

    let verifier_cd = serde_json::to_string(&circuit_data.verifier_only).unwrap();
    std::fs::write(
        "../gnark-plonky2-verifier/testdata/own/verifier_only_circuit_data.json",
        verifier_cd,
    )
    .unwrap();
}
