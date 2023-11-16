use std::iter;

use plonky2::iop::target::BoolTarget;
use plonky2::iop::witness::{PartialWitness, WitnessWrite};
use plonky2_field::goldilocks_field::GoldilocksField;
use plonky2_field::types::{Field64, PrimeField64};

use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::plonk::circuit_data::CircuitConfig;
use plonky2_blake2b256::circuit::blake2_circuit_from_targets;

use crate::{common::array_to_bits, prelude::*, ProofWithCircuitData};

pub struct TrieNodeData {
    pub left_data: Vec<u8>,
    pub right_data: Vec<u8>,
}

pub struct MerkleProof {
    /// Ordered from leaf to the root.
    pub nodes: Vec<TrieNodeData>,
    pub leaf_data: Vec<u8>,
    pub root_hash: [u8; 32],
}

impl MerkleProof {
    pub fn prove(&self) -> ProofWithCircuitData {
        // order of public inputs:
        // - leaf data
        // - node #0 left data
        // - node #0 right data
        // - node #1 left data
        // - ...
        // - node #n right data
        // - root hash

        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());

        let leaf_targets = create_bool_public_inputs(&mut builder, self.leaf_data.len() * 8);
        let mut last_hash_targets =
            blake2_circuit_from_targets(&mut builder, leaf_targets.clone()).to_vec();

        let mut all_node_targets = vec![];
        for node in &self.nodes {
            let left_data_targets =
                create_bool_public_inputs(&mut builder, node.left_data.len() * 8);

            let right_data_targets =
                create_bool_public_inputs(&mut builder, node.right_data.len() * 8);

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

        let circuit = builder.build::<C>();

        // Set public inputs.
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

        let proof = circuit.prove(pw).unwrap();

        ProofWithCircuitData {
            proof,
            circuit_data: circuit,
        }
    }
}

fn create_bool_public_inputs(
    builder: &mut CircuitBuilder<F, D>,
    length_in_bits: usize,
) -> Vec<BoolTarget> {
    iter::repeat(())
        .take(length_in_bits)
        .map(|_| {
            let target = builder.add_virtual_bool_target_safe();
            builder.register_public_input(target.target);
            target
        })
        .collect()
}
