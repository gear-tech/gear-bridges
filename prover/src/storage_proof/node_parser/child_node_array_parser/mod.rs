use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;

use super::{BranchNodeDataPaddedTarget, NodeDataBlockTarget, PartialStorageAddressTarget};
use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Blake2Target, HalfByteTarget, SingleTarget, TargetSet},
    },
    prelude::*,
    ProofWithCircuitData,
};

mod child_node_parser;
mod scale_compact_integer_parser;

impl_target_set! {
    pub struct ChildNodeArrayParserInputTarget {
        pub node_data: BranchNodeDataPaddedTarget,
        // Within the first data block.
        pub read_offset: SingleTarget,
        pub overall_children_amount: SingleTarget,
        pub claimed_child_index_in_array: SingleTarget,
        pub claimed_child_hash: Blake2Target
    }
}

pub fn define(input: ChildNodeArrayParserInputTarget, builder: &mut CircuitBuilder<F, D>) {
    todo!()
}
