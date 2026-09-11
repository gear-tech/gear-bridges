//! ### Circuit that's used to parse children nodes from encoded branch node.

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{
            CircuitConfig, CircuitData, CommonCircuitData, VerifierCircuitData,
            VerifierCircuitTarget, VerifierOnlyCircuitData,
        },
        proof::{ProofWithPublicInputs, ProofWithPublicInputsTarget},
    },
    recursion::dummy_circuit::cyclic_base_proof,
};
use plonky2_field::types::Field;

use self::child_node_parser::{ChildNodeParser, ChildNodeParserTarget};
use crate::{
    common::{
        array_to_bits, common_data_for_recursion,
        targets::{
            impl_parsable_target_set, impl_target_set, Blake2Target, ParsableTargetSet, TargetSet,
            VerifierDataTarget,
        },
        ProofWithCircuitData,
    },
    prelude::{consts::BLAKE2_DIGEST_SIZE, *},
    storage_inclusion::storage_trie_proof::node_parser::{
        BranchNodeDataPaddedTarget, MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS, NODE_DATA_BLOCK_BYTES,
    },
};
use std::{
    iter,
    sync::{Arc, OnceLock},
};

mod child_node_parser;

impl_parsable_target_set! {
    /// Public inputs for `ChildNodeArrayParser`.
    pub struct ChildNodeArrayParserTarget {
        /// Encoded node data, padded to a max branch node encoded length.
        pub node_data: BranchNodeDataPaddedTarget,
        /// Offset to read children info from `node_data`.
        pub initial_read_offset: Target,
        /// Should point to the end of a data, as children are located at the end of encoded branch
        /// node.
        pub final_read_offset: Target,
        /// Overall amount of non-empty children nodes.
        pub overall_children_amount: Target,
        /// Index of a child that we claim will be next in our trie traversal. Note that it's not a
        /// nibble, but basically index in the array of nodes that we read from encoded data.
        pub claimed_child_index_in_array: Target,
        /// Hash of a child that we claim will be next in our trie traversal.
        pub claimed_child_hash: Blake2Target,
    }
}

#[derive(Clone)]
pub struct InitialData {
    /// Padded SCALE encoded node data.
    pub node_data: [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],
    /// Offset to read children info from `node_data`.
    pub read_offset: usize,
    /// Index of a child that we claim will be next in our trie traversal. Note that it's not a
    /// nibble, but basically index in the array of nodes that we read from encoded data.
    pub claimed_child_index_in_array: usize,
    /// Hash of a child that we claim will be next in our trie traversal.
    pub claimed_child_hash: [u8; BLAKE2_DIGEST_SIZE],
}

pub struct ChildNodeArrayParser {
    /// Initial data for recursive circuit.
    pub initial_data: InitialData,
    /// Lengths of encoded children nodes.
    pub children_lengths: Vec<usize>,
}

struct FinalProjectionTemplate {
    circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    inner_proof_with_pis: ProofWithPublicInputsTarget<D>,
    inner_verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    inner_common_data: CommonCircuitData<F, D>,
    inner_verifier_only: VerifierOnlyCircuitData<C, D>,
}

impl FinalProjectionTemplate {
    fn cached(
        inner_proof: &ProofWithCircuitData<CyclicRecursionTargetWithVerifierData>,
    ) -> &'static Self {
        // The projection has one fixed inner proof shape; child contents are
        // carried by the per-call inner proof witness.
        static CACHE: OnceLock<FinalProjectionTemplate> = OnceLock::new();
        let template = CACHE.get_or_init(|| Self::build(inner_proof));
        let inner_verifier_data = inner_proof.shared_circuit_data();
        if !Arc::ptr_eq(&template.inner_verifier_data, &inner_verifier_data) {
            let inner_data = inner_verifier_data.as_ref();
            assert_eq!(
                &template.inner_common_data, &inner_data.common,
                "ChildNodeArrayParser final projection received incompatible common data"
            );
            assert_eq!(
                &template.inner_verifier_only, &inner_data.verifier_only,
                "ChildNodeArrayParser final projection received incompatible verifier data"
            );
        }
        template
    }

    fn instantiate(
        &self,
        inner_proof: &ProofWithCircuitData<CyclicRecursionTargetWithVerifierData>,
    ) -> FinalProjectionCircuit<'_> {
        let mut witness = PartialWitness::new();
        witness.set_proof_with_pis_target(&self.inner_proof_with_pis, &inner_proof.proof());
        FinalProjectionCircuit {
            template: self,
            witness,
        }
    }

    fn build(inner_proof: &ProofWithCircuitData<CyclicRecursionTargetWithVerifierData>) -> Self {
        log::debug!("Building child node array parser final projection template...");

        let inner_data = inner_proof.circuit_data();
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let inner_proof_with_pis = builder.add_virtual_proof_with_pis(&inner_data.common);
        let inner_verifier = builder.constant_verifier_data(&inner_data.verifier_only);
        builder.verify_proof::<C>(&inner_proof_with_pis, &inner_verifier, &inner_data.common);
        let inner_target = CyclicRecursionTargetWithVerifierData::parse_exact(
            &mut inner_proof_with_pis.public_inputs.clone().into_iter(),
        );

        ChildNodeArrayParserTarget {
            node_data: inner_target.inner.node_data,
            initial_read_offset: inner_target.inner.initial_read_offset,
            final_read_offset: inner_target.inner.read_offset,
            overall_children_amount: inner_target.inner.overall_children_amount,
            claimed_child_index_in_array: inner_target.inner.claimed_child_index_in_array,
            claimed_child_hash: inner_target.inner.claimed_child_hash,
        }
        .register_as_public_inputs(&mut builder);

        let circuit_data = Arc::new(builder.build::<C>());
        let verifier_data = Arc::new(circuit_data.verifier_data());

        Self {
            circuit_data,
            verifier_data,
            inner_proof_with_pis,
            inner_verifier_data: inner_proof.shared_circuit_data(),
            inner_common_data: inner_data.common.clone(),
            inner_verifier_only: inner_data.verifier_only.clone(),
        }
    }
}

struct FinalProjectionCircuit<'a> {
    template: &'a FinalProjectionTemplate,
    witness: PartialWitness<F>,
}

impl FinalProjectionCircuit<'_> {
    fn prove(self) -> ProofWithCircuitData<ChildNodeArrayParserTarget> {
        ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.template.circuit_data,
            Arc::clone(&self.template.verifier_data),
            self.witness,
        )
    }
}

impl ChildNodeArrayParser {
    pub fn prove(self) -> ProofWithCircuitData<ChildNodeArrayParserTarget> {
        let inner_proof = self.inner_proof();
        FinalProjectionTemplate::cached(&inner_proof)
            .instantiate(&inner_proof)
            .prove()
    }

    fn inner_proof(self) -> ProofWithCircuitData<CyclicRecursionTargetWithVerifierData> {
        log::debug!("Proving child node array parser...");
        let claimed_child_hash = array_to_bits(&self.initial_data.claimed_child_hash)
            .try_into()
            .expect("Correct array length");

        let mut read_offset = self.initial_data.read_offset;
        let mut cyclic_proof: Option<ProofWithCircuitData<CyclicRecursionTargetWithVerifierData>> =
            None;
        for (child_idx, child_length) in self.children_lengths.into_iter().enumerate() {
            let assert_child_hash = child_idx == self.initial_data.claimed_child_index_in_array;

            // NOTE: Constructing inner proofs to have `claimed_child_hash` all equal even
            // if internally `ChildNodeParser` will not check the value of `claimed_child_hash`
            // when `assert_child_hash` is `false`. It's used in the recursive proof construction
            // to conveniently assert that for every proof in chain its `claimed_child_hash` equals
            // to the previous one which gives us authentic `claimed_child_hash` in the latest proof
            // in chain that we can later assert on.
            let inner_circuit = ChildNodeParser {
                node_data: self.initial_data.node_data,
                read_offset,
                assert_child_hash,
                claimed_child_hash,
            };

            let inner_proof = inner_circuit.prove();
            let circuit = CircuitTemplate::cached(&inner_proof).instantiate(&inner_proof);

            cyclic_proof = Some(if let Some(cyclic_proof) = cyclic_proof {
                circuit.prove_recursive(cyclic_proof.proof())
            } else {
                circuit.prove_initial(self.initial_data.clone())
            });

            read_offset += child_length;
        }

        log::debug!("Proven child node array parser");

        cyclic_proof.expect("Expected at least one child node in array")
    }
}

impl_parsable_target_set! {
    struct CyclicRecursionTarget {
        node_data: BranchNodeDataPaddedTarget,
        initial_read_offset: Target,
        read_offset: Target,
        overall_children_amount: Target,
        claimed_child_index_in_array: Target,
        claimed_child_hash: Blake2Target,
    }
}

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

impl_target_set! {
    struct CyclicRecursionTargetWithVerifierData {
        pub inner: CyclicRecursionTarget,
        pub verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>
    }
}

struct CircuitTemplate {
    cyclic_circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<plonky2::plonk::circuit_data::VerifierCircuitData<F, C, D>>,
    common_data: Arc<CommonCircuitData<F, D>>,
    inner_proof_with_pis: ProofWithPublicInputsTarget<D>,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,
    condition: BoolTarget,
    verifier_data_target: VerifierCircuitTarget,
    inner_verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    inner_verifier_only: VerifierOnlyCircuitData<C, D>,
    inner_common_data: CommonCircuitData<F, D>,
}

impl CircuitTemplate {
    fn cached(inner_proof: &ProofWithCircuitData<ChildNodeParserTarget>) -> &'static Self {
        static CACHE: OnceLock<CircuitTemplate> = OnceLock::new();
        let template = CACHE.get_or_init(|| Self::build(inner_proof));
        let inner_verifier_data = inner_proof.shared_circuit_data();
        if !Arc::ptr_eq(&template.inner_verifier_data, &inner_verifier_data) {
            let inner_data = inner_verifier_data.as_ref();
            assert_eq!(
                &template.inner_common_data, &inner_data.common,
                "ChildNodeArrayParser cache received incompatible inner common data"
            );
            assert_eq!(
                &template.inner_verifier_only, &inner_data.verifier_only,
                "ChildNodeArrayParser cache received incompatible inner verifier data"
            );
        }
        template
    }

    fn instantiate<'a>(
        &'a self,
        inner_proof: &ProofWithCircuitData<ChildNodeParserTarget>,
    ) -> Circuit<'a> {
        let mut witness = PartialWitness::new();
        witness.set_verifier_data_target(
            &self.verifier_data_target,
            &self.cyclic_circuit_data.verifier_only,
        );
        witness.set_proof_with_pis_target(&self.inner_proof_with_pis, &inner_proof.proof());
        Circuit {
            template: self,
            witness,
        }
    }

    fn build(inner_proof: &ProofWithCircuitData<ChildNodeParserTarget>) -> Self {
        log::debug!("    Building child node parser recursion template...");

        let inner_data = inner_proof.circuit_data();
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());

        let inner_proof_with_pis = builder.add_virtual_proof_with_pis(&inner_data.common);
        let inner_verifier_data = builder.constant_verifier_data(&inner_data.verifier_only);
        builder.verify_proof::<C>(
            &inner_proof_with_pis,
            &inner_verifier_data,
            &inner_data.common,
        );
        let inner_proof_pis = ChildNodeParserTarget::parse_exact(
            &mut inner_proof_with_pis.public_inputs.clone().into_iter(),
        );

        let mut virtual_targets = iter::repeat(()).map(|_| builder.add_virtual_target());
        let future_inner_cyclic_proof_pis = CyclicRecursionTarget::parse(&mut virtual_targets);
        future_inner_cyclic_proof_pis.register_as_public_inputs(&mut builder);

        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(builder.num_public_inputs(), NUM_GATES_COMMON);

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);
        let mut inner_cyclic_proof_pis = CyclicRecursionTargetWithVerifierData::parse_exact(
            &mut inner_cyclic_proof_with_pis
                .public_inputs
                .clone()
                .into_iter(),
        )
        .inner;

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

        let final_pis = CyclicRecursionTarget {
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
            .expect("Failed to build circuit");

        let cyclic_circuit_data = Arc::new(builder.build::<C>());
        let verifier_data = Arc::new(cyclic_circuit_data.verifier_data());

        log::debug!("    Built child node parser recursion template");

        Self {
            cyclic_circuit_data,
            verifier_data,
            common_data: Arc::new(common_data),
            inner_proof_with_pis,
            inner_cyclic_proof_with_pis,
            condition,
            verifier_data_target,
            inner_verifier_data: inner_proof.shared_circuit_data(),
            inner_verifier_only: inner_data.verifier_only.clone(),
            inner_common_data: inner_data.common.clone(),
        }
    }
}

struct Circuit<'a> {
    template: &'a CircuitTemplate,
    witness: PartialWitness<F>,
}

impl Circuit<'_> {
    fn prove_initial(
        mut self,
        initial_data: InitialData,
    ) -> ProofWithCircuitData<CyclicRecursionTargetWithVerifierData> {
        log::debug!("    Proving child node parser recursion layer(initial)...");

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

        CyclicRecursionTarget::parse_public_inputs_exact(&mut public_inputs.clone());
        let public_inputs = public_inputs.enumerate().collect();

        self.witness.set_bool_target(self.template.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.template.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.template.common_data,
                &self.template.cyclic_circuit_data.verifier_only,
                public_inputs,
            ),
        );

        let result = ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.template.cyclic_circuit_data,
            Arc::clone(&self.template.verifier_data),
            self.witness,
        );

        log::debug!("    Proven child node parser recursion layer(initial)...");

        result
    }

    fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<CyclicRecursionTargetWithVerifierData> {
        log::debug!("    Proving child node parser recursion layer...");
        self.witness.set_bool_target(self.template.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.template.inner_cyclic_proof_with_pis, &composed_proof);

        let result = ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.template.cyclic_circuit_data,
            Arc::clone(&self.template.verifier_data),
            self.witness,
        );

        log::debug!("    Proven child node parser recursion layer");

        result
    }
}

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
            iter::repeat_n(NotClaimed(32), 15)
                .chain(iter::once(Claimed([0xAA; BLAKE2_DIGEST_SIZE])))
                .collect(),
        );
    }

    fn test_case(child_types: Vec<MockChildType>) {
        let children_lengths = child_types.iter().map(|ty| ty.encode().len()).collect();

        let (claimed_idx, claimed_hash) = child_types
            .iter()
            .enumerate()
            .filter_map(|(idx, child)| match child {
                MockChildType::Claimed(hash) => Some((idx, hash)),
                _ => None,
            })
            .next()
            .expect("At least one claimed child");

        let node_data = compose_all_children(&child_types);

        ChildNodeArrayParser {
            initial_data: InitialData {
                node_data,
                read_offset: 0,
                claimed_child_index_in_array: claimed_idx,
                claimed_child_hash: *claimed_hash,
            },
            children_lengths,
        }
        .prove();
    }
}
