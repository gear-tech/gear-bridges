//! ### Circuit that's used to prove that message was queued for relaying.

use crate::{
    block_finality::BlockFinality,
    common::{
        array_to_bits,
        blake2::{CircuitTargets as Blake2CircuitTargets, MAX_DATA_BYTES},
        get_env_variable,
        targets::{
            impl_target_set, ArrayTarget, Blake2Target, Blake2TargetGoldilocks,
            MessageTargetGoldilocks, TargetBitOperations, TargetSet,
        },
        BuilderExt, ProofWithCircuitData,
    },
    consts::MESSAGE_SIZE_IN_BITS,
    header_chain::{CircuitTargets as HeaderChainCircuit, HeaderChainTarget},
    prelude::*,
    storage_inclusion::StorageInclusion,
};
use parity_scale_codec::Encode;
use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};
use rayon::{
    iter::{IntoParallelRefIterator, ParallelIterator},
    ThreadPool, ThreadPoolBuilder,
};

use std::sync::LazyLock;

fn header_thread_pool() -> &'static ThreadPool {
    static POOL: LazyLock<ThreadPool> = LazyLock::new(|| {
        let stack_size = get_env_variable("RUST_MIN_STACK", crate::consts::SIZE_THREAD_STACK_MIN);
        let thread_count = get_env_variable("HEADER_PROOF_THREADS", 5usize).max(1);
        ThreadPoolBuilder::new()
            .stack_size(stack_size)
            .num_threads(thread_count)
            .build()
            .expect("MessageSent: failed to create ThreadPool")
    });
    &POOL
}

impl_target_set! {
    /// Public inputs for `MessageSent`.
    pub struct MessageSentTarget {
        /// Blake2 hash of concatenated validator public inputs.
        pub validator_set_hash: Blake2TargetGoldilocks,
        /// Actual GRANDPA authority set id.
        pub authority_set_id: Target,
        /// Block number where message was sent.
        pub block_number: Target,
        /// Contents of the message that gets relayed.
        pub message_contents: MessageTargetGoldilocks,
    }
}

impl_target_set! {
    struct MessageInStorageTarget {
        merkle_trie_root: ArrayTarget<BoolTarget, MESSAGE_SIZE_IN_BITS>,
    }
}

impl MessageInStorageTarget {
    fn hash(&self, builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let bit_targets = self
            .clone()
            .into_targets_iter()
            .map(BoolTarget::new_unsafe)
            .collect::<Vec<_>>();
        let mut hash_targets =
            plonky2_blake2b256::circuit::blake2_circuit_from_targets(builder, bit_targets)
                .into_iter()
                .map(|t| t.target);

        Blake2Target::parse_exact(&mut hash_targets)
    }
}

pub struct MessageSent {
    /// Proof that block where message is present in storage is finalized.
    pub block_finality: BlockFinality,
    pub headers: Vec<GearHeader>,
    /// Proof that message is present in the storage.
    pub inclusion_proof: StorageInclusion,
    /// Original data stored in substrate storage.
    pub message_storage_data: Vec<u8>,
}

impl MessageSent {
    pub fn prove(self) -> ProofWithCircuitData<MessageSentTarget> {
        log::debug!("Proving message presence in finalized block...");

        let inclusion_proof = self.inclusion_proof.prove();
        let finality_proof = self.block_finality.prove();

        log::debug!("Composing inclusion and finality proofs...");

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let inclusion_proof_target =
            builder.recursively_verify_constant_proof(&inclusion_proof, &mut witness);
        let finality_proof_target =
            builder.recursively_verify_constant_proof(&finality_proof, &mut witness);

        // Prove the header chain in bounded chunks. The old implementation
        // retained every per-header proof and nested a local pool inside the
        // global Rayon pool. Both behaviors caused large memory spikes on long
        // chains. Hash one chunk in a single explicit pool, fold it into the
        // already-proven suffix, then release the chunk before continuing.
        let chunk_size = get_env_variable("HEADER_PROOF_CHUNK_SIZE", 8usize).max(1);
        let thread_pool = header_thread_pool();

        let circuit_blake2 = Blake2CircuitTargets::cached();
        let circuit_chain = HeaderChainCircuit::cached();
        let mut headers = self.headers;
        headers.sort_by_key(|header| header.number);

        let mut proof_chain = None;
        for header_chunk in headers.rchunks(chunk_size) {
            let proof_hashes = thread_pool.install(|| {
                header_chunk
                    .par_iter()
                    .map(|header| circuit_blake2.prove::<MAX_DATA_BYTES>(header.encode().as_ref()))
                    .collect::<Vec<_>>()
            });

            // rchunks yields newest-to-oldest chunks. Folding each chunk from
            // newest to oldest preserves the original parent-link direction.
            proof_chain = proof_hashes.into_iter().rfold(
                proof_chain,
                |proof_recursive, proof_header_hash| {
                    Some(circuit_chain.prove(&proof_header_hash, proof_recursive.as_ref()))
                },
            );
        }
        let proof_chain = proof_chain.expect("Headers is not an empty list");

        let target_proof_chain = builder.add_virtual_proof_with_pis(circuit_chain.common());
        let target_verifier = builder.constant_verifier_data(circuit_chain.verifier_only());

        builder.verify_proof::<C>(
            &target_proof_chain,
            &target_verifier,
            circuit_chain.common(),
        );

        let mut iter_public_inputs = target_proof_chain.public_inputs.iter().copied();
        let HeaderChainTarget {
            hash_header_start,
            hash_header,
            ..
        } = HeaderChainTarget::parse(&mut iter_public_inputs);

        // connect targets of header chain proof
        inclusion_proof_target
            .block_hash
            .connect(&hash_header, &mut builder);
        finality_proof_target
            .message
            .block_hash
            .connect(&hash_header_start, &mut builder);

        witness.set_proof_with_pis_target(&target_proof_chain, &proof_chain.proof());
        let storage_data_bits = array_to_bits(&self.message_storage_data);
        let mut storage_data_bit_targets = storage_data_bits.into_iter().map(|bit| {
            let target = builder.add_virtual_bool_target_safe();
            witness.set_bool_target(target, bit);
            target.target
        });
        let storage_data_target =
            MessageInStorageTarget::parse_exact(&mut storage_data_bit_targets);

        storage_data_target
            .hash(&mut builder)
            .connect(&inclusion_proof_target.storage_item_hash, &mut builder);

        MessageSentTarget {
            validator_set_hash: Blake2TargetGoldilocks::from_blake2_target(
                finality_proof_target.validator_set_hash,
                &mut builder,
            ),
            authority_set_id: Target::from_u64_bits_le_lossy(
                finality_proof_target.message.authority_set_id,
                &mut builder,
            ),
            block_number: inclusion_proof_target.block_number,
            message_contents: MessageTargetGoldilocks::from_bit_array(
                storage_data_target.merkle_trie_root,
                &mut builder,
            ),
        }
        .register_as_public_inputs(&mut builder);

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}
