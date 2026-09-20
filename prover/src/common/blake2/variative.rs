use super::*;
use plonky2::plonk::circuit_data::ProverCircuitData;

struct CachedProverTargets {
    prover_data: ProverCircuitData<F, C, D>,
    targets: Vec<Target>,
    verifier_data: Arc<VerifierCircuitData<F, C, D>>,
}

fn load_cached_prover_targets(index: usize) -> CachedProverTargets {
    let path =
        env::var("VBLAKE2_CACHE_PATH").expect(r#"VariativeBlake2: "VBLAKE2_CACHE_PATH" is set"#);
    let serializer_gate = GateSerializer;
    let serializer_generator = GeneratorSerializer::<C, D>::default();

    let now = Instant::now();
    let prover_data = {
        let file = fs::OpenOptions::new()
            .read(true)
            .open(format!("{path}/prover_circuit_data-{index}"))
            .expect("VariativeBlake2: open a file with correctly formed data for read");
        let reader = io::BufReader::with_capacity(*BUFFER_SIZE, file);

        let mut read_adapter = ReadAdapter::new(reader, None);

        read_adapter
            .read_prover_circuit_data::<F, C, D>(&serializer_gate, &serializer_generator)
            .expect("Correctly formed serialized data")
    };

    log::trace!(
        "Loading cached VariativeBlake2 circuit (index = {index}) time: {}ms",
        now.elapsed().as_millis()
    );

    let serialized = fs::read(format!("{path}/prover_circuit_data-targets-{index}"))
        .expect("VariativeBlake2: Good file with serialized data");
    let mut buffer_read = Buffer::new(&serialized[..]);
    let targets = buffer_read
        .read_target_vec()
        .expect("VariativeBlake2: buffer_read.read_target_vec()");

    // Clone the common data once when populating the process cache. Every
    // proof below shares this Arc rather than cloning the 655k-gate metadata.
    let verifier_data = Arc::new(VerifierCircuitData {
        verifier_only: VERIFIER_DATA_BY_BLOCK_COUNT[index - 1].clone(),
        common: prover_data.common.clone(),
    });

    CachedProverTargets {
        prover_data,
        targets,
        verifier_data,
    }
}

static CACHED_PROVER_TARGETS1: LazyLock<CachedProverTargets> =
    LazyLock::new(|| load_cached_prover_targets(1));
static CACHED_PROVER_TARGETS2: LazyLock<CachedProverTargets> =
    LazyLock::new(|| load_cached_prover_targets(2));
// Production traces show indices 3, 4, and 15 are also hot in storage and
// validator-set proofs. Each serialized prover circuit is large, so keep this
// explicit bounded set rather than retaining all 50 possible block counts.
static CACHED_PROVER_TARGETS3: LazyLock<CachedProverTargets> =
    LazyLock::new(|| load_cached_prover_targets(3));
static CACHED_PROVER_TARGETS4: LazyLock<CachedProverTargets> =
    LazyLock::new(|| load_cached_prover_targets(4));
static CACHED_PROVER_TARGETS15: LazyLock<CachedProverTargets> =
    LazyLock::new(|| load_cached_prover_targets(15));
static CACHED_PROVER_TARGETS16: LazyLock<CachedProverTargets> =
    LazyLock::new(|| load_cached_prover_targets(16));

impl_target_set! {
    pub struct VariativeBlake2Target {
        pub data: ArrayTarget<ByteTarget, MAX_DATA_BYTES>,
        pub length: Target,
        pub hash: Blake2Target
    }
}

/// Inner circuit that will have different `VerifierOnlyCircuitData` for each block count.
/// This circuit asserts that data padding is zeroed (it applies to targets, not the `data` field).
pub struct VariativeBlake2;

impl VariativeBlake2 {
    // The function uses cached circuit data from files.
    pub fn prove(data: &[u8]) -> ProofWithCircuitData<VariativeBlake2Target> {
        let index = data.len().div_ceil(BLOCK_BYTES).max(1);

        match index {
            1 => {
                let cached = &*CACHED_PROVER_TARGETS1;
                return Self::prove_inner(
                    index,
                    &cached.prover_data,
                    &cached.targets,
                    Arc::clone(&cached.verifier_data),
                    data,
                );
            }
            2 => {
                let cached = &*CACHED_PROVER_TARGETS2;
                return Self::prove_inner(
                    index,
                    &cached.prover_data,
                    &cached.targets,
                    Arc::clone(&cached.verifier_data),
                    data,
                );
            }
            3 => {
                let cached = &*CACHED_PROVER_TARGETS3;
                return Self::prove_inner(
                    index,
                    &cached.prover_data,
                    &cached.targets,
                    Arc::clone(&cached.verifier_data),
                    data,
                );
            }
            4 => {
                let cached = &*CACHED_PROVER_TARGETS4;
                return Self::prove_inner(
                    index,
                    &cached.prover_data,
                    &cached.targets,
                    Arc::clone(&cached.verifier_data),
                    data,
                );
            }
            15 => {
                let cached = &*CACHED_PROVER_TARGETS15;
                return Self::prove_inner(
                    index,
                    &cached.prover_data,
                    &cached.targets,
                    Arc::clone(&cached.verifier_data),
                    data,
                );
            }
            16 => {
                let cached = &*CACHED_PROVER_TARGETS16;
                return Self::prove_inner(
                    index,
                    &cached.prover_data,
                    &cached.targets,
                    Arc::clone(&cached.verifier_data),
                    data,
                );
            }
            _ => {}
        }

        let path = env::var("VBLAKE2_CACHE_PATH")
            .expect(r#"VariativeBlake2: "VBLAKE2_CACHE_PATH" is set"#);
        let serializer_gate = GateSerializer;
        let serializer_generator = GeneratorSerializer::<C, D>::default();

        let now = Instant::now();
        let prover_data = {
            let file = fs::OpenOptions::new()
                .read(true)
                .open(format!("{path}/prover_circuit_data-{index}"))
                .expect("VariativeBlake2: open a file with correctly formed data for read");
            let reader = io::BufReader::with_capacity(*BUFFER_SIZE, file);

            let mut read_adapter = ReadAdapter::new(reader, None);

            read_adapter
                .read_prover_circuit_data::<F, C, D>(&serializer_gate, &serializer_generator)
                .expect("Correctly formed serialized data")
        };

        log::trace!(
            "Loading circuit data (index = {index}) directly time: {}ms",
            now.elapsed().as_millis()
        );

        let serialized = fs::read(format!("{path}/prover_circuit_data-targets-{index}"))
            .expect("VariativeBlake2: Good file with serialized data");
        let mut buffer_read = Buffer::new(&serialized[..]);
        let targets = buffer_read
            .read_target_vec()
            .expect("VariativeBlake2: buffer_read.read_target_vec()");

        let verifier_data = Arc::new(VerifierCircuitData {
            verifier_only: VERIFIER_DATA_BY_BLOCK_COUNT[index - 1].clone(),
            common: prover_data.common.clone(),
        });

        Self::prove_inner(index, &prover_data, &targets, verifier_data, data)
    }

    fn prove_inner(
        _index: usize,
        prover_data: &ProverCircuitData<F, C, D>,
        targets: &[Target],
        verifier_data: Arc<VerifierCircuitData<F, C, D>>,
        data: &[u8],
    ) -> ProofWithCircuitData<VariativeBlake2Target> {
        let witness = Self::set_witness(targets, data);

        let now = Instant::now();

        let ProofWithPublicInputs {
            proof,
            public_inputs,
        } = prover_data.prove(witness).unwrap();

        log::trace!(
            "VariativeBlake2 prove time: {}ms",
            now.elapsed().as_millis()
        );

        ProofWithCircuitData::from_proof_and_shared_circuit_data(
            ProofWithPublicInputs {
                proof,
                public_inputs,
            },
            verifier_data,
        )
    }

    #[allow(dead_code)]
    pub fn prove_non_cached(data: &[u8]) -> ProofWithCircuitData<VariativeBlake2Target> {
        let (mut builder, targets) = Self::create_builder_targets(data.len());

        // Standardize degree.
        while builder.num_gates() < NUM_GATES {
            builder.add_gate(NoopGate, vec![]);
        }

        let witness = Self::set_witness(&targets, data);

        ProofWithCircuitData::prove_from_builder(builder, witness)
    }

    fn set_witness(targets: &[Target], data: &[u8]) -> PartialWitness<F> {
        assert!(targets.len() > data.len());

        let mut witness = PartialWitness::new();
        witness.set_target(targets[0], F::from_canonical_usize(data.len()));

        for i in 0..data.len() {
            witness.set_target(targets[i + 1], F::from_canonical_u8(data[i]));
        }
        // zero the remaining tail
        for target in targets.iter().skip(1 + data.len()) {
            witness.set_target(*target, F::from_canonical_u8(0));
        }

        witness
    }

    pub fn create_builder_targets(len: usize) -> (CircuitBuilder<F, D>, Vec<Target>) {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);

        let block_count = len.div_ceil(BLOCK_BYTES).max(1);

        let length_target = builder.add_virtual_target();

        let mut targets = Vec::with_capacity(MAX_DATA_BYTES + 1);
        targets.push(length_target);

        let data_target: [ByteTarget; MAX_DATA_BYTES] = iter::repeat_n((), MAX_DATA_BYTES)
            .map(|_| {
                let target = builder.add_virtual_target();
                targets.push(target);

                ByteTarget::from_target_safe(target, &mut builder)
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("Size of the vec is correct");

        // Assert that padding is zeroed.
        let mut data_end = builder._false();
        let mut current_idx = builder.zero();
        let zero = builder.zero();
        for byte in data_target.iter().take(block_count * BLOCK_BYTES) {
            let len_exceeded = builder.is_equal(current_idx, length_target);
            data_end = builder.or(len_exceeded, data_end);

            let byte_is_zero = builder.is_equal(byte.as_target(), zero);
            let byte_is_not_zero = builder.not(byte_is_zero);

            let byte_invalid = builder.and(data_end, byte_is_not_zero);
            builder.assert_zero(byte_invalid.target);

            current_idx = builder.add_const(current_idx, F::ONE);
        }

        // Assert upper bound for length.
        let length_is_max = builder.is_equal(current_idx, length_target);
        let length_valid = builder.or(length_is_max, data_end);
        builder.assert_one(length_valid.target);

        // Assert lower bound for length.
        let max_length = builder.constant(F::from_canonical_usize(block_count * BLOCK_BYTES));
        let padded_length = builder.sub(max_length, length_target);
        let block_bytes_target = builder.constant(F::from_canonical_usize(BLOCK_BYTES));
        let compare_with_zero = builder.sub(block_bytes_target, padded_length);
        builder.range_check(compare_with_zero, 32);

        let data_target = ArrayTarget(data_target);
        let data_target_bits = data_target
            .0
            .iter()
            .flat_map(|t| t.as_bit_targets(&mut builder).0.into_iter().rev());

        let hasher_input = data_target_bits
            .take(BLOCK_BITS * block_count)
            .collect::<Vec<_>>();

        let hash = blake2_circuit_from_message_targets_and_length_target(
            &mut builder,
            hasher_input,
            length_target,
        );
        let hash = Blake2Target::parse_exact(&mut hash.into_iter().map(|t| t.target));

        VariativeBlake2Target {
            data: data_target,
            length: length_target,
            hash,
        }
        .register_as_public_inputs(&mut builder);

        (builder, targets)
    }
}
