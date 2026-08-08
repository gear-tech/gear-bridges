//! Circuit that's used to prove that majority of validators have signed GRANDPA message.

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{
            CircuitConfig, CircuitData, CommonCircuitData, VerifierCircuitData,
            VerifierCircuitTarget,
        },
        proof::{ProofWithPublicInputs, ProofWithPublicInputsTarget},
    },
    recursion::dummy_circuit::cyclic_base_proof,
};
use plonky2_field::types::Field;
use rayon::ThreadPoolBuilder;
use std::{
    iter,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

mod indexed_validator_sign;
mod single_validator_sign;

use crate::{
    common::{
        array_to_bits, common_data_for_recursion, get_env_variable,
        targets::{
            impl_parsable_target_set, impl_target_set, Blake2Target, ParsableTargetSet, TargetSet,
            VerifierDataTarget,
        },
        BuilderExt, ProofWithCircuitData,
    },
    prelude::{
        consts::{BLAKE2_DIGEST_SIZE, GRANDPA_VOTE_LENGTH},
        *,
    },
};

use self::indexed_validator_sign::IndexedValidatorSignTarget;

use indexed_validator_sign::IndexedValidatorSign;

use super::{validator_set_hash::ValidatorSetHash, GrandpaMessageTarget, ProcessedPreCommit};

impl_target_set! {
    /// Public inputs for `ValidatorSignsChain`.
    pub struct ValidatorSignsChainTarget {
        /// Blake2 hash of concatenated validator public keys.
        pub validator_set_hash: Blake2Target,
        /// GRANDPA message.
        pub message: GrandpaMessageTarget,
    }
}

pub struct ValidatorSignsChain {
    /// `ValidatorSetHash` proof builder.
    pub validator_set_hash: ValidatorSetHash,
    /// All the pre-commits that're planned to process(that is, prove that they're all signed message).
    pub pre_commits: Vec<ProcessedPreCommit>,
    /// GRANDPA message.
    pub message: [u8; GRANDPA_VOTE_LENGTH],
    pub count_thread: Option<usize>,
}

type ProofRequest = (usize, ProofWithCircuitData<IndexedValidatorSignTarget>);

enum Request {
    Pair(Box<(ProofRequest, ProofRequest)>),
    SingleItem(Box<ProofRequest>),
}

impl From<Request>
    for (
        ProofWithCircuitData<IndexedValidatorSignTarget>,
        Option<ProofWithCircuitData<IndexedValidatorSignTarget>>,
    )
{
    fn from(request: Request) -> Self {
        match request {
            Request::SingleItem(data) => {
                let (_index, proof) = *data;
                (proof, None)
            }
            Request::Pair(data) => {
                let (index_1, proof_1) = data.0;
                let (index_2, proof_2) = data.1;

                if index_1 < index_2 {
                    (proof_1, Some(proof_2))
                } else {
                    (proof_2, Some(proof_1))
                }
            }
        }
    }
}

fn send_proof_requests_for_pre_commits<'a>(
    pre_commits: &'a [ProcessedPreCommit],
    mut send_pair: impl FnMut(&'a ProcessedPreCommit, &'a ProcessedPreCommit),
    mut send_single: impl FnMut(&'a ProcessedPreCommit),
) {
    let (chunks, remainder) = pre_commits.as_chunks::<2>();
    debug_assert!(remainder.len() < 2);

    for chunk in chunks {
        let [left, right] = chunk;
        send_pair(left, right);
    }

    if let Some(single) = remainder.first() {
        send_single(single);
    }
}

impl ValidatorSignsChain {
    pub fn prove(mut self) -> ProofWithCircuitData<ValidatorSignsChainTarget> {
        log::debug!("Proving validator signs chain...");

        let validator_set_hash = self.validator_set_hash.compute_hash();

        let now = Instant::now();

        let validator_set_hash_proof = self.validator_set_hash.prove();

        log::info!(
            "validator_set_hash.prove() time: {}ms",
            now.elapsed().as_millis()
        );

        let now = Instant::now();

        self.pre_commits
            .sort_by(|a, b| a.validator_idx.cmp(&b.validator_idx));

        // Bound the queue so slow recursive composition cannot retain every
        // large signer proof while workers continue producing them.
        let channel_capacity = get_env_variable("SIGN_PROOF_CHANNEL_CAPACITY", 2usize).max(1);
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Request>(channel_capacity);
        let composed_result = Arc::new(Mutex::new(None));

        // Keep one bounded pool for both concurrent signer proofs. The old
        // implementation nested a two-thread scheduler around two pools of
        // `worker_thread_count` threads, which could create roughly twice the
        // requested workers and contend with Plonky2's parallel prover.
        // One worker is needed for composition while another proves a signer.
        let worker_thread_count = self.count_thread.unwrap_or(30).max(2);
        let pool = ThreadPoolBuilder::new()
            .stack_size(get_env_variable(
                "RUST_MIN_STACK",
                crate::consts::SIZE_THREAD_STACK_MIN,
            ))
            .num_threads(worker_thread_count)
            .build()
            .expect("ValidatorSignsChain: failed to create ThreadPool");

        let worker_func = |pre_commit: &ProcessedPreCommit| {
            let proof = IndexedValidatorSign {
                public_key: pre_commit.public_key,
                index: pre_commit.validator_idx,
                signature: pre_commit.signature,
                message: self.message,
            }
            .prove(&validator_set_hash_proof);

            (pre_commit.validator_idx, proof)
        };

        // Run composition inside the same pool as signer proofs, rather than
        // creating a separate OS thread that falls back to Rayon’s global pool.
        pool.scope(|scope| {
            let composed_result = Arc::clone(&composed_result);
            scope.spawn(move |_| {
                let result = (|| {
                    let Ok(request) = receiver.recv() else {
                        return None;
                    };

                    let (proof_initial, proof_maybe) = request.into();
                    let initial_data = SignCompositionInitialData {
                        validator_set_hash,
                        message: self.message,
                    };
                    let mut composed_proof =
                        SignComposition::build(&proof_initial).prove_initial(initial_data);
                    if let Some(proof) = proof_maybe {
                        composed_proof =
                            SignComposition::build(&proof).prove_recursive(composed_proof.proof());
                    }

                    while let Ok(request) = receiver.recv() {
                        let (proof, proof_maybe) = request.into();
                        composed_proof =
                            SignComposition::build(&proof).prove_recursive(composed_proof.proof());
                        if let Some(proof) = proof_maybe {
                            composed_proof = SignComposition::build(&proof)
                                .prove_recursive(composed_proof.proof());
                        }
                    }

                    Some(composed_proof)
                })();
                *composed_result.lock().unwrap() = result;
            });

            send_proof_requests_for_pre_commits(
                &self.pre_commits,
                |left, right| {
                    let (result_1, result_2) =
                        pool.join(|| worker_func(left), || worker_func(right));

                    sender
                        .send(Request::Pair(Box::new((result_1, result_2))))
                        .unwrap();
                },
                |single| {
                    let result = pool.install(|| worker_func(single));
                    sender.send(Request::SingleItem(Box::new(result))).unwrap();
                },
            );

            drop(sender);
        });
        let composed_proof = composed_result
            .lock()
            .unwrap()
            .take()
            .expect("composition worker should return");

        log::info!("inner_proofs time: {}ms", now.elapsed().as_millis());

        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let mut witness = PartialWitness::new();

        let composed_proof_pis =
            builder.recursively_verify_constant_proof(&composed_proof, &mut witness);

        // Assert that sign_count > 2/3 * validator_count
        // 3 * sign_count - 2 * validator_count - 1 >= 0
        {
            let triple_sign_count = builder.mul_const(
                F::from_canonical_usize(3),
                composed_proof_pis.inner.sign_count,
            );
            let double_validator_count =
                builder.mul_const(F::TWO, composed_proof_pis.inner.validator_count);
            let lhs = builder.sub(triple_sign_count, double_validator_count);
            let lhs = builder.add_const(lhs, F::NEG_ONE);
            builder.range_check(lhs, 32);
        }

        ValidatorSignsChainTarget {
            validator_set_hash: composed_proof_pis.inner.validator_set_hash,
            message: composed_proof_pis.inner.message,
        }
        .register_as_public_inputs(&mut builder);

        let result = ProofWithCircuitData::prove_from_builder(builder, witness);

        log::debug!("Proven validator signs chain");

        result
    }
}

const VERIFIER_DATA_NUM_CAP_ELEMENTS: usize = 16;

impl_target_set! {
    struct SignCompositionTarget {
        inner: SignCompositionTargetWithoutCircuitData,

        verifier_data: VerifierDataTarget<VERIFIER_DATA_NUM_CAP_ELEMENTS>
    }
}

impl_parsable_target_set! {
    struct SignCompositionTargetWithoutCircuitData {
        validator_set_hash: Blake2Target,
        validator_count: Target,

        message: GrandpaMessageTarget,

        latest_validator_idx: Target,
        sign_count: Target,
    }
}

struct SignCompositionInitialData {
    validator_set_hash: [u8; BLAKE2_DIGEST_SIZE],
    message: [u8; GRANDPA_VOTE_LENGTH],
}

/// Circuit shape shared by all signer-composition layers.
///
/// The inner proof and verifier data are supplied through the witness for each
/// use; only the recursive circuit shape is cached.
struct SignCompositionCircuitTemplate {
    cyclic_circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    common_data: Arc<CommonCircuitData<F, D>>,
    condition: BoolTarget,
    inner_proof_with_pis: ProofWithPublicInputsTarget<D>,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,
    verifier_data_target: VerifierCircuitTarget,
}

impl SignCompositionCircuitTemplate {
    fn cached(inner_proof: &ProofWithCircuitData<IndexedValidatorSignTarget>) -> &'static Self {
        // All IndexedValidatorSign proofs in one proving run use the same
        // circuit. Keep one recursion template instead of rebuilding it for
        // every validator signature. This process-wide cache assumes all
        // signer proofs use the deployment's single circuit configuration.
        static CACHE: OnceLock<SignCompositionCircuitTemplate> = OnceLock::new();
        CACHE.get_or_init(|| Self::build(inner_proof))
    }

    fn build(inner_proof: &ProofWithCircuitData<IndexedValidatorSignTarget>) -> Self {
        log::debug!("    Building sign composition recursion template...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut inner_proof_witness = PartialWitness::new();

        let inner_circuit_data = inner_proof.circuit_data();
        let inner_proof_with_pis = builder.add_virtual_proof_with_pis(&inner_circuit_data.common);
        let inner_verifier_data = builder.constant_verifier_data(&inner_circuit_data.verifier_only);
        builder.verify_proof::<C>(
            &inner_proof_with_pis,
            &inner_verifier_data,
            &inner_circuit_data.common,
        );
        inner_proof_witness.set_proof_with_pis_target(&inner_proof_with_pis, &inner_proof.proof());
        let inner_proof_pis = IndexedValidatorSignTarget::parse_exact(
            &mut inner_proof_with_pis.public_inputs.clone().into_iter(),
        );

        let mut virtual_targets = iter::repeat(()).map(|_| builder.add_virtual_target());
        let future_inner_cyclic_proof_pis =
            SignCompositionTargetWithoutCircuitData::parse(&mut virtual_targets);
        future_inner_cyclic_proof_pis.register_as_public_inputs(&mut builder);

        let verifier_data_target = builder.add_verifier_data_public_inputs();
        let common_data = common_data_for_recursion(builder.num_public_inputs(), NUM_GATES_COMMON);

        let condition = builder.add_virtual_bool_target_safe();

        let inner_cyclic_proof_with_pis = builder.add_virtual_proof_with_pis(&common_data);
        let inner_cyclic_proof_pis = SignCompositionTarget::parse_exact(
            &mut inner_cyclic_proof_with_pis
                .public_inputs
                .clone()
                .into_iter(),
        );
        let mut inner_cyclic_proof_pis = inner_cyclic_proof_pis.inner;

        let neg_one = builder.neg_one();
        inner_cyclic_proof_pis.latest_validator_idx = builder.select(
            condition,
            inner_cyclic_proof_pis.latest_validator_idx,
            neg_one,
        );

        let zero = builder.zero();
        inner_cyclic_proof_pis.sign_count =
            builder.select(condition, inner_cyclic_proof_pis.sign_count, zero);

        inner_cyclic_proof_pis.validator_count = builder.select(
            condition,
            inner_cyclic_proof_pis.validator_count,
            inner_proof_pis.validator_count,
        );

        inner_proof_pis
            .message
            .connect(&inner_cyclic_proof_pis.message, &mut builder);
        inner_proof_pis
            .validator_set_hash
            .connect(&inner_cyclic_proof_pis.validator_set_hash, &mut builder);
        inner_proof_pis
            .validator_count
            .connect(&inner_cyclic_proof_pis.validator_count, &mut builder);

        let validator_idx_diff = builder.sub(
            inner_proof_pis.validator_idx,
            inner_cyclic_proof_pis.latest_validator_idx,
        );
        let to_compare_with_zero = builder.add_const(validator_idx_diff, F::NEG_ONE);
        builder.range_check(to_compare_with_zero, 32);

        let sign_count = builder.add_const(inner_cyclic_proof_pis.sign_count, F::ONE);

        let final_pis = SignCompositionTargetWithoutCircuitData {
            validator_set_hash: inner_cyclic_proof_pis.validator_set_hash,
            validator_count: inner_cyclic_proof_pis.validator_count,
            message: inner_cyclic_proof_pis.message,
            latest_validator_idx: inner_proof_pis.validator_idx,
            sign_count,
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

        log::debug!("    Built sign composition recursion template");

        Self {
            cyclic_circuit_data,
            verifier_data,
            common_data: Arc::new(common_data),
            condition,
            inner_proof_with_pis,
            inner_cyclic_proof_with_pis,
            verifier_data_target,
        }
    }
}

/// Inner cyclic recursion proof.
struct SignComposition {
    cyclic_circuit_data: Arc<CircuitData<F, C, D>>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
    common_data: Arc<CommonCircuitData<F, D>>,
    condition: BoolTarget,
    inner_cyclic_proof_with_pis: ProofWithPublicInputsTarget<D>,
    witness: PartialWitness<F>,
}

impl SignComposition {
    fn prove_initial(
        mut self,
        initial_data: SignCompositionInitialData,
    ) -> ProofWithCircuitData<SignCompositionTarget> {
        log::debug!("    Proving sign composition recursion layer(initial)...");

        let validator_set_hash = array_to_bits(&initial_data.validator_set_hash);
        let message = array_to_bits(&initial_data.message);

        let public_inputs = validator_set_hash
            .into_iter()
            .map(|bit| bit as usize)
            .chain(iter::once(0))
            .chain(message.into_iter().map(|bit| bit as usize))
            .chain(iter::once(0))
            .chain(iter::once(0))
            .map(F::from_canonical_usize);

        // Length check.
        SignCompositionTargetWithoutCircuitData::parse_public_inputs_exact(
            &mut public_inputs.clone(),
        );

        let public_inputs = public_inputs.enumerate().collect();

        self.witness.set_bool_target(self.condition, false);
        self.witness.set_proof_with_pis_target::<C, D>(
            &self.inner_cyclic_proof_with_pis,
            &cyclic_base_proof(
                &self.common_data,
                &self.cyclic_circuit_data.verifier_only,
                public_inputs,
            ),
        );

        let result = ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.cyclic_circuit_data,
            Arc::clone(&self.verifier_data),
            self.witness,
        );

        log::debug!("    Proven sign composition recursion layer(initial)...");

        result
    }

    fn prove_recursive(
        mut self,
        composed_proof: ProofWithPublicInputs<F, C, D>,
    ) -> ProofWithCircuitData<SignCompositionTarget> {
        log::debug!("    Proving sign composition recursion layer...");
        self.witness.set_bool_target(self.condition, true);
        self.witness
            .set_proof_with_pis_target(&self.inner_cyclic_proof_with_pis, &composed_proof);

        let result = ProofWithCircuitData::prove_from_shared_circuit_data(
            &self.cyclic_circuit_data,
            Arc::clone(&self.verifier_data),
            self.witness,
        );

        log::debug!("    Proven sign composition recursion layer");

        result
    }

    fn build(inner_proof: &ProofWithCircuitData<IndexedValidatorSignTarget>) -> Self {
        let template = SignCompositionCircuitTemplate::cached(inner_proof);
        let mut witness = PartialWitness::new();
        witness.set_proof_with_pis_target(&template.inner_proof_with_pis, &inner_proof.proof());
        witness.set_verifier_data_target(
            &template.verifier_data_target,
            &template.cyclic_circuit_data.verifier_only,
        );

        Self {
            cyclic_circuit_data: Arc::clone(&template.cyclic_circuit_data),
            verifier_data: Arc::clone(&template.verifier_data),
            common_data: Arc::clone(&template.common_data),
            condition: template.condition,
            inner_cyclic_proof_with_pis: template.inner_cyclic_proof_with_pis.clone(),
            witness,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre_commit(validator_idx: usize) -> ProcessedPreCommit {
        ProcessedPreCommit {
            validator_idx,
            public_key: [0; consts::ED25519_PUBLIC_KEY_SIZE],
            signature: [0; consts::ED25519_SIGNATURE_SIZE],
        }
    }

    #[test]
    fn proof_request_batches_handle_single_pre_commit() {
        let pre_commits = vec![pre_commit(0)];
        let mut pairs = Vec::new();
        let mut singles = Vec::new();

        send_proof_requests_for_pre_commits(
            &pre_commits,
            |left, right| pairs.push((left.validator_idx, right.validator_idx)),
            |single| singles.push(single.validator_idx),
        );

        assert!(pairs.is_empty());
        assert_eq!(singles, vec![0]);
    }

    #[test]
    fn proof_request_batches_handle_pairs_and_remainder() {
        let pre_commits = vec![pre_commit(0), pre_commit(1), pre_commit(2)];
        let mut pairs = Vec::new();
        let mut singles = Vec::new();

        send_proof_requests_for_pre_commits(
            &pre_commits,
            |left, right| pairs.push((left.validator_idx, right.validator_idx)),
            |single| singles.push(single.validator_idx),
        );

        assert_eq!(pairs, vec![(0, 1)]);
        assert_eq!(singles, vec![2]);
    }
}
