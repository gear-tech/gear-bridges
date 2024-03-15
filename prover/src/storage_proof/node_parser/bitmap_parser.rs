use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;

use super::{NodeDataBlockTarget, PartialStorageAddressTarget};
use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Blake2Target, HalfByteTarget, SingleTarget, TargetSet},
    },
    prelude::*,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct BitmapParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
        pub read_offset: SingleTarget,
        pub claimed_child_node_nibble: HalfByteTarget,
    }
}

impl_target_set! {
    pub struct BitmapParserOutputTarget {
        pub resulting_offset: SingleTarget,
        pub overall_children_amount: SingleTarget,
        pub child_index_in_array: SingleTarget
    }
}

pub fn define(
    input: BitmapParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> BitmapParserOutputTarget {
    todo!()
}
