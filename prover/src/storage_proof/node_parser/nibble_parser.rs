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
        targets::{impl_target_set, Blake2Target, SingleTarget, TargetSet},
    },
    prelude::*,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct NibbleParserInputTarget {
        pub first_node_data_block: NodeDataBlockTarget,
        pub read_offset: SingleTarget,
        pub nibble_count: SingleTarget,
        pub partial_address: PartialStorageAddressTarget
    }
}

impl_target_set! {
    pub struct NibbleParserOutputTarget {
        pub resulting_offset: SingleTarget,
        pub partial_address: PartialStorageAddressTarget
    }
}

pub fn define(
    input: NibbleParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> NibbleParserOutputTarget {
    todo!()
}
