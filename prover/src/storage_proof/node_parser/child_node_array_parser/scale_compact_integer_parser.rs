use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;
use plonky2_field::{packed::PackedField, types::Field};

use super::{NodeDataBlockTarget, PartialStorageAddressTarget};
use crate::{
    common::{
        array_to_bits,
        targets::{
            impl_target_set, Blake2Target, ByteTarget, HalfByteTarget, SingleTarget, TargetSet,
        },
        xor_targets,
    },
    prelude::*,
    ProofWithCircuitData,
};

impl_target_set! {
    pub struct ScaleCompactIntegerParserInputTarget {
        pub first_byte: ByteTarget
    }
}

impl_target_set! {
    pub struct ScaleCompactIntegerParserOutputTarget {
        pub decoded: SingleTarget
    }
}

/// Supports only single-byte mode values for now.
pub fn define(
    input: ScaleCompactIntegerParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> ScaleCompactIntegerParserOutputTarget {
    todo!()
}
