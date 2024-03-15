use std::iter;

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;

use crate::{
    common::{
        array_to_bits,
        targets::{
            impl_array_target_wrapper, impl_target_set, Blake2Target, ByteTarget, HalfByteTarget,
            SingleTarget, TargetSet,
        },
    },
    prelude::*,
    ProofWithCircuitData,
};

mod bitmap_parser;
mod branch_header_parser;
mod child_node_array_parser;
mod nibble_parser;

const NODE_DATA_BLOCK_BYTES: usize = 128;
const MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS: usize = 5;
const MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES: usize = 64;

impl_array_target_wrapper!(NodeDataBlockTarget, ByteTarget, NODE_DATA_BLOCK_BYTES);
impl_array_target_wrapper!(
    BranchNodeDataPaddedTarget,
    NodeDataBlockTarget,
    MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS
);

impl_array_target_wrapper!(
    StorageAddressPaddedTarget,
    HalfByteTarget,
    MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES
);

impl StorageAddressPaddedTarget {
    pub fn empty(builder: &mut CircuitBuilder<F, D>) -> Self {
        let zero = builder.zero();
        let mut nibbles = iter::repeat(zero).take(MAX_STORAGE_ADDRESS_LENGTH_IN_NIBBLES);
        let target = Self::parse(&mut nibbles);

        assert_eq!(nibbles.next(), None);

        target
    }
}

impl_target_set! {
    pub struct PartialStorageAddressTarget {
        pub address: StorageAddressPaddedTarget,
        pub current_offset: SingleTarget
    }
}

impl PartialStorageAddressTarget {
    pub fn empty(builder: &mut CircuitBuilder<F, D>) -> Self {
        Self {
            address: StorageAddressPaddedTarget::empty(builder),
            current_offset: builder.zero().into(),
        }
    }
}
