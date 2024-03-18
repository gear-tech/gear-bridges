use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, CommonCircuitData},
        proof::ProofWithPublicInputsTarget,
    },
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;

use self::child_node_parser::{ChildNodeParser, ChildNodeParserTarget};

use super::{BranchNodeDataPaddedTarget, NodeDataBlockTarget, PartialStorageAddressTarget};
use crate::{
    common::{
        array_to_bits, common_data_for_recursion,
        targets::{
            impl_target_set, Blake2Target, HalfByteTarget, SingleTarget, TargetSet,
            VerifierDataTarget,
        },
        ConstantRecursiveVerifier,
    },
    prelude::*,
    ProofWithCircuitData,
};

mod child_node_parser;
mod scale_compact_integer_parser;

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

impl_target_set! {
    pub struct ChildNodeArrayParserTarget {
        pub node_data: BranchNodeDataPaddedTarget,
        pub initial_read_offset: SingleTarget,
        pub read_offset: SingleTarget,
        pub overall_children_amount: SingleTarget,
        pub claimed_child_index_in_array: SingleTarget,
        pub claimed_child_hash: Blake2Target,

        pub verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>
    }
}

impl ChildNodeArrayParserTarget {
    fn remove_verifier_data(self) -> ChildNodeArrayParserTargetWithoutCircuitData {
        ChildNodeArrayParserTargetWithoutCircuitData {
            node_data: self.node_data,
            initial_read_offset: self.initial_read_offset,
            read_offset: self.read_offset,
            overall_children_amount: self.overall_children_amount,
            claimed_child_index_in_array: self.claimed_child_index_in_array,
            claimed_child_hash: self.claimed_child_hash,
        }
    }
}

impl_target_set! {
    pub struct ChildNodeArrayParserTargetWithoutCircuitData {
        pub node_data: BranchNodeDataPaddedTarget,
        pub initial_read_offset: SingleTarget,
        pub read_offset: SingleTarget,
        pub overall_children_amount: SingleTarget,
        pub claimed_child_index_in_array: SingleTarget,
        pub claimed_child_hash: Blake2Target,
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
    // TODO: Assert verifier data?
    pub fn build(inner_circuit: ChildNodeParser) -> Circuit {
        let inner_proof = inner_circuit.prove();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut pw = PartialWitness::new();

        let inner_proof_pis = builder.recursively_verify_constant_proof(inner_proof, &mut pw);

        let mut virtual_targets = iter::repeat(()).map(|_| builder.add_virtual_target());
        let future_inner_cyclic_proof_pis =
            ChildNodeArrayParserTargetWithoutCircuitData::parse(&mut virtual_targets);
        future_inner_cyclic_proof_pis.register_as_public_inputs(&mut builder);

        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(
            CircuitConfig::standard_recursion_config(),
            builder.num_public_inputs(),
            1 << 13,
        );

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);
        let inner_cyclic_proof_pis = ChildNodeArrayParserTarget::parse_exact(
            &mut inner_cyclic_proof_with_pis
                .public_inputs
                .clone()
                .into_iter(),
        );
        let mut inner_cyclic_proof_pis = inner_cyclic_proof_pis.remove_verifier_data();

        inner_cyclic_proof_pis.read_offset = builder
            .select(
                condition,
                inner_cyclic_proof_pis.read_offset.to_target(),
                inner_cyclic_proof_pis.initial_read_offset.to_target(),
            )
            .into();

        let zero = builder.zero();
        inner_cyclic_proof_pis.overall_children_amount = builder
            .select(
                condition,
                inner_cyclic_proof_pis.overall_children_amount.to_target(),
                zero,
            )
            .into();

        inner_proof_pis
            .node_data
            .connect(&future_inner_cyclic_proof_pis.node_data, &mut builder);
        inner_proof_pis
            .read_offset
            .connect(&inner_cyclic_proof_pis.read_offset, &mut builder);
        let assert_child_hash = builder.is_equal(
            inner_cyclic_proof_pis
                .claimed_child_index_in_array
                .to_target(),
            inner_cyclic_proof_pis.overall_children_amount.to_target(),
        );
        inner_proof_pis
            .assert_child_hash
            .connect(&assert_child_hash, &mut builder);
        inner_proof_pis.claimed_child_hash.connect(
            &future_inner_cyclic_proof_pis.claimed_child_hash,
            &mut builder,
        );

        let one = builder.one();
        let overall_children_amount = builder
            .add(
                inner_cyclic_proof_pis.overall_children_amount.to_target(),
                one,
            )
            .into();

        let resulting_read_offset = inner_proof_pis.resulting_read_offset;

        let final_pis = ChildNodeArrayParserTargetWithoutCircuitData {
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

        Circuit {
            cyclic_circuit_data,
            common_data,
            condition,
            inner_cyclic_proof_with_pis,
            witness: pw,
        }
    }

    pub fn prove_initial(self) {
        //
    }

    pub fn prove_recursive(self) {
        //
    }
}
