use itertools::Itertools;
use plonky2::{
    gates::noop::NoopGate,
    iop::{
        target::BoolTarget,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{
            CircuitConfig, CircuitData, CommonCircuitData, VerifierCircuitData,
            VerifierCircuitTarget,
        },
        proof::{Proof, ProofWithPublicInputs},
    },
};
use serde::{Deserialize, Serialize};
use std::{marker::PhantomData, sync::Arc};

use crate::prelude::*;

#[macro_use]
pub mod targets;
pub mod generic_blake2;
pub mod poseidon_bn128;

use plonky2_field::goldilocks_field::GoldilocksField;
use targets::TargetSet;

use self::poseidon_bn128::config::PoseidonBN128GoldilocksConfig;

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

    pub fn circuit_data(&self) -> &VerifierCircuitData<F, C, D> {
        &self.circuit_data
    }

    pub fn public_inputs(&self) -> Vec<GoldilocksField> {
        self.public_inputs.clone()
    }

    pub fn proof(&self) -> ProofWithPublicInputs<F, C, D> {
        ProofWithPublicInputs {
            proof: self.proof.clone(),
            public_inputs: self.public_inputs.clone(),
        }
    }

    pub fn export(self) -> SerializedDataToVerify {
        let proof_with_public_inputs = ProofWithPublicInputs {
            proof: self.proof,
            public_inputs: self.public_inputs,
        };

        SerializedDataToVerify {
            proof_with_public_inputs: serde_json::to_string(&proof_with_public_inputs).unwrap(),
            common_circuit_data: serde_json::to_string(&self.circuit_data.common).unwrap(),
            verifier_only_circuit_data: serde_json::to_string(&self.circuit_data.verifier_only)
                .unwrap(),
        }
    }

    pub fn export_wrapped(self) -> SerializedDataToVerify {
        let proof_with_public_inputs = ProofWithPublicInputs {
            proof: self.proof,
            public_inputs: self.public_inputs,
        };

        let (proof_with_public_inputs, circuit_data) =
            wrap_bn128(&self.circuit_data, proof_with_public_inputs);

        SerializedDataToVerify {
            proof_with_public_inputs: serde_json::to_string(&proof_with_public_inputs).unwrap(),
            common_circuit_data: serde_json::to_string(&circuit_data.common).unwrap(),
            verifier_only_circuit_data: serde_json::to_string(&circuit_data.verifier_only).unwrap(),
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

#[derive(Clone, Serialize, Deserialize)]
pub struct SerializedDataToVerify {
    pub proof_with_public_inputs: String,
    pub common_circuit_data: String,
    pub verifier_only_circuit_data: String,
}

fn wrap_bn128(
    inner_circuit_data: &VerifierCircuitData<F, C, D>,
    proof_with_public_inputs: ProofWithPublicInputs<F, C, D>,
) -> (
    ProofWithPublicInputs<F, PoseidonBN128GoldilocksConfig, D>,
    CircuitData<F, PoseidonBN128GoldilocksConfig, D>,
) {
    let mut builder: CircuitBuilder<F, D> =
        CircuitBuilder::new(CircuitConfig::standard_recursion_config());

    let proof_with_pis_target = builder.add_virtual_proof_with_pis(&inner_circuit_data.common);
    let verifier_circuit_target = builder.constant_verifier_data(&inner_circuit_data.verifier_only);

    builder.register_public_inputs(&proof_with_pis_target.public_inputs);

    let mut witness = PartialWitness::new();
    witness.set_proof_with_pis_target(&proof_with_pis_target, &proof_with_public_inputs);

    builder.verify_proof::<C>(
        &proof_with_pis_target,
        &verifier_circuit_target,
        &inner_circuit_data.common,
    );

    let circuit_data = builder.build::<PoseidonBN128GoldilocksConfig>();
    let proof = circuit_data.prove(witness).unwrap();

    (proof, circuit_data)
}

pub trait BuilderExt {
    fn recursively_verify_constant_proof<T: TargetSet>(
        &mut self,
        proof: &ProofWithCircuitData<T>,
        witness: &mut PartialWitness<F>,
    ) -> T;

    /// Select if `condition` { `a` } else { `b` }
    fn select_target_set<T: TargetSet>(&mut self, condition: BoolTarget, a: &T, b: &T) -> T;

    fn xor(&mut self, a: BoolTarget, b: BoolTarget) -> BoolTarget;
}

impl BuilderExt for CircuitBuilder<F, D> {
    fn recursively_verify_constant_proof<T: TargetSet>(
        &mut self,
        proof: &ProofWithCircuitData<T>,
        witness: &mut PartialWitness<F>,
    ) -> T {
        let proof_with_pis_target = self.add_virtual_proof_with_pis(&proof.circuit_data.common);
        let verifier_data_target = self.constant_verifier_data(&proof.circuit_data.verifier_only);

        witness.set_proof_with_pis_target(&proof_with_pis_target, &proof.proof());

        self.verify_proof::<C>(
            &proof_with_pis_target,
            &verifier_data_target,
            &proof.circuit_data.common,
        );

        T::parse_exact(&mut proof_with_pis_target.public_inputs.into_iter())
    }

    fn select_target_set<T: TargetSet>(&mut self, condition: BoolTarget, a: &T, b: &T) -> T {
        let mut result = a
            .clone()
            .into_targets_iter()
            .zip_eq(b.clone().into_targets_iter())
            .map(|(a, b)| self.select(condition, a, b));
        T::parse_exact(&mut result)
    }

    // !(!a & !b) & !(a & b)
    fn xor(&mut self, a: BoolTarget, b: BoolTarget) -> BoolTarget {
        let not_a = self.not(a);
        let not_b = self.not(b);

        let c = self.and(not_a, not_b);
        let c = self.not(c);
        let d = self.and(a, b);
        let d = self.not(d);
        self.and(c, d)
    }
}

pub fn common_data_for_recursion(
    config: CircuitConfig,
    public_input_count: usize,
    num_gates: usize,
) -> CommonCircuitData<F, D> {
    let builder = CircuitBuilder::<F, D>::new(config.clone());
    let data = builder.build::<C>();
    let mut builder = CircuitBuilder::<F, D>::new(config.clone());
    let proof = builder.add_virtual_proof_with_pis(&data.common);
    let verifier_data = VerifierCircuitTarget {
        constants_sigmas_cap: builder.add_virtual_cap(data.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };

    builder.verify_proof::<C>(&proof, &verifier_data, &data.common);
    let data = builder.build::<C>();

    let mut builder = CircuitBuilder::<F, D>::new(config);
    let proof = builder.add_virtual_proof_with_pis(&data.common);
    let verifier_data = VerifierCircuitTarget {
        constants_sigmas_cap: builder.add_virtual_cap(data.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };
    builder.verify_proof::<C>(&proof, &verifier_data, &data.common);
    while builder.num_gates() < num_gates {
        builder.add_gate(NoopGate, vec![]);
    }
    let mut data = builder.build::<C>().common;
    data.num_public_inputs = public_input_count;
    data
}

pub fn array_to_bits(data: &[u8]) -> Vec<bool> {
    data.iter().copied().flat_map(byte_to_bits).collect()
}

pub fn byte_to_bits(byte: u8) -> [bool; 8] {
    (0..8)
        .rev()
        .map(move |bit_idx| (byte >> bit_idx) % 2 == 1)
        .collect::<Vec<_>>()
        .try_into()
        .expect("8 bits in byte")
}

pub fn bits_to_byte(bits: [bool; 8]) -> u8 {
    bits.into_iter()
        .rev()
        .enumerate()
        .map(|(no, bit)| (bit as u8) << no)
        .sum()
}

pub fn pad_byte_vec<const L: usize>(mut data: Vec<u8>) -> [u8; L] {
    assert!(data.len() <= L);

    data.append(&mut vec![0; L - data.len()]);
    data.try_into().expect("Correct length of Vec")
}
