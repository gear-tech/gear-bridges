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
    common::targets::{impl_target_set, ArrayTarget, SingleTarget, TargetSet},
    next_validator_set::{NextValidatorSet, NextValidatorSetTarget},
    prelude::{consts::SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS, *},
    ProofWithCircuitData,
};

#[derive(Clone)]
pub struct ValidatorSetGenesis {
    pub validator_set_hash: [u64; SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS],
    pub authority_set_id: u64,
}

// circuit digest + merkle caps
const VERIFIER_DATA_LEN: usize = 4 + 4 * 16;

impl_target_set! {
    pub struct LatestValidatorSetTarget {
        genesis_set_id: SingleTarget,
        current_set_id: SingleTarget,
        chain_length: SingleTarget,
        verifier_data: ArrayTarget<SingleTarget, VERIFIER_DATA_LEN>,
    }
}

pub struct LatestValidatorSet {
    pub genesis_data: ValidatorSetGenesis,
    pub change_proof: NextValidatorSet,
}

pub struct CircuitDataWithTargets {
    cyclic_circuit_data: CircuitData<F, C, D>,

    common_data: CommonCircuitData<F, D>,

    condition: BoolTarget,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,
    verifier_data_target: VerifierCircuitTarget,

    witness: PartialWitness<F>,
}

impl CircuitDataWithTargets {
    pub fn prove_initial(
        mut self,
        genesis_set_id: u64,
    ) -> ProofWithCircuitData<LatestValidatorSetTarget> {
        let initial_hash_pis = vec![F::from_canonical_u64(genesis_set_id)]
            .into_iter()
            .enumerate()
            .collect();

        self.witness.set_bool_target(self.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.common_data,
                &self.cyclic_circuit_data.verifier_only,
                initial_hash_pis,
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

// TODO: verify that verifier data is correct in the proof that will be built on top of it.
impl LatestValidatorSet {
    pub fn build_circuit(&self) -> CircuitDataWithTargets {
        let next_validator_set_proof = self.change_proof.prove();

        println!(
            "NVS VD: {:?} {:?}",
            next_validator_set_proof
                .circuit_data()
                .verifier_only
                .circuit_digest
                .elements,
            next_validator_set_proof
                .circuit_data()
                .verifier_only
                .constants_sigmas_cap
                .0
                .iter()
                .map(|h| h.elements)
                .flatten()
                .collect::<Vec<_>>()
        );

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let one = builder.one();

        let genesis_authority_set_id = builder.add_virtual_target();
        builder.register_public_input(genesis_authority_set_id);

        // Verify authority set change
        let next_validator_set_proof_target = builder
            .add_virtual_proof_with_pis::<C>(&next_validator_set_proof.circuit_data().common);

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

        let mut next_authority_set_public_inputs_iter =
            next_validator_set_proof_target.public_inputs.into_iter();
        let next_authority_set_public_inputs =
            NextValidatorSetTarget::parse(&mut next_authority_set_public_inputs_iter);
        assert_eq!(next_authority_set_public_inputs_iter.next(), None);

        let current_set_id = next_authority_set_public_inputs
            .current_authority_set_id
            .to_target();
        let next_set_id = builder.add(current_set_id, one);

        // -------------------------------
        builder.register_public_input(next_set_id);
        // TODO: can be removed?
        let counter = builder.add_virtual_public_input();

        let mut common_data = common_data_for_recursion();
        let verifier_data_target = builder.add_verifier_data_public_inputs();
        common_data.num_public_inputs = builder.num_public_inputs();

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis::<C>(&common_data);
        let inner_cyclic_pis = &inner_cyclic_proof_with_pis.public_inputs;
        let inner_cyclic_genesis_set_id = inner_cyclic_pis[0];
        let inner_cyclic_current_set_id = inner_cyclic_pis[1];
        let inner_cyclic_counter = inner_cyclic_pis[2];

        builder.connect(genesis_authority_set_id, inner_cyclic_genesis_set_id);

        let actual_current_authority_set_id = builder.select(
            condition,
            inner_cyclic_current_set_id,
            genesis_authority_set_id,
        );
        builder.connect(actual_current_authority_set_id, current_set_id);

        let new_counter = builder.mul_add(condition.target, inner_cyclic_counter, one);
        builder.connect(counter, new_counter);

        builder
            .conditionally_verify_cyclic_proof_or_dummy::<C>(
                condition,
                &inner_cyclic_proof_with_pis,
                &common_data,
            )
            .unwrap();

        let cyclic_circuit_data = builder.build::<C>();

        CircuitDataWithTargets {
            cyclic_circuit_data,

            common_data,

            condition,
            inner_cyclic_proof_with_pis,
            verifier_data_target,

            witness,
        }
    }
}

fn common_data_for_recursion() -> CommonCircuitData<F, D> {
    let config = CircuitConfig::standard_recursion_config();
    let builder = CircuitBuilder::<F, D>::new(config);
    let data = builder.build::<C>();
    let config = CircuitConfig::standard_recursion_config();
    let mut builder = CircuitBuilder::<F, D>::new(config);
    let proof = builder.add_virtual_proof_with_pis::<C>(&data.common);
    let verifier_data = VerifierCircuitTarget {
        constants_sigmas_cap: builder.add_virtual_cap(data.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };

    builder.verify_proof::<C>(&proof, &verifier_data, &data.common);
    let data = builder.build::<C>();

    let config = CircuitConfig::standard_recursion_config();
    let mut builder = CircuitBuilder::<F, D>::new(config);
    let proof = builder.add_virtual_proof_with_pis::<C>(&data.common);
    let verifier_data = VerifierCircuitTarget {
        constants_sigmas_cap: builder.add_virtual_cap(data.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };
    builder.verify_proof::<C>(&proof, &verifier_data, &data.common);
    while builder.num_gates() < 1 << 13 {
        builder.add_gate(NoopGate, vec![]);
    }
    builder.build::<C>().common
}
