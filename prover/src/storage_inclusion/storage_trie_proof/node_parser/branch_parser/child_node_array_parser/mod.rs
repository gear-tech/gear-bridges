use plonky2::{
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
use plonky2_field::types::Field;

use self::child_node_parser::ChildNodeParser;
use crate::{
    common::{
        array_to_bits, common_data_for_recursion,
        targets::{
            impl_parsable_target_set, impl_target_set, Blake2Target, ParsableTargetSet, TargetSet,
            VerifierDataTarget,
        },
        BuilderExt,
    },
    prelude::{consts::BLAKE2_DIGEST_SIZE, *},
    storage_inclusion::storage_trie_proof::node_parser::{
        BranchNodeDataPaddedTarget, MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS, NODE_DATA_BLOCK_BYTES,
    },
    ProofWithCircuitData,
};
use std::iter;

mod child_node_parser;
mod scale_compact_integer_parser;

// TODO: Split into 2 files.

impl_parsable_target_set! {
    pub struct ChildNodeArrayParserTarget {
        pub node_data: BranchNodeDataPaddedTarget,
        pub initial_read_offset: Target,
        pub final_read_offset: Target,
        pub overall_children_amount: Target,
        pub claimed_child_index_in_array: Target,
        pub claimed_child_hash: Blake2Target,
    }
}

#[derive(Clone)]
pub struct InitialData {
    pub node_data: [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],
    pub read_offset: usize,
    pub claimed_child_index_in_array: usize,
    pub claimed_child_hash: [u8; BLAKE2_DIGEST_SIZE],
}

pub struct ChildNodeArrayParser {
    pub initial_data: InitialData,
    pub children_lengths: Vec<usize>,
}

impl ChildNodeArrayParser {
    pub fn prove(self) -> ProofWithCircuitData<ChildNodeArrayParserTarget> {
        let inner_proof = self.inner_proof();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let inner_proof_pis = builder.recursively_verify_constant_proof(inner_proof, &mut witness);

        ChildNodeArrayParserTarget {
            node_data: inner_proof_pis.node_data,
            initial_read_offset: inner_proof_pis.initial_read_offset,
            final_read_offset: inner_proof_pis.read_offset,
            overall_children_amount: inner_proof_pis.overall_children_amount,
            claimed_child_index_in_array: inner_proof_pis.claimed_child_index_in_array,
            claimed_child_hash: inner_proof_pis.claimed_child_hash,
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::from_builder(builder, witness)
    }

    fn inner_proof(self) -> ProofWithCircuitData<CyclicRecursionTarget> {
        log::info!("Proving child node array parser...");
        let claimed_child_hash = array_to_bits(&self.initial_data.claimed_child_hash)
            .try_into()
            .unwrap();

        let mut read_offset = self.initial_data.read_offset;
        let mut cyclic_proof: Option<ProofWithCircuitData<CyclicRecursionTarget>> = None;
        for (child_idx, child_length) in self.children_lengths.into_iter().enumerate() {
            let assert_child_hash = child_idx == self.initial_data.claimed_child_index_in_array;

            let inner_circuit = ChildNodeParser {
                node_data: self.initial_data.node_data,
                read_offset,
                assert_child_hash,
                claimed_child_hash,
            };

            let circuit = Circuit::build(inner_circuit);

            cyclic_proof = Some(if let Some(cyclic_proof) = cyclic_proof {
                circuit.prove_recursive(cyclic_proof.proof())
            } else {
                circuit.prove_initial(self.initial_data.clone())
            });

            read_offset += child_length;
        }

        log::info!("Proven child node array parser");

        cyclic_proof.unwrap()
    }
}

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

impl_target_set! {
    struct CyclicRecursionTarget {
        pub node_data: BranchNodeDataPaddedTarget,
        pub initial_read_offset: Target,
        pub read_offset: Target,
        pub overall_children_amount: Target,
        pub claimed_child_index_in_array: Target,
        pub claimed_child_hash: Blake2Target,

        pub verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>
    }
}

impl CyclicRecursionTarget {
    fn remove_verifier_data(self) -> CyclicRecursionTargetWithoutCircuitData {
        CyclicRecursionTargetWithoutCircuitData {
            node_data: self.node_data,
            initial_read_offset: self.initial_read_offset,
            read_offset: self.read_offset,
            overall_children_amount: self.overall_children_amount,
            claimed_child_index_in_array: self.claimed_child_index_in_array,
            claimed_child_hash: self.claimed_child_hash,
        }
    }
}

// TODO: Remove
impl_parsable_target_set! {
    struct CyclicRecursionTargetWithoutCircuitData {
        node_data: BranchNodeDataPaddedTarget,
        initial_read_offset: Target,
        read_offset: Target,
        overall_children_amount: Target,
        claimed_child_index_in_array: Target,
        claimed_child_hash: Blake2Target,
    }
}

struct Circuit {
    cyclic_circuit_data: CircuitData<F, C, D>,

    common_data: CommonCircuitData<F, D>,

    condition: BoolTarget,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,

    witness: PartialWitness<F>,
}

impl Circuit {
    fn prove_initial(
        mut self,
        initial_data: InitialData,
    ) -> ProofWithCircuitData<CyclicRecursionTarget> {
        log::info!("    Proving child node parser recursion layer(initial)...");

        let public_inputs = initial_data
            .node_data
            .into_iter()
            .flatten()
            .map(|byte| byte as usize)
            .chain(iter::once(initial_data.read_offset))
            .chain(iter::once(0))
            .chain(iter::once(0))
            .chain(iter::once(initial_data.claimed_child_index_in_array))
            .chain(
                array_to_bits(&initial_data.claimed_child_hash)
                    .into_iter()
                    .map(|bit| bit as usize),
            )
            .map(F::from_canonical_usize);

        // Length check.
        CyclicRecursionTargetWithoutCircuitData::parse_public_inputs_exact(
            &mut public_inputs.clone(),
        );

        let public_inputs = public_inputs.enumerate().collect();

        self.witness.set_bool_target(self.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.common_data,
                &self.cyclic_circuit_data.verifier_only,
                public_inputs,
            ),
        );

        let result =
            ProofWithCircuitData::from_circuit_data(self.cyclic_circuit_data, self.witness);

        log::info!("    Proven child node parser recursion layer(initial)...");

        result
    }

    fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<CyclicRecursionTarget> {
        log::info!("    Proving child node parser recursion layer...");
        self.witness.set_bool_target(self.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.inner_cyclic_proof_with_pis, &composed_proof);

        let result =
            ProofWithCircuitData::from_circuit_data(self.cyclic_circuit_data, self.witness);

        log::info!("    Proven child node parser recursion layer");

        result
    }

    fn build(inner_circuit: ChildNodeParser) -> Circuit {
        log::info!("    Proving child node correctness...");

        let inner_proof = inner_circuit.prove();

        log::info!("    Proven child node correctness");

        log::info!("    Building child node parser recursion layer...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut pw = PartialWitness::new();

        let inner_proof_pis = builder.recursively_verify_constant_proof(inner_proof, &mut pw);

        let mut virtual_targets = iter::repeat(()).map(|_| builder.add_virtual_target());
        let future_inner_cyclic_proof_pis =
            CyclicRecursionTargetWithoutCircuitData::parse(&mut virtual_targets);
        future_inner_cyclic_proof_pis.register_as_public_inputs(&mut builder);

        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(
            CircuitConfig::standard_recursion_config(),
            builder.num_public_inputs(),
            1 << 13,
        );

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);
        let inner_cyclic_proof_pis = CyclicRecursionTarget::parse_exact(
            &mut inner_cyclic_proof_with_pis
                .public_inputs
                .clone()
                .into_iter(),
        );
        let mut inner_cyclic_proof_pis = inner_cyclic_proof_pis.remove_verifier_data();

        inner_cyclic_proof_pis.read_offset = builder.select(
            condition,
            inner_cyclic_proof_pis.read_offset,
            inner_cyclic_proof_pis.initial_read_offset,
        );

        let zero = builder.zero();
        inner_cyclic_proof_pis.overall_children_amount = builder.select(
            condition,
            inner_cyclic_proof_pis.overall_children_amount,
            zero,
        );

        inner_proof_pis
            .node_data
            .connect(&future_inner_cyclic_proof_pis.node_data, &mut builder);
        inner_proof_pis
            .read_offset
            .connect(&inner_cyclic_proof_pis.read_offset, &mut builder);
        let assert_child_hash = builder.is_equal(
            inner_cyclic_proof_pis.claimed_child_index_in_array,
            inner_cyclic_proof_pis.overall_children_amount,
        );
        inner_proof_pis
            .assert_child_hash
            .connect(&assert_child_hash, &mut builder);
        inner_proof_pis.claimed_child_hash.connect(
            &future_inner_cyclic_proof_pis.claimed_child_hash,
            &mut builder,
        );

        let overall_children_amount =
            builder.add_const(inner_cyclic_proof_pis.overall_children_amount, F::ONE);

        let resulting_read_offset = inner_proof_pis.resulting_read_offset;

        let final_pis = CyclicRecursionTargetWithoutCircuitData {
            node_data: inner_cyclic_proof_pis.node_data,
            initial_read_offset: inner_cyclic_proof_pis.initial_read_offset,
            read_offset: resulting_read_offset,
            overall_children_amount,
            claimed_child_index_in_array: inner_cyclic_proof_pis.claimed_child_index_in_array,
            claimed_child_hash: inner_cyclic_proof_pis.claimed_child_hash,
        };

        future_inner_cyclic_proof_pis.connect(&final_pis, &mut builder);

        builder
            .conditionally_verify_cyclic_proof_or_dummy::<C>(
                condition,
                &inner_cyclic_proof_with_pis,
                &common_data,
            )
            .unwrap();

        let cyclic_circuit_data = builder.build::<C>();

        pw.set_verifier_data_target(&verifier_data_target, &cyclic_circuit_data.verifier_only);

        log::info!("    Built child node parser recursion layer");

        Circuit {
            cyclic_circuit_data,
            common_data,
            condition,
            inner_cyclic_proof_with_pis,
            witness: pw,
        }
    }
}

// TODO: Rewrite to use ChildNodeArrayParser.
#[cfg(test)]
mod tests {
    use super::{child_node_parser::tests_common::*, *};

    #[test]
    fn test_child_node_array_parser_recursion_initial() {
        use MockChildType::*;
        test_case(vec![Claimed([0; BLAKE2_DIGEST_SIZE])]);
    }

    #[test]
    fn test_child_node_array_parser_recursion_two_nodes() {
        use MockChildType::*;
        test_case(vec![Claimed([1; BLAKE2_DIGEST_SIZE]), NotClaimed(32)]);
        test_case(vec![Claimed([1; BLAKE2_DIGEST_SIZE]), NotClaimed(20)]);
        test_case(vec![NotClaimed(20), Claimed([1; BLAKE2_DIGEST_SIZE])]);
    }

    #[test]
    fn test_child_node_array_parser_recursion_max_nodes() {
        use MockChildType::*;

        test_case(
            iter::repeat(NotClaimed(32))
                .take(15)
                .chain(iter::once(Claimed([0xAA; BLAKE2_DIGEST_SIZE])))
                .collect(),
        );
    }

    fn test_case(child_types: Vec<MockChildType>) {
        let (claimed_idx, claimed_hash) = child_types
            .iter()
            .enumerate()
            .filter_map(|(idx, child)| match child {
                MockChildType::Claimed(hash) => Some((idx, hash)),
                _ => None,
            })
            .next()
            .unwrap();

        let claimed_hash_bits = array_to_bits(claimed_hash).try_into().unwrap();
        let node_data = compose_all_children(&child_types);

        let mut read_offset = 0;
        let mut cyclic_proof = None;
        for child_type in &child_types {
            let assert_child_hash = matches!(child_type, &MockChildType::Claimed(_));

            let inner_circuit = ChildNodeParser {
                node_data: node_data.clone(),
                read_offset,
                assert_child_hash,
                claimed_child_hash: claimed_hash_bits,
            };

            let circuit = Circuit::build(inner_circuit);

            cyclic_proof = Some(if let Some(cyclic_proof) = cyclic_proof {
                circuit.prove_recursive(cyclic_proof).proof()
            } else {
                circuit
                    .prove_initial(InitialData {
                        node_data: node_data.clone(),
                        read_offset: 0,
                        claimed_child_index_in_array: claimed_idx,
                        claimed_child_hash: claimed_hash.clone(),
                    })
                    .proof()
            });

            read_offset += child_type.encode().len();
        }
    }
}
