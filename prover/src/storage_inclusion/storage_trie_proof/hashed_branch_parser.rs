//! Circuit that's used to prove correct parsing of branch node.

use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{
            CircuitConfig, CircuitData, CommonCircuitData, VerifierCircuitData,
            VerifierOnlyCircuitData,
        },
        proof::ProofWithPublicInputsTarget,
    },
};
use std::sync::{Arc, OnceLock};

use crate::{
    common::{
        blake2::{CircuitTargets as Blake2CircuitTargets, GenericBlake2Target},
        targets::{impl_parsable_target_set, Blake2Target, TargetSet},
        ProofWithCircuitData,
    },
    prelude::*,
    storage_inclusion::storage_trie_proof::node_parser::{
        MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS, NODE_DATA_BLOCK_BYTES,
    },
};

use super::{
    node_parser::branch_parser::{BranchParser, BranchParserTarget},
    storage_address::StorageAddressTarget,
};

impl_parsable_target_set! {
    /// Public inputs for `HashedBranchParser`.
    pub struct HashedBranchParserTarget {
        /// Blake2 hash of encoded node data.
        pub node_hash: Blake2Target,
        /// Blake2 hash of the children node.
        pub child_node_hash: Blake2Target,

        /// Address composed from all the nodes from root to this.
        pub partial_address: StorageAddressTarget,
        /// `partial_address` concatenated with this node address part.
        pub resulting_partial_address: StorageAddressTarget,
    }
}

pub struct HashedBranchParser {
    /// Inner non-hashed branch parser.
    pub branch_parser: BranchParser,
}

struct HashedBranchParserCircuitTemplate {
    circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    hasher_proof_with_pis: ProofWithPublicInputsTarget<D>,
    branch_proof_with_pis: ProofWithPublicInputsTarget<D>,
    hasher_verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    hasher_common_data: CommonCircuitData<F, D>,
    hasher_verifier_only: VerifierOnlyCircuitData<C, D>,
    branch_verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    branch_common_data: CommonCircuitData<F, D>,
    branch_verifier_only: VerifierOnlyCircuitData<C, D>,
}

impl HashedBranchParserCircuitTemplate {
    fn cached(
        hasher_proof: &ProofWithCircuitData<GenericBlake2Target>,
        branch_proof: &ProofWithCircuitData<BranchParserTarget>,
    ) -> &'static Self {
        // Both inner proof shapes are fixed by the deployment circuit config;
        // node contents remain per-call witnesses.
        static CACHE: OnceLock<HashedBranchParserCircuitTemplate> = OnceLock::new();
        let template = CACHE.get_or_init(|| Self::build(hasher_proof, branch_proof));
        let hasher_verifier_data = hasher_proof.shared_circuit_data();
        if !Arc::ptr_eq(&template.hasher_verifier_data, &hasher_verifier_data) {
            let hasher_data = hasher_verifier_data.as_ref();
            assert_eq!(
                &template.hasher_common_data, &hasher_data.common,
                "HashedBranchParser cache received incompatible hasher common data"
            );
            assert_eq!(
                &template.hasher_verifier_only, &hasher_data.verifier_only,
                "HashedBranchParser cache received incompatible hasher verifier data"
            );
        }
        let branch_verifier_data = branch_proof.shared_circuit_data();
        if !Arc::ptr_eq(&template.branch_verifier_data, &branch_verifier_data) {
            let branch_data = branch_verifier_data.as_ref();
            assert_eq!(
                &template.branch_common_data, &branch_data.common,
                "HashedBranchParser cache received incompatible branch common data"
            );
            assert_eq!(
                &template.branch_verifier_only, &branch_data.verifier_only,
                "HashedBranchParser cache received incompatible branch verifier data"
            );
        }
        template
    }

    fn instantiate(
        &self,
        hasher_proof: &ProofWithCircuitData<GenericBlake2Target>,
        branch_proof: &ProofWithCircuitData<BranchParserTarget>,
    ) -> HashedBranchParserCircuit<'_> {
        let mut witness = PartialWitness::new();
        witness.set_proof_with_pis_target(&self.hasher_proof_with_pis, &hasher_proof.proof());
        witness.set_proof_with_pis_target(&self.branch_proof_with_pis, &branch_proof.proof());
        HashedBranchParserCircuit {
            template: self,
            witness,
        }
    }

    fn build(
        hasher_proof: &ProofWithCircuitData<GenericBlake2Target>,
        branch_proof: &ProofWithCircuitData<BranchParserTarget>,
    ) -> Self {
        log::debug!("Building hashed branch parser circuit template...");

        let hasher_data = hasher_proof.circuit_data();
        let branch_data = branch_proof.circuit_data();
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());

        let hasher_proof_with_pis = builder.add_virtual_proof_with_pis(&hasher_data.common);
        let hasher_verifier = builder.constant_verifier_data(&hasher_data.verifier_only);
        builder.verify_proof::<C>(
            &hasher_proof_with_pis,
            &hasher_verifier,
            &hasher_data.common,
        );
        let hasher_target = GenericBlake2Target::parse_exact(
            &mut hasher_proof_with_pis.public_inputs.clone().into_iter(),
        );

        let branch_proof_with_pis = builder.add_virtual_proof_with_pis(&branch_data.common);
        let branch_verifier = builder.constant_verifier_data(&branch_data.verifier_only);
        builder.verify_proof::<C>(
            &branch_proof_with_pis,
            &branch_verifier,
            &branch_data.common,
        );
        let branch_target = BranchParserTarget::parse_exact(
            &mut branch_proof_with_pis.public_inputs.clone().into_iter(),
        );

        hasher_target
            .length
            .connect(&branch_target.node_data_length, &mut builder);

        let mut branch_parser_node_data = branch_target.padded_node_data.into_targets_iter();
        let mut hasher_node_data = hasher_target.data.into_targets_iter();
        loop {
            let branch_parser_byte = branch_parser_node_data.next();
            let hasher_byte = hasher_node_data.next();

            match (branch_parser_byte, hasher_byte) {
                (Some(a), Some(b)) => builder.connect(a, b),
                (Some(_), None) => {
                    panic!("Generic blake2 hasher circuit have insifficient maximum data length")
                }
                _ => break,
            }
        }

        HashedBranchParserTarget {
            node_hash: hasher_target.hash,
            child_node_hash: branch_target.child_node_hash,
            partial_address: branch_target.partial_address,
            resulting_partial_address: branch_target.resulting_partial_address,
        }
        .register_as_public_inputs(&mut builder);

        let circuit_data = Arc::new(builder.build::<C>());
        let verifier_data = Arc::new(circuit_data.verifier_data());

        Self {
            circuit_data,
            verifier_data,
            hasher_proof_with_pis,
            branch_proof_with_pis,
            hasher_verifier_data: hasher_proof.shared_circuit_data(),
            hasher_common_data: hasher_data.common.clone(),
            hasher_verifier_only: hasher_data.verifier_only.clone(),
            branch_verifier_data: branch_proof.shared_circuit_data(),
            branch_common_data: branch_data.common.clone(),
            branch_verifier_only: branch_data.verifier_only.clone(),
        }
    }
}

struct HashedBranchParserCircuit<'a> {
    template: &'a HashedBranchParserCircuitTemplate,
    witness: PartialWitness<F>,
}

impl HashedBranchParserCircuit<'_> {
    fn prove(self) -> ProofWithCircuitData<HashedBranchParserTarget> {
        ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.template.circuit_data,
            Arc::clone(&self.template.verifier_data),
            self.witness,
        )
    }
}

impl HashedBranchParser {
    pub fn prove(self) -> ProofWithCircuitData<HashedBranchParserTarget> {
        const MAX_DATA_LENGTH_ESTIMATION: usize =
            MAX_BRANCH_NODE_DATA_LENGTH_IN_BLOCKS * NODE_DATA_BLOCK_BYTES;

        let circuit = Blake2CircuitTargets::cached();
        let hasher_proof =
            circuit.prove::<MAX_DATA_LENGTH_ESTIMATION>(&self.branch_parser.node_data);
        let branch_parser_proof = self.branch_parser.prove();

        log::debug!("Composing hasher proof and branch parser proof...");
        let template =
            HashedBranchParserCircuitTemplate::cached(&hasher_proof, &branch_parser_proof);
        let result = template
            .instantiate(&hasher_proof, &branch_parser_proof)
            .prove();

        log::debug!("Composed hasher proof and branch parser proof");

        result
    }
}
