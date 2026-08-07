//! Circuit that's used to prove correct parsing of all the branch nodes from root to leaf.

use plonky2::{
    iop::{
        target::BoolTarget,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{
            CircuitConfig, CircuitData, CommonCircuitData, VerifierCircuitData,
            VerifierCircuitTarget, VerifierOnlyCircuitData,
        },
        proof::{ProofWithPublicInputs, ProofWithPublicInputsTarget},
    },
    recursion::dummy_circuit::cyclic_base_proof,
};
use plonky2_field::types::Field;
use sp_core::{Blake2Hasher, Hasher};
use std::{
    iter,
    sync::{Arc, OnceLock},
};

use super::{
    hashed_branch_parser::{HashedBranchParser, HashedBranchParserTarget},
    storage_address::StorageAddressTarget,
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
    /// Public inputs for `BranchNodeChain`.
    pub struct BranchNodeChainParserTarget {
        /// Storage root.
        pub root_hash: Blake2Target,
        /// Blake2 hash of encoded leaf data.
        pub leaf_hash: Blake2Target,
        /// Result of composition of all addresses from root node to the last branch node.
        pub partial_address: StorageAddressTarget,
    }
}

impl_target_set! {
    /// Intermediate target for cyclic recursion.
    struct BranchNodeChainParserTargetWithVerifierData {
        inner: BranchNodeChainParserTarget,
        verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>
    }
}

pub struct BranchNodeChain {
    /// Encoded branch nodes, arranged from root to leaf.
    pub nodes: Vec<BranchNodeData>,
}

struct FinalProjectionTemplate {
    circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    inner_proof_with_pis: ProofWithPublicInputsTarget<D>,
    inner_verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    inner_common_data: CommonCircuitData<F, D>,
    inner_verifier_only: VerifierOnlyCircuitData<C, D>,
}

impl FinalProjectionTemplate {
    fn cached(
        inner_proof: &ProofWithCircuitData<BranchNodeChainParserTargetWithVerifierData>,
    ) -> &'static Self {
        // The projection has one fixed inner proof shape; chain contents are
        // carried by the per-call inner proof witness.
        static CACHE: OnceLock<FinalProjectionTemplate> = OnceLock::new();
        let template = CACHE.get_or_init(|| Self::build(inner_proof));
        let inner_verifier_data = inner_proof.shared_circuit_data();
        if !Arc::ptr_eq(&template.inner_verifier_data, &inner_verifier_data) {
            let inner_data = inner_verifier_data.as_ref();
            assert_eq!(
                &template.inner_common_data, &inner_data.common,
                "BranchNodeChain final projection received incompatible common data"
            );
            assert_eq!(
                &template.inner_verifier_only, &inner_data.verifier_only,
                "BranchNodeChain final projection received incompatible verifier data"
            );
        }
        template
    }

    fn instantiate(
        &self,
        inner_proof: &ProofWithCircuitData<BranchNodeChainParserTargetWithVerifierData>,
    ) -> FinalProjectionCircuit<'_> {
        let mut witness = PartialWitness::new();
        witness.set_proof_with_pis_target(&self.inner_proof_with_pis, &inner_proof.proof());
        FinalProjectionCircuit {
            template: self,
            witness,
        }
    }

    fn build(
        inner_proof: &ProofWithCircuitData<BranchNodeChainParserTargetWithVerifierData>,
    ) -> Self {
        log::debug!("Building branch node chain final projection template...");

        let inner_data = inner_proof.circuit_data();
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let inner_proof_with_pis = builder.add_virtual_proof_with_pis(&inner_data.common);
        let inner_verifier = builder.constant_verifier_data(&inner_data.verifier_only);
        builder.verify_proof::<C>(&inner_proof_with_pis, &inner_verifier, &inner_data.common);
        let inner_target = BranchNodeChainParserTargetWithVerifierData::parse_exact(
            &mut inner_proof_with_pis.public_inputs.clone().into_iter(),
        );

        BranchNodeChainParserTarget {
            root_hash: inner_target.inner.root_hash,
            leaf_hash: inner_target.inner.leaf_hash,
            partial_address: inner_target.inner.partial_address,
        }
        .register_as_public_inputs(&mut builder);

        let circuit_data = Arc::new(builder.build::<C>());
        let verifier_data = Arc::new(circuit_data.verifier_data());

        Self {
            circuit_data,
            verifier_data,
            inner_proof_with_pis,
            inner_verifier_data: inner_proof.shared_circuit_data(),
            inner_common_data: inner_data.common.clone(),
            inner_verifier_only: inner_data.verifier_only.clone(),
        }
    }
}

struct FinalProjectionCircuit<'a> {
    template: &'a FinalProjectionTemplate,
    witness: PartialWitness<F>,
}

impl FinalProjectionCircuit<'_> {
    fn prove(self) -> ProofWithCircuitData<BranchNodeChainParserTarget> {
        ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.template.circuit_data,
            Arc::clone(&self.template.verifier_data),
            self.witness,
        )
    }
}

impl BranchNodeChain {
    pub fn prove(self) -> ProofWithCircuitData<BranchNodeChainParserTarget> {
        log::debug!("Proving branch node chain...");

        let inner = self.inner_proof();

        let result = FinalProjectionTemplate::cached(&inner)
            .instantiate(&inner)
            .prove();

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

            let inner_proof = inner_circuit.prove();
            let circuit = CircuitTemplate::cached(&inner_proof).instantiate(&inner_proof);

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

struct CircuitTemplate {
    cyclic_circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    common_data: Arc<CommonCircuitData<F, D>>,
    inner_proof_with_pis: ProofWithPublicInputsTarget<D>,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,
    condition: BoolTarget,
    verifier_data_target: VerifierCircuitTarget,
    inner_verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    inner_verifier_only: VerifierOnlyCircuitData<C, D>,
    inner_common_data: CommonCircuitData<F, D>,
}

impl CircuitTemplate {
    fn cached(inner_proof: &ProofWithCircuitData<HashedBranchParserTarget>) -> &'static Self {
        static CACHE: OnceLock<CircuitTemplate> = OnceLock::new();
        let template = CACHE.get_or_init(|| Self::build(inner_proof));
        let inner_verifier_data = inner_proof.shared_circuit_data();
        if !Arc::ptr_eq(&template.inner_verifier_data, &inner_verifier_data) {
            let inner_data = inner_verifier_data.as_ref();
            assert_eq!(
                &template.inner_common_data, &inner_data.common,
                "BranchNodeChain cache received incompatible inner common data"
            );
            assert_eq!(
                &template.inner_verifier_only, &inner_data.verifier_only,
                "BranchNodeChain cache received incompatible inner verifier data"
            );
        }
        template
    }

    fn instantiate<'a>(
        &'a self,
        inner_proof: &ProofWithCircuitData<HashedBranchParserTarget>,
    ) -> Circuit<'a> {
        let mut witness = PartialWitness::new();
        witness.set_verifier_data_target(
            &self.verifier_data_target,
            &self.cyclic_circuit_data.verifier_only,
        );
        witness.set_proof_with_pis_target(&self.inner_proof_with_pis, &inner_proof.proof());
        Circuit {
            template: self,
            witness,
        }
    }

    fn build(inner_proof: &ProofWithCircuitData<HashedBranchParserTarget>) -> Self {
        log::debug!("    Building storage trie recursion template...");

        let inner_data = inner_proof.circuit_data();
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let inner_proof_with_pis = builder.add_virtual_proof_with_pis(&inner_data.common);
        let inner_verifier_data = builder.constant_verifier_data(&inner_data.verifier_only);
        builder.verify_proof::<C>(
            &inner_proof_with_pis,
            &inner_verifier_data,
            &inner_data.common,
        );
        let inner_proof_pis = HashedBranchParserTarget::parse_exact(
            &mut inner_proof_with_pis.public_inputs.clone().into_iter(),
        );

        let mut virtual_targets = iter::repeat(()).map(|_| builder.add_virtual_target());
        let future_inner_cyclic_proof_pis =
            BranchNodeChainParserTarget::parse(&mut virtual_targets);
        future_inner_cyclic_proof_pis.register_as_public_inputs(&mut builder);

        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(builder.num_public_inputs(), NUM_GATES_COMMON);

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);
        let inner_cyclic_proof_pis = BranchNodeChainParserTargetWithVerifierData::parse_exact(
            &mut inner_cyclic_proof_with_pis
                .public_inputs
                .clone()
                .into_iter(),
        );
        let mut inner_cyclic_proof_pis = BranchNodeChainParserTarget {
            root_hash: inner_cyclic_proof_pis.inner.root_hash,
            leaf_hash: inner_cyclic_proof_pis.inner.leaf_hash,
            partial_address: inner_cyclic_proof_pis.inner.partial_address,
        };

        inner_cyclic_proof_pis.leaf_hash = builder.select_target_set(
            condition,
            &inner_cyclic_proof_pis.leaf_hash,
            &inner_cyclic_proof_pis.root_hash,
        );
        let empty_address = StorageAddressTarget::empty(&mut builder);
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

        let cyclic_circuit_data = Arc::new(builder.build::<C>());
        let verifier_data = Arc::new(cyclic_circuit_data.verifier_data());

        log::debug!("    Built storage trie recursion template");

        Self {
            cyclic_circuit_data,
            verifier_data,
            common_data: Arc::new(common_data),
            inner_proof_with_pis,
            inner_cyclic_proof_with_pis,
            condition,
            verifier_data_target,
            inner_verifier_data: inner_proof.shared_circuit_data(),
            inner_verifier_only: inner_data.verifier_only.clone(),
            inner_common_data: inner_data.common.clone(),
        }
    }
}

struct Circuit<'a> {
    template: &'a CircuitTemplate,
    witness: PartialWitness<F>,
}

impl Circuit<'_> {
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

        self.witness.set_bool_target(self.template.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.template.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.template.common_data,
                &self.template.cyclic_circuit_data.verifier_only,
                public_inputs,
            ),
        );

        let result = ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.template.cyclic_circuit_data,
            Arc::clone(&self.template.verifier_data),
            self.witness,
        );

        log::debug!("    Proven storage trie recursion layer(initial)...");

        result
    }

    fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<BranchNodeChainParserTargetWithVerifierData> {
        log::debug!("    Proving storage trie recursion layer...");
        self.witness.set_bool_target(self.template.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.template.inner_cyclic_proof_with_pis, &composed_proof);

        let result = ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.template.cyclic_circuit_data,
            Arc::clone(&self.template.verifier_data),
            self.witness,
        );

        log::debug!("    Proven storage trie recursion layer");

        result
    }
}
