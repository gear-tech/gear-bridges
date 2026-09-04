//! Circuit that's used to prove that majority of validators have signed GRANDPA message.

use plonky2::{
    iop::{
        target::{BoolTarget, Target},
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
use rayon::ThreadPoolBuilder;
use std::{iter, time::Instant};

mod indexed_validator_sign;
mod single_validator_sign;

use crate::{
    common::{
        array_to_bits, common_data_for_recursion,
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

        let (sender, receiver) = std::sync::mpsc::channel::<Request>();
        let thread = std::thread::spawn(move || {
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
                    composed_proof =
                        SignComposition::build(&proof).prove_recursive(composed_proof.proof());
                }
            }

            Some(composed_proof)
        });

        let pool = ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let worker_thread_count = self.count_thread.unwrap_or(30);
        let pools = [
            ThreadPoolBuilder::new()
                .num_threads(worker_thread_count)
                .build()
                .unwrap(),
            ThreadPoolBuilder::new()
                .num_threads(worker_thread_count)
                .build()
                .unwrap(),
        ];

        let worker_func = |pre_commit: &ProcessedPreCommit, pool: &rayon::ThreadPool| {
            let (index, proof) = pool.install(|| {
                let proof = IndexedValidatorSign {
                    public_key: pre_commit.public_key,
                    index: pre_commit.validator_idx,
                    signature: pre_commit.signature,
                    message: self.message,
                }
                .prove(&validator_set_hash_proof);

                (pre_commit.validator_idx, proof)
            });

            (index, proof)
        };

        send_proof_requests_for_pre_commits(
            &self.pre_commits,
            |left, right| {
                let (result_1, result_2) = pool.join(
                    || worker_func(left, &pools[0]),
                    || worker_func(right, &pools[1]),
                );

                sender
                    .send(Request::Pair(Box::new((result_1, result_2))))
                    .unwrap();
            },
            |single| {
                sender
                    .send(Request::SingleItem(Box::new(worker_func(
                        single, &pools[1],
                    ))))
                    .unwrap();
            },
        );

        drop(sender);
        let composed_proof = thread
            .join()
            .expect("should be joinable")
            .expect("there is a proof");

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

/// Inner cyclic recursion proof.
struct SignComposition {
    cyclic_circuit_data: CircuitData<F, C, D>,

    common_data: CommonCircuitData<F, D>,

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

        let result =
            ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness);

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

        let result =
            ProofWithCircuitData::prove_from_circuit_data(&self.cyclic_circuit_data, self.witness);

        log::debug!("    Proven sign composition recursion layer");

        result
    }

    fn build(inner_proof: &ProofWithCircuitData<IndexedValidatorSignTarget>) -> SignComposition {
        log::debug!("    Building sign composition recursion layer...");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut pw = PartialWitness::new();

        let inner_proof_pis = builder.recursively_verify_constant_proof(inner_proof, &mut pw);

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

        let cyclic_circuit_data = builder.build::<C>();

        pw.set_verifier_data_target(&verifier_data_target, &cyclic_circuit_data.verifier_only);

        log::debug!("    Built sign composition recursion layer");

        SignComposition {
            cyclic_circuit_data,
            common_data,
            condition,
            inner_cyclic_proof_with_pis,
            witness: pw,
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

    // ---- SECURITY(CR-1) regression tests -------------------------------------------
    //
    // Constraint-satisfaction level (no `VBLAKE2_CACHE_PATH` needed): circuits are built
    // straight from `VariativeBlake2::create_builder_targets`, mirroring the exact wiring of
    // `ValidatorSetHash::prove` (bit carve) and `IndexedValidatorSign::prove`
    // (slot read + slot<->pk connect).

    use std::panic::{catch_unwind, AssertUnwindSafe};

    use plonky2_blake2b256::circuit::BLOCK_BYTES;

    use crate::common::{
        blake2::{variative::VariativeBlake2, MAX_DATA_BYTES},
        targets::{ByteTarget, Ed25519PublicKeyTarget, PaddedValidatorSetTarget},
    };

    /// Deterministic fake validator public key; every byte is non-zero so any constraint
    /// check against zeroed padding fires regardless of bit ordering.
    fn fake_public_key(seed: usize) -> [u8; consts::ED25519_PUBLIC_KEY_SIZE] {
        core::array::from_fn(|i| (((seed + 1) * 31 + i * 7) as u8) | 1)
    }

    /// Constraint violations surface either as a prover panic (copy-constraint collision)
    /// or as an error from `prove`; both mean "not satisfied".
    fn prove_satisfies(circuit: &CircuitData<F, C, D>, witness: PartialWitness<F>) -> bool {
        catch_unwind(AssertUnwindSafe(move || circuit.prove(witness).is_ok())).unwrap_or(false)
    }

    /// Carve the 64 padded validator slots out of the blake2 `data` byte targets, exactly
    /// like `ValidatorSetHash::prove` does.
    fn carve_validator_set(
        builder: &mut CircuitBuilder<F, D>,
        data_targets: &[Target],
    ) -> PaddedValidatorSetTarget {
        let mut bit_targets = data_targets.iter().flat_map(|target| {
            ByteTarget::from_target_unsafe(*target)
                .as_bit_targets(builder)
                .0
                .into_iter()
                .rev()
                .map(|bit| bit.target)
        });
        PaddedValidatorSetTarget::parse(&mut bit_targets)
    }

    /// Connect a read public key target to fixed expected bytes (MSB-first per byte, the
    /// same order the carve above produces), standing in for the `SingleValidatorSign`
    /// proof the vote would carry.
    fn connect_pk_to_constants(
        builder: &mut CircuitBuilder<F, D>,
        pk: &Ed25519PublicKeyTarget,
        expected: &[u8; consts::ED25519_PUBLIC_KEY_SIZE],
    ) {
        for (bit, expected_bit) in (**pk).0.iter().zip(array_to_bits(expected)) {
            let constant = builder.constant(F::from_canonical_usize(expected_bit as usize));
            builder.connect(bit.target, constant);
        }
    }

    /// Set the full `VariativeBlake2` witness: declared `length` + all data bytes (zeroed
    /// past `data`). `data` may extend beyond `length` — that tail is what the padding
    /// constraints must force to zero.
    fn set_blake2_witness(
        length_target: Target,
        data_targets: &[Target],
        declared_length: usize,
        data: &[u8],
        witness: &mut PartialWitness<F>,
    ) {
        assert_eq!(data_targets.len(), MAX_DATA_BYTES);
        assert!(data.len() <= MAX_DATA_BYTES);
        witness.set_target(length_target, F::from_canonical_usize(declared_length));
        for (target, byte) in data_targets.iter().zip(
            data.iter()
                .copied()
                .chain(iter::repeat(0))
                .take(MAX_DATA_BYTES),
        ) {
            witness.set_target(*target, F::from_canonical_u8(byte));
        }
    }

    /// (a) Negative: attacker keys planted into padded validator slots beyond
    /// `4*ceil(n/4)` — i.e. past the zero-constrained blake2 window — must fail constraint
    /// satisfaction. Pre-fix these bytes were unconstrained public inputs, so an attacker
    /// vote reading such a slot satisfied every circuit (forgery, see CR-1 PoC).
    #[test]
    fn padded_attacker_validator_keys_fail_constraints() {
        const N: usize = 15; // historical Vara era size (probed live)
        let honest_set: Vec<u8> = (0..N).flat_map(fake_public_key).collect();
        let honest_len = honest_set.len();

        // Blakes2 window is ceil(480/128)*128 = 512 bytes = slots [0, 16); the first slot
        // not covered by the pre-fix padding check is 4*ceil(15/4) = 16.
        let attacker_slot = 4 * N.div_ceil(4);
        assert_eq!(
            attacker_slot * consts::ED25519_PUBLIC_KEY_SIZE,
            honest_set.len().div_ceil(BLOCK_BYTES) * BLOCK_BYTES
        );
        let attacker_key = fake_public_key(attacker_slot + 64);

        let (mut builder, targets) = VariativeBlake2::create_builder_targets(honest_set.len());
        let (length_target, data_targets) = targets.split_first().expect("length + data targets");

        // Mirror the forged `IndexedValidatorSign`: the vote reads the attacker slot and
        // the slot<->pk connect passes against the attacker's key.
        let validator_set = carve_validator_set(&mut builder, data_targets);
        let read_at = builder.constant(F::from_canonical_usize(attacker_slot));
        let read_pk = validator_set.random_read(read_at, &mut builder);
        connect_pk_to_constants(&mut builder, &read_pk, &attacker_key);

        // data = honest keys, zero window padding, attacker key just past the window end.
        let mut data = honest_set;
        data.resize(attacker_slot * consts::ED25519_PUBLIC_KEY_SIZE, 0);
        data.extend(attacker_key);

        let mut witness = PartialWitness::new();
        set_blake2_witness(*length_target, data_targets, honest_len, &data, &mut witness);

        assert!(
            !prove_satisfies(&builder.build::<C>(), witness),
            "CR-1: padded attacker key in slot {attacker_slot} must violate the \
             full-window padding zero constraints"
        );
    }

    /// (b) Negative: a vote whose index equals (or exceeds) the real validator count must
    /// fail the `IndexedValidatorSign` range gate; the last honest index must pass.
    #[test]
    fn vote_index_must_be_below_validator_count() {
        let mut builder = CircuitBuilder::new(CircuitConfig::standard_recursion_config());
        let count_target = builder.add_virtual_target();
        let idx_target = builder.add_virtual_target();
        IndexedValidatorSign::constrain_vote_index(&mut builder, count_target, idx_target);
        let circuit = builder.build::<C>();

        let witness = |count: usize, idx: usize| {
            let mut w = PartialWitness::new();
            w.set_target(count_target, F::from_canonical_usize(count));
            w.set_target(idx_target, F::from_canonical_usize(idx));
            w
        };

        // The first padded slot address, idx == validator_count.
        assert!(
            !prove_satisfies(&circuit, witness(15, 15)),
            "CR-1: idx == validator_count must fail the range gate"
        );
        assert!(
            !prove_satisfies(&circuit, witness(15, 16)),
            "CR-1: idx > validator_count must fail the range gate"
        );
        assert!(
            prove_satisfies(&circuit, witness(15, 14)),
            "the last real validator (idx == validator_count - 1) must pass"
        );
    }

    /// (c) Positive (compat guard): honest full-signature quorums at the historical era
    /// sizes n = 15 and n = 59 — zeroed padding within the full data window, every vote
    /// index < validator_count, quorum inequality `3*sign_count - 2*validator_count - 1 >= 0`
    /// — must still satisfy the constraints.
    #[test]
    fn honest_full_sign_quorum_still_satisfies_at_historical_sizes() {
        for n in [15usize, 59] {
            let honest_set: Vec<u8> = (0..n).flat_map(fake_public_key).collect();
            let (mut builder, targets) = VariativeBlake2::create_builder_targets(honest_set.len());
            let (length_target, data_targets) =
                targets.split_first().expect("length + data targets");

            // Same count<->length binding as `ValidatorSetHash::prove`.
            let validator_count = builder.add_virtual_target();
            let desired_len = builder
                .mul_const(F::from_canonical_usize(consts::ED25519_PUBLIC_KEY_SIZE), validator_count);
            builder.connect(desired_len, *length_target);

            // Full quorum: every one of the n validators signs, each vote through the
            // new range gate.
            for idx in 0..n {
                let idx_target = builder.constant(F::from_canonical_usize(idx));
                IndexedValidatorSign::constrain_vote_index(&mut builder, validator_count, idx_target);
            }

            // Representative votes: slot reads for the first and last honest validator,
            // connected to their real keys (the slot<->pk connect of each vote proof).
            let validator_set = carve_validator_set(&mut builder, data_targets);
            for slot in [0usize, n - 1] {
                let read_at = builder.constant(F::from_canonical_usize(slot));
                let pk = validator_set.random_read(read_at, &mut builder);
                connect_pk_to_constants(&mut builder, &pk, &fake_public_key(slot));
            }

            // Same quorum inequality as the final check of `ValidatorSignsChain::prove`.
            let sign_count = builder.constant(F::from_canonical_usize(n));
            let triple_sign_count = builder.mul_const(F::from_canonical_usize(3), sign_count);
            let double_validator_count = builder.mul_const(F::TWO, validator_count);
            let lhs = builder.sub(triple_sign_count, double_validator_count);
            let lhs = builder.add_const(lhs, F::NEG_ONE);
            builder.range_check(lhs, 32);

            let mut witness = PartialWitness::new();
            set_blake2_witness(*length_target, data_targets, honest_set.len(), &honest_set, &mut witness);
            witness.set_target(validator_count, F::from_canonical_usize(n));

            assert!(
                prove_satisfies(&builder.build::<C>(), witness),
                "CR-1 compat guard: honest full-sign quorum at n={n} must satisfy"
            );
        }
    }


}
