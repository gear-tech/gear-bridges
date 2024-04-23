use itertools::Itertools;
use plonky2::{
    field::types::Field,
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, CommonCircuitData},
        proof::{ProofWithPublicInputs, ProofWithPublicInputsTarget},
    },
    recursion::dummy_circuit::cyclic_base_proof,
};

use crate::{
    common::{
        common_data_for_recursion,
        targets::{impl_target_set, Blake2TargetGoldilocks, TargetSet, VerifierDataTarget},
        BuilderExt, ProofWithCircuitData,
    },
    prelude::*,
    proving::GenesisConfig,
};

pub mod next_validator_set;

use next_validator_set::NextValidatorSet;

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

impl_target_set! {
    pub struct LatestValidatorSetTarget {
        pub genesis_set_id: Target,
        pub genesis_hash: Blake2TargetGoldilocks,
        pub current_set_id: Target,
        pub current_hash: Blake2TargetGoldilocks,

        pub verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>,
    }
}

pub struct LatestValidatorSet {
    pub change_proof: NextValidatorSet,
}

struct Circuit {
    cyclic_circuit_data: CircuitData<F, C, D>,

    common_data: CommonCircuitData<F, D>,

    condition: BoolTarget,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,

    witness: PartialWitness<F>,
}

impl Circuit {
    pub fn prove_genesis(
        mut self,
        config: GenesisConfig,
    ) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        let genesis_data_pis = vec![config.validator_set_id]
            .into_iter()
            .chain(config.validator_set_hash)
            .map(F::from_noncanonical_u64)
            .enumerate()
            .collect();

        self.witness.set_bool_target(self.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.common_data,
                &self.cyclic_circuit_data.verifier_only,
                genesis_data_pis,
            ),
        );

        ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness)
    }

    pub fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        self.witness.set_bool_target(self.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.inner_cyclic_proof_with_pis, &composed_proof);

        ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness)
    }
}

impl LatestValidatorSet {
    pub fn prove_genesis(
        self,
        config: GenesisConfig,
    ) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        let circuit = self.build_circuit();
        circuit.prove_genesis(config)
    }

    pub fn prove_recursive(
        self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        let circuit = self.build_circuit();
        circuit.prove_recursive(composed_proof)
    }

    fn build_circuit(self) -> Circuit {
        let next_validator_set_proof = self.change_proof.prove();

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let one = builder.one();

        let genesis_authority_set_id = builder.add_virtual_public_input();
        let genesis_authority_set_hash = Blake2TargetGoldilocks::parse(
            &mut std::iter::repeat(()).map(|_| builder.add_virtual_target()),
        );
        genesis_authority_set_hash.register_as_public_inputs(&mut builder);

        // Verify validator set change
        let mut witness = PartialWitness::new();

        let next_authority_set_public_inputs =
            builder.recursively_verify_constant_proof(&next_validator_set_proof, &mut witness);

        let current_set_id = next_authority_set_public_inputs.current_authority_set_id;
        let next_set_id = builder.add(current_set_id, one);

        let current_set_hash = next_authority_set_public_inputs.current_validator_set_hash;

        builder.register_public_input(next_set_id);
        next_authority_set_public_inputs
            .next_validator_set
            .register_as_public_inputs(&mut builder);

        // Recursion
        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(
            CircuitConfig::standard_recursion_config(),
            builder.num_public_inputs(),
            1 << 13,
        );

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);

        let inner_cyclic_targets = LatestValidatorSetTarget::parse_exact(
            &mut inner_cyclic_proof_with_pis.public_inputs.iter().cloned(),
        );

        builder.connect(
            genesis_authority_set_id,
            inner_cyclic_targets.genesis_set_id,
        );

        genesis_authority_set_hash.connect(&inner_cyclic_targets.genesis_hash, &mut builder);

        let actual_current_authority_set_id = builder.select(
            condition,
            inner_cyclic_targets.current_set_id,
            genesis_authority_set_id,
        );
        builder.connect(actual_current_authority_set_id, current_set_id);

        let actual_current_authority_set_hash = Blake2TargetGoldilocks::parse_exact(
            &mut inner_cyclic_targets
                .current_hash
                .into_targets_iter()
                .zip_eq(genesis_authority_set_hash.into_targets_iter())
                .map(|(inner, outer)| builder.select(condition, inner, outer)),
        );
        actual_current_authority_set_hash.connect(&current_set_hash, &mut builder);

        builder
            .conditionally_verify_cyclic_proof_or_dummy::<C>(
                condition,
                &inner_cyclic_proof_with_pis,
                &common_data,
            )
            .unwrap();

        let cyclic_circuit_data = builder.build::<C>();

        witness.set_verifier_data_target(&verifier_data_target, &cyclic_circuit_data.verifier_only);

        Circuit {
            cyclic_circuit_data,

            common_data,

            condition,
            inner_cyclic_proof_with_pis,

            witness,
        }
    }
}
