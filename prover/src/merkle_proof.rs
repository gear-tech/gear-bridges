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
        targets::{BitArrayTarget, Blake2Target},
        TargetSet,
    },
    prelude::*,
    ProofWithCircuitData,
};

#[derive(Clone)]
pub struct MerkleProofTarget<const LEAF_DATA_LENGTH: usize> {
    pub leaf_data: BitArrayTarget<LEAF_DATA_LENGTH>,
    pub root_hash: Blake2Target,
}

impl<const LEAF_DATA_LENGTH: usize> TargetSet for MerkleProofTarget<LEAF_DATA_LENGTH> {
    fn parse(targets: &mut impl Iterator<Item = Target>) -> Self {
        Self {
            leaf_data: BitArrayTarget::<LEAF_DATA_LENGTH>::parse(targets),
            root_hash: Blake2Target::parse(targets),
        }
    }
}

#[derive(Clone)]
pub struct TrieNodeData {
    pub left_data: Vec<u8>,
    pub right_data: Vec<u8>,
}

#[derive(Clone)]
pub struct MerkleProof<const LEAF_DATA_LENGTH_IN_BITS: usize>
where
    [(); LEAF_DATA_LENGTH_IN_BITS / 8]:,
{
    /// Ordered from leaf to the root.
    pub nodes: Vec<TrieNodeData>,
    pub leaf_data: [u8; LEAF_DATA_LENGTH_IN_BITS / 8],
    pub root_hash: [u8; 32],
}

impl<const LEAF_DATA_LENGTH_IN_BITS: usize> MerkleProof<LEAF_DATA_LENGTH_IN_BITS>
where
    [(); LEAF_DATA_LENGTH_IN_BITS / 8]:,
{
    pub fn prove(&self) -> ProofWithCircuitData<MerkleProofTarget<LEAF_DATA_LENGTH_IN_BITS>> {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());

        let leaf_targets = create_bool_public_inputs(&mut builder, self.leaf_data.len() * 8);
        let mut last_hash_targets =
            blake2_circuit_from_targets(&mut builder, leaf_targets.clone()).to_vec();

        let mut all_node_targets = vec![];
        for node in &self.nodes {
            let left_data_targets = create_bool_targets(&mut builder, node.left_data.len() * 8);

            let right_data_targets = create_bool_targets(&mut builder, node.right_data.len() * 8);

            all_node_targets.push(
                left_data_targets
                    .iter()
                    .chain(right_data_targets.iter())
                    .copied()
                    .collect::<Vec<_>>(),
            );

            let node_targets = left_data_targets
                .into_iter()
                .chain(last_hash_targets.drain(..))
                .chain(right_data_targets.into_iter())
                .collect();

            last_hash_targets = blake2_circuit_from_targets(&mut builder, node_targets).to_vec();
        }

        for target in last_hash_targets {
            builder.register_public_input(target.target);
        }

        let mut pw = PartialWitness::new();

        let leaf_data_bits = array_to_bits(&self.leaf_data).into_iter();
        for (target, bit) in leaf_targets.into_iter().zip(leaf_data_bits) {
            pw.set_bool_target(target, bit);
        }

        for (node_targets, node) in all_node_targets.into_iter().zip(self.nodes.iter()) {
            let node_data = array_to_bits(
                &node
                    .left_data
                    .iter()
                    .chain(node.right_data.iter())
                    .copied()
                    .collect::<Vec<_>>(),
            );

            for (target, bit) in node_targets.into_iter().zip(node_data.into_iter()) {
                pw.set_bool_target(target, bit);
            }
        }

        ProofWithCircuitData::from_builder(builder, pw)
    }
}

fn create_bool_targets(
    builder: &mut CircuitBuilder<F, D>,
    length_in_bits: usize,
) -> Vec<BoolTarget> {
    iter::repeat(())
        .take(length_in_bits)
        .map(|_| builder.add_virtual_bool_target_safe())
        .collect()
}

fn create_bool_public_inputs(
    builder: &mut CircuitBuilder<F, D>,
    length_in_bits: usize,
) -> Vec<BoolTarget> {
    let targets = create_bool_targets(builder, length_in_bits);
    for target in &targets {
        builder.register_public_input(target.target);
    }
    targets
}
