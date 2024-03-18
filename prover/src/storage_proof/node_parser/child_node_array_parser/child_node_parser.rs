use super::super::{
    BranchNodeDataPaddedTarget, MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS, NODE_DATA_BLOCK_BYTES,
};
use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Blake2Target, HalfByteTarget, SingleTarget, TargetSet},
    },
    consts::BLAKE2_DIGEST_SIZE_IN_BITS,
    prelude::*,
    ProofWithCircuitData,
};
use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;
use plonky2_field::types::Field;

impl_target_set! {
    pub struct ChildNodeParserTarget {
        pub node_data: BranchNodeDataPaddedTarget,

        pub read_offset: SingleTarget,
        pub resulting_read_offset: SingleTarget,

        pub assert_child_hash: BoolTarget,
        pub claimed_child_hash: Blake2Target,
    }
}

struct ChildNodeParser {
    pub node_data: [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],

    pub read_offset: usize,

    pub assert_child_hash: bool,
    pub claimed_child_hash: [bool; BLAKE2_DIGEST_SIZE_IN_BITS],
}

impl ChildNodeParser {
    fn prove_initial(self) -> ProofWithCircuitData<ChildNodeParserTarget> {
        todo!()
    }

    fn prove(self) -> ProofWithCircuitData<ChildNodeParserTarget> {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let mut pw = PartialWitness::new();

        let node_data = BranchNodeDataPaddedTarget::add_virtual_unsafe(&mut builder);
        node_data.set_witness(&self.node_data, &mut pw);

        let read_offset = builder.add_virtual_target();
        pw.set_target(read_offset, F::from_canonical_usize(self.read_offset));

        let assert_child_hash = builder.add_virtual_bool_target_unsafe();
        pw.set_bool_target(assert_child_hash, self.assert_child_hash);

        let claimed_child_hash = Blake2Target::add_virtual_unsafe(&mut builder);
        claimed_child_hash.set_witness(&self.claimed_child_hash, &mut pw);

        // TODO

        ProofWithCircuitData::from_builder(builder, pw)
    }
}
