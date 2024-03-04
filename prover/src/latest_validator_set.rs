use itertools::Itertools;
use plonky2::{
    field::types::Field,
    gates::noop::NoopGate,
    hash::hash_types::MerkleCapTarget,
    iop::{
        target::BoolTarget,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, CommonCircuitData, VerifierCircuitTarget},
        proof::{ProofWithPublicInputs, ProofWithPublicInputsTarget},
    },
    recursion::dummy_circuit::cyclic_base_proof,
};

use crate::{
    common::targets::{
        impl_target_set, Sha256TargetGoldilocks, SingleTarget, TargetSet, VerifierDataTarget,
    },
    next_validator_set::{NextValidatorSet, NextValidatorSetTarget},
    prelude::*,
    ProofWithCircuitData,
};

use self::consts::{GENESIS_AUTHORITY_SET_ID, GENESIS_VALIDATOR_SET_HASH};

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

impl_target_set! {
    pub struct LatestValidatorSetTarget {
        pub genesis_set_id: SingleTarget,
        pub genesis_hash: Sha256TargetGoldilocks,
        pub current_set_id: SingleTarget,
        pub current_hash: Sha256TargetGoldilocks,
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
    verifier_data_target: VerifierCircuitTarget,

    witness: PartialWitness<F>,
}

impl Circuit {
    pub fn prove_genesis(mut self) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        let genesis_data_pis = vec![GENESIS_AUTHORITY_SET_ID]
            .into_iter()
            .chain(GENESIS_VALIDATOR_SET_HASH.into_iter())
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
        self.witness.set_verifier_data_target(
            &self.verifier_data_target,
            &self.cyclic_circuit_data.verifier_only,
        );

        ProofWithCircuitData::from_circuit_data(self.cyclic_circuit_data, self.witness)
    }

    pub fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        self.witness.set_bool_target(self.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.inner_cyclic_proof_with_pis, &composed_proof);
        self.witness.set_verifier_data_target(
            &self.verifier_data_target,
            &self.cyclic_circuit_data.verifier_only,
        );

        ProofWithCircuitData::from_circuit_data(self.cyclic_circuit_data, self.witness)
    }
}

impl LatestValidatorSet {
    pub fn prove_genesis(&self) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        let circuit = self.build_circuit();
        circuit.prove_genesis()
    }

    pub fn prove_recursive(
        &self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        let circuit = self.build_circuit();
        circuit.prove_recursive(composed_proof)
    }

    fn build_circuit(&self) -> Circuit {
        let next_validator_set_proof = self.change_proof.prove();

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let one = builder.one();

        let genesis_authority_set_id = builder.add_virtual_public_input();
        let genesis_authority_set_hash = Sha256TargetGoldilocks::parse(
            &mut std::iter::repeat(()).map(|_| builder.add_virtual_target()),
        );
        genesis_authority_set_hash.register_as_public_inputs(&mut builder);

        // Verify validator set change
        let next_validator_set_proof_target =
            builder.add_virtual_proof_with_pis(&next_validator_set_proof.circuit_data().common);

        let desired_authority_set_change_proof_circuit_digest =
            builder.constant_hash(next_validator_set_proof.circuit_digest());
        let desired_next_validator_set_merkle_cap = MerkleCapTarget(
            next_validator_set_proof
                .circuit_data()
                .verifier_only
                .constants_sigmas_cap
                .0
                .iter()
                .map(|value| builder.constant_hash(*value))
                .collect::<Vec<_>>(),
        );

        let next_validator_set_verifier_target = VerifierCircuitTarget {
            constants_sigmas_cap: desired_next_validator_set_merkle_cap,
            circuit_digest: desired_authority_set_change_proof_circuit_digest,
        };

        let mut witness = PartialWitness::new();

        witness.set_proof_with_pis_target(
            &next_validator_set_proof_target,
            &next_validator_set_proof.proof(),
        );

        builder.verify_proof::<C>(
            &next_validator_set_proof_target,
            &next_validator_set_verifier_target,
            &next_validator_set_proof.circuit_data().common,
        );

        let next_authority_set_public_inputs = NextValidatorSetTarget::parse_exact(
            &mut next_validator_set_proof_target.public_inputs.into_iter(),
        );

        let current_set_id = next_authority_set_public_inputs
            .current_authority_set_id
            .to_target();
        let next_set_id = builder.add(current_set_id, one);

        let current_set_hash = next_authority_set_public_inputs.validator_set_hash;

        builder.register_public_input(next_set_id);
        next_authority_set_public_inputs
            .next_validator_set_hash
            .register_as_public_inputs(&mut builder);

        // Recursion
        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(builder.num_public_inputs());

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);

        let inner_cyclic_targets = LatestValidatorSetTarget::parse_exact(
            &mut inner_cyclic_proof_with_pis.public_inputs.iter().cloned(),
        );

        builder.connect(
            genesis_authority_set_id,
            inner_cyclic_targets.genesis_set_id.to_target(),
        );

        genesis_authority_set_hash.connect(&inner_cyclic_targets.genesis_hash, &mut builder);

        let actual_current_authority_set_id = builder.select(
            condition,
            inner_cyclic_targets.current_set_id.to_target(),
            genesis_authority_set_id,
        );
        builder.connect(actual_current_authority_set_id, current_set_id);

        let actual_current_authority_set_hash = Sha256TargetGoldilocks::parse_exact(
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

        Circuit {
            cyclic_circuit_data,

            common_data,

            condition,
            inner_cyclic_proof_with_pis,
            verifier_data_target,

            witness,
        }
    }
}

fn common_data_for_recursion(public_input_count: usize) -> CommonCircuitData<F, D> {
    let config = CircuitConfig::standard_recursion_config();
    let builder = CircuitBuilder::<F, D>::new(config);
    let data = builder.build::<C>();
    let config = CircuitConfig::standard_recursion_config();
    let mut builder = CircuitBuilder::<F, D>::new(config);
    let proof = builder.add_virtual_proof_with_pis(&data.common);
    let verifier_data = VerifierCircuitTarget {
        constants_sigmas_cap: builder.add_virtual_cap(data.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };

    builder.verify_proof::<C>(&proof, &verifier_data, &data.common);
    let data = builder.build::<C>();

    let config = CircuitConfig::standard_recursion_config();
    let mut builder = CircuitBuilder::<F, D>::new(config);
    let proof = builder.add_virtual_proof_with_pis(&data.common);
    let verifier_data = VerifierCircuitTarget {
        constants_sigmas_cap: builder.add_virtual_cap(data.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };
    builder.verify_proof::<C>(&proof, &verifier_data, &data.common);
    while builder.num_gates() < 1 << 13 {
        builder.add_gate(NoopGate, vec![]);
    }
    let mut data = builder.build::<C>().common;
    data.num_public_inputs = public_input_count;
    data
}
