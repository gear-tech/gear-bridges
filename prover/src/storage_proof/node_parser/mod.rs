use super::storage_address::{PartialStorageAddressTarget, StorageAddressPaddedTarget};
use crate::{
    common::{
        pad_byte_vec,
        targets::{
            impl_array_target_wrapper, impl_target_set, ArrayTarget, ByteTarget, HalfByteTarget,
            ParsableTargetSet, SingleTarget, TargetSet,
        },
    },
    prelude::*,
};
use itertools::Itertools;
use plonky2::{
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::circuit_builder::CircuitBuilder,
};
use plonky2_field::types::{Field, PrimeField64};
use plonky2_u32::gadgets::multiple_comparison::list_le_circuit;

mod branch_parser;
mod header_parser;
mod leaf_parser;
mod nibble_parser;

const NODE_DATA_BLOCK_BYTES: usize = 128;
const MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS: usize = 5;

impl_array_target_wrapper!(NodeDataBlockTarget, ByteTarget, NODE_DATA_BLOCK_BYTES);

impl ParsableTargetSet for NodeDataBlockTarget {
    type PublicInputsData = [u8; NODE_DATA_BLOCK_BYTES];

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs
            .take(NODE_DATA_BLOCK_BYTES)
            .map(|pi| pi.to_canonical_u64() as u8)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}

impl NodeDataBlockTarget {
    pub fn constant(
        data: &[u8; NODE_DATA_BLOCK_BYTES],
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let mut data_targets = data
            .iter()
            .map(|byte| builder.constant(F::from_canonical_u8(*byte)));
        Self::parse_exact(&mut data_targets)
    }

    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> Self {
        let mut targets = (0..NODE_DATA_BLOCK_BYTES).map(|_| builder.add_virtual_target());
        Self::parse_exact(&mut targets)
    }

    pub fn add_virtual_safe(builder: &mut CircuitBuilder<F, D>) -> Self {
        let mut targets = (0..NODE_DATA_BLOCK_BYTES).map(|_| {
            let target = builder.add_virtual_target();
            ByteTarget::from_target_safe(target, builder).to_target()
        });
        Self::parse_exact(&mut targets)
    }

    pub fn set_witness(&self, data: &[u8; NODE_DATA_BLOCK_BYTES], witness: &mut PartialWitness<F>) {
        self.0
             .0
            .iter()
            .zip_eq(data.iter())
            .for_each(|(target, value)| {
                witness.set_target(target.to_target(), F::from_canonical_u8(*value))
            });
    }
}

type LeafNodeDataPaddedTarget = NodeDataBlockTarget;

impl_array_target_wrapper!(
    BranchNodeDataPaddedTarget,
    NodeDataBlockTarget,
    MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS
);

impl ParsableTargetSet for BranchNodeDataPaddedTarget {
    type PublicInputsData = [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS];

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        (0..MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS)
            .map(|_| NodeDataBlockTarget::parse_public_inputs(public_inputs))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}

impl BranchNodeDataPaddedTarget {
    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> Self {
        Self::add_virtual(builder, NodeDataBlockTarget::add_virtual_unsafe)
    }

    pub fn add_virtual_safe(builder: &mut CircuitBuilder<F, D>) -> Self {
        Self::add_virtual(builder, NodeDataBlockTarget::add_virtual_safe)
    }

    fn add_virtual(
        builder: &mut CircuitBuilder<F, D>,
        create_block: impl Fn(&mut CircuitBuilder<F, D>) -> NodeDataBlockTarget,
    ) -> Self {
        let targets = (0..MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS)
            .map(|_| create_block(builder))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        Self(ArrayTarget(targets))
    }

    pub fn set_witness(
        &self,
        data: &[[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],
        witness: &mut PartialWitness<F>,
    ) {
        self.0
             .0
            .iter()
            .zip_eq(data.into_iter())
            .for_each(|(target, data)| target.set_witness(data, witness));
    }

    pub fn random_read_array<const L: usize>(
        &self,
        at: SingleTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> ArrayTarget<ByteTarget, L> {
        let targets = (0..L)
            .map(|offset| {
                let offset = builder.constant(F::from_canonical_usize(offset));
                let read_at = builder.add(at.to_target(), offset);
                self.random_read(read_at.into(), builder)
            })
            .collect::<Vec<_>>();

        ArrayTarget(targets.try_into().unwrap())
    }

    pub fn random_read(&self, at: SingleTarget, builder: &mut CircuitBuilder<F, D>) -> ByteTarget {
        let block_size = builder.constant(F::from_canonical_usize(NODE_DATA_BLOCK_BYTES));
        let max_data_size = builder.constant(F::from_canonical_usize(
            NODE_DATA_BLOCK_BYTES * MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS,
        ));

        let shifted_block_size = builder.add(block_size, max_data_size);
        let mut current_offset = at.to_target();
        let mut block_already_selected = builder._false();
        let mut final_data = builder.zero();
        for block in &self.0 .0 {
            // Check that `current_offset` < `block_size`. This check is performed as:
            // `current_offset` + `max_data_size` + 1 <= `block_size` + `max_data_size`
            // to avoid negative numbers.
            let one = builder.one();
            let shifted_current_offset = builder.add_many(vec![current_offset, max_data_size, one]);
            let current_offset_eligible = list_le_circuit(
                builder,
                vec![shifted_current_offset],
                vec![shifted_block_size],
                32,
            );

            let block_not_yet_selected = builder.not(block_already_selected);
            let read_from_current_block =
                builder.and(current_offset_eligible, block_not_yet_selected);

            block_already_selected = builder.or(read_from_current_block, block_already_selected);

            let zero = builder.zero();

            // Returns `if b { x } else { y }`.
            // If we don't select from current block then we don't care about actual data that's read.
            let read_from = builder.select(read_from_current_block, current_offset, zero);
            let read_data = block.random_read(read_from.into(), builder);

            let masked_read_data =
                builder.mul(read_data.to_target(), read_from_current_block.target);
            final_data = builder.add(final_data, masked_read_data);

            current_offset = builder.sub(current_offset, block_size);
        }

        ByteTarget::from_target_unsafe(final_data)
    }
}

pub fn compose_padded_node_data(
    node_data: Vec<u8>,
) -> [[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS] {
    assert!(node_data.len() <= NODE_DATA_BLOCK_BYTES * MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS);

    let node_data_padded: [u8; NODE_DATA_BLOCK_BYTES * MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS] =
        pad_byte_vec(node_data);
    node_data_padded
        .chunks(NODE_DATA_BLOCK_BYTES)
        .map(|chunk| chunk.try_into().unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use plonky2::plonk::circuit_data::CircuitConfig;

    #[test]
    fn test_node_data_padded_random_read() {
        test_case(
            &[
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
                pad_byte_vec(vec![]),
            ],
            0,
            0,
        );

        let meaningful_data = &[
            pad_byte_vec(vec![0xAA; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xBB; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xCC; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xDD; NODE_DATA_BLOCK_BYTES]),
            pad_byte_vec(vec![0xEE; NODE_DATA_BLOCK_BYTES]),
        ];

        test_case(meaningful_data, 0, 0xAA);
        test_case(meaningful_data, NODE_DATA_BLOCK_BYTES - 1, 0xAA);
        test_case(meaningful_data, NODE_DATA_BLOCK_BYTES, 0xBB);
        test_case(
            meaningful_data,
            NODE_DATA_BLOCK_BYTES * MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS - 1,
            0xEE,
        );
    }

    fn test_case(
        data: &[[u8; NODE_DATA_BLOCK_BYTES]; MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS],
        at: usize,
        expected_data: u8,
    ) {
        let mut config = CircuitConfig::standard_recursion_config();
        config.num_wires = 160;
        config.num_routed_wires = 130;

        let mut builder = CircuitBuilder::<F, D>::new(config);
        let mut pw = PartialWitness::new();

        let node_data = BranchNodeDataPaddedTarget::add_virtual_unsafe(&mut builder);
        node_data.set_witness(data, &mut pw);

        let at_target = builder.add_virtual_target();
        pw.set_target(at_target, F::from_canonical_usize(at));

        let data = node_data.random_read(at_target.into(), &mut builder);
        let expected = ByteTarget::constant(expected_data, &mut builder);

        data.connect(&expected, &mut builder);

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Failed to prove");
        circuit.verify(proof).expect("Failed to verify");
    }
}
