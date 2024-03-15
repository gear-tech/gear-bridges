use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;

use super::NodeDataBlockTarget;
use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Blake2Target, SingleTarget, TargetSet},
    },
    prelude::*,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct BranchHeaderParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
    }
}

impl_target_set! {
    pub struct BranchHeaderParserOutputTarget {
        pub nibble_count: SingleTarget,
        pub resulting_offset: SingleTarget,
    }
}

pub fn define(
    input: BranchHeaderParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> BranchHeaderParserOutputTarget {
    todo!()
}
