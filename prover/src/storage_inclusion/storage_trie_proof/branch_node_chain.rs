use plonky2::{
    iop::{
        target::BoolTarget,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, CommonCircuitData},
        proof::{ProofWithPublicInputs, ProofWithPublicInputsTarget},
    },
    recursion::dummy_circuit::cyclic_base_proof,
};
use plonky2_field::types::Field;
use sp_core::{Blake2Hasher, Hasher};
use std::iter;

use super::{
    hashed_branch_parser::HashedBranchParser, storage_address::PartialStorageAddressTarget,
    BranchNodeData,
};
use crate::{
    common::{
        array_to_bits, common_data_for_recursion,
        targets::{
            impl_parsable_target_set, impl_target_set, Blake2Target, ParsableTargetSet, TargetSet,
            VerifierDataTarget,
        },
        BuilderExt, ProofWithCircuitData,
    },
    prelude::{consts::BLAKE2_DIGEST_SIZE, *},
    storage_inclusion::storage_trie_proof::node_parser::branch_parser::BranchParser,
};

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

impl_parsable_target_set! {
    pub struct BranchNodeChainParserTarget {
        pub root_hash: Blake2Target,
        pub leaf_hash: Blake2Target,
        pub partial_address: PartialStorageAddressTarget,
    }
}

impl_target_set! {
    struct BranchNodeChainParserTargetWithVerifierData {
        root_hash: Blake2Target,
        leaf_hash: Blake2Target,
        partial_address: PartialStorageAddressTarget,

        verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>
    }
}

pub struct BranchNodeChain {
    pub nodes: Vec<BranchNodeData>,
}

impl BranchNodeChain {
    pub fn prove(self) -> ProofWithCircuitData<BranchNodeChainParserTarget> {
        log::debug!("Proving branch node chain...");

        let inner = self.inner_proof();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let public_inputs = builder.recursively_verify_constant_proof(&inner, &mut witness);

        BranchNodeChainParserTarget {
            root_hash: public_inputs.root_hash,
            leaf_hash: public_inputs.leaf_hash,
            partial_address: public_inputs.partial_address,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::prove_from_builder(builder, witness);

        log::debug!("Proven branch node chain");

        result
    }

    fn inner_proof(self) -> ProofWithCircuitData<BranchNodeChainParserTargetWithVerifierData> {
        let root_hash = Blake2Hasher::hash(&self.nodes[0].data).0;

        let mut composed_proof: Option<ProofWithCircuitData<_>> = None;
        for node in self.nodes {
            let partial_address_nibbles = if let Some(composed_proof) = &composed_proof {
                let address = BranchNodeChainParserTarget::parse_public_inputs(
                    &mut composed_proof.public_inputs().into_iter(),
                )
                .partial_address;
                address.padded_address[..address.length as usize].to_vec()
            } else {
                vec![]
            };

            let inner_circuit = HashedBranchParser {
                branch_parser: BranchParser {
                    node_data: node.data,
                    claimed_child_node_nibble: node.child_nibble,
                    partial_address_nibbles,
                },
            };

            let circuit = Circuit::build(inner_circuit);

            let new_proof = if let Some(composed_proof) = composed_proof {
                circuit.prove_recursive(composed_proof.proof())
            } else {
                circuit.prove_initial(root_hash)
            };
            composed_proof = Some(new_proof);
        }

        composed_proof.expect("Non-zero node count")
    }
}

struct Circuit {
    cyclic_circuit_data: CircuitData<F, C, D>,

    common_data: CommonCircuitData<F, D>,

    condition: BoolTarget,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,

    witness: PartialWitness<F>,
}

impl Circuit {
    fn prove_initial(
        mut self,
        root_hash: [u8; BLAKE2_DIGEST_SIZE],
    ) -> ProofWithCircuitData<BranchNodeChainParserTargetWithVerifierData> {
        log::debug!("    Proving storage trie recursion layer(initial)...");

        let root_hash_bits = array_to_bits(&root_hash);
        let public_inputs = root_hash_bits
            .into_iter()
            .map(F::from_bool)
            .enumerate()
            .collect();

        self.witness.set_bool_target(self.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.common_data,
                &self.cyclic_circuit_data.verifier_only,
                public_inputs,
            ),
        );

        let result =
            ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness);

        log::debug!("    Proven storage trie recursion layer(initial)...");

        result
    }

    fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<BranchNodeChainParserTargetWithVerifierData> {
        log::debug!("    Proving storage trie recursion layer...");
        self.witness.set_bool_target(self.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.inner_cyclic_proof_with_pis, &composed_proof);

        let result =
            ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness);

        log::debug!("    Proven storage trie recursion layer");

        result
    }

    fn build(inner: HashedBranchParser) -> Circuit {
        let inner_proof = inner.prove();

        log::debug!("    Building storage trie recursion layer...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut pw = PartialWitness::new();

        let inner_proof_pis = builder.recursively_verify_constant_proof(&inner_proof, &mut pw);

        let mut virtual_targets = iter::repeat(()).map(|_| builder.add_virtual_target());
        let future_inner_cyclic_proof_pis =
            BranchNodeChainParserTarget::parse(&mut virtual_targets);
        future_inner_cyclic_proof_pis.register_as_public_inputs(&mut builder);

        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(
            CircuitConfig::standard_recursion_config(),
            builder.num_public_inputs(),
            1 << 13,
        );

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);
        let inner_cyclic_proof_pis = BranchNodeChainParserTargetWithVerifierData::parse_exact(
            &mut inner_cyclic_proof_with_pis
                .public_inputs
                .clone()
                .into_iter(),
        );
        let mut inner_cyclic_proof_pis = BranchNodeChainParserTarget {
            root_hash: inner_cyclic_proof_pis.root_hash,
            leaf_hash: inner_cyclic_proof_pis.leaf_hash,
            partial_address: inner_cyclic_proof_pis.partial_address,
        };

        inner_cyclic_proof_pis.leaf_hash = builder.select_target_set(
            condition,
            &inner_cyclic_proof_pis.leaf_hash,
            &inner_cyclic_proof_pis.root_hash,
        );
        let empty_address = PartialStorageAddressTarget::empty(&mut builder);
        inner_cyclic_proof_pis.partial_address = builder.select_target_set(
            condition,
            &inner_cyclic_proof_pis.partial_address,
            &empty_address,
        );

        inner_proof_pis
            .node_hash
            .connect(&inner_cyclic_proof_pis.leaf_hash, &mut builder);
        inner_proof_pis
            .partial_address
            .connect(&inner_cyclic_proof_pis.partial_address, &mut builder);

        let final_pis = BranchNodeChainParserTarget {
            root_hash: inner_cyclic_proof_pis.root_hash,
            leaf_hash: inner_proof_pis.child_node_hash,
            partial_address: inner_proof_pis.resulting_partial_address,
        };

        future_inner_cyclic_proof_pis.connect(&final_pis, &mut builder);

        builder
            .conditionally_verify_cyclic_proof_or_dummy::<C>(
                condition,
                &inner_cyclic_proof_with_pis,
                &common_data,
            )
            .expect("Failed to build circuit");

        let cyclic_circuit_data = builder.build::<C>();

        pw.set_verifier_data_target(&verifier_data_target, &cyclic_circuit_data.verifier_only);

        log::debug!("    Built storage parser recursion layer");

        Circuit {
            cyclic_circuit_data,
            common_data,
            condition,
            inner_cyclic_proof_with_pis,
            witness: pw,
        }
    }
}
