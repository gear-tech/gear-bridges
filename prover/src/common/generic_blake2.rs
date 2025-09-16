//! ### Contains circuit that's used to compute blake2 hash of generic-length data.

use lazy_static::lazy_static;
use plonky2::{
    gates::noop::NoopGate,
    iop::{
        target::Target,
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{CircuitConfig, CircuitData, VerifierOnlyCircuitData, VerifierCircuitData, ProverCircuitData},
        proof::ProofWithPublicInputs,
    },
};
use plonky2_blake2b256::circuit::{
    blake2_circuit_from_message_targets_and_length_target, BLOCK_BITS, BLOCK_BYTES,
};
use plonky2_field::types::Field;
use plonky2_u32::gates::{
    add_many_u32::{U32AddManyGate, U32AddManyGenerator},
    arithmetic_u32::{U32ArithmeticGate, U32ArithmeticGenerator},
};
use std::env;

use crate::{
    common::{
        targets::{ArrayTarget, Blake2Target, ByteTarget, TargetSet},
        ProofWithCircuitData,
    },
    prelude::*,
};

use super::pad_byte_vec;
use std::{sync::Arc, time::Instant, marker::PhantomData};


/// Maximum amount of blake2 blocks.
const MAX_BLOCK_COUNT: usize = 50;
/// Max data length that this circuit will accept.
pub const MAX_DATA_BYTES: usize = MAX_BLOCK_COUNT * BLOCK_BYTES;

impl_parsable_target_set! {
    /// Public inputs for `GenericBlake2`.
    pub struct GenericBlake2Target {
        /// It's guaranteed that padding of data will be zeroed and asserted to be equal 0.
        pub data: ArrayTarget<ByteTarget, MAX_DATA_BYTES>,
        /// Length of a useful data.
        pub length: Target,
        /// Resulting hash.
        pub hash: Blake2Target
    }
}

// Unlike `VariativeBlake2`, this circuit will have constant `VerifierOnlyCircuitData` across all
// the valid inputs.
pub struct GenericBlake2 {
    /// Data to be hashed.
    data: Vec<u8>,
}

impl GenericBlake2 {
    /// Create new `GenericBlake2` circuit.
    ///
    /// This function will statically check that `MAX_DATA_LENGTH_ESTIMATION`
    /// don't exceed `MAX_DATA_BYTES`.
    pub fn new<const MAX_DATA_LENGTH_ESTIMATION: usize>(data: Vec<u8>) -> Self {
        #[allow(clippy::let_unit_value)]
        let _ = AssertDataLengthValid::<MAX_DATA_LENGTH_ESTIMATION>::VALID;

        assert!(data.len() <= MAX_DATA_LENGTH_ESTIMATION);

        Self { data }
    }
}

struct AssertDataLengthValid<const DATA_LENGTH: usize>;

impl<const DATA_LENGTH: usize> AssertDataLengthValid<DATA_LENGTH> {
    const VALID: () = assert!(DATA_LENGTH <= MAX_DATA_BYTES);
}

impl GenericBlake2 {
    pub fn prove(self) -> ProofWithCircuitData<GenericBlake2Target> {
        log::trace!("GenericBlake2; prove");

        let block_count = self.data.len().div_ceil(BLOCK_BYTES).max(1);
        assert!(block_count <= MAX_BLOCK_COUNT, "block_count = {block_count}, MAX_BLOCK_COUNT = {MAX_BLOCK_COUNT}");

        let variative_proof = VariativeBlake2 { data: self.data }.prove();
        log::trace!("GenericBlake2; variative_proof is ready");

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        let block_count_target = builder.add_virtual_target();
        witness.set_target(block_count_target, F::from_canonical_usize(block_count));

        let proof_with_pis_target =
            builder.add_virtual_proof_with_pis(&variative_proof.circuit_data().common);

        let mut verifier_data_targets = VERIFIER_DATA_BY_BLOCK_COUNT
            .iter()
            .map(|verifier_data| builder.constant_verifier_data(verifier_data))
            .collect::<Vec<_>>();
        
        log::trace!("GenericBlake2; push data targets");
        for _ in verifier_data_targets.len()..verifier_data_targets.len().next_power_of_two() {
            verifier_data_targets.push(
                verifier_data_targets
                    .last()
                    .expect("VERIFIER_DATA_BY_BLOCK_COUNT must be >= 1")
                    .clone(),
            );
        }


        log::trace!("GenericBlake2; verify_proof");
        // It's ok not to check `verifier_data_idx` range as `GenericBlake2` just exposes all the
        // public inputs of `VariativeBlake2`, so we need to check just that it's contained in
        // pre-computed verifier data array. All the other assertions must be performed in
        // `VariativeBlake2`.
        let verifier_data_idx = builder.add_const(block_count_target, F::NEG_ONE);
        let verifier_data_target =
            builder.random_access_verifier_data(verifier_data_idx, verifier_data_targets);

        witness.set_proof_with_pis_target(&proof_with_pis_target, &variative_proof.proof());
        builder.verify_proof::<C>(
            &proof_with_pis_target,
            &verifier_data_target,
            &variative_proof.circuit_data().common,
        );

        log::trace!("GenericBlake2; VariativeBlake2Target::parse_exact(");
        let inner_pis = VariativeBlake2Target::parse_exact(
            &mut proof_with_pis_target.public_inputs.into_iter(),
        );

        GenericBlake2Target {
            data: inner_pis.data,
            length: inner_pis.length,
            hash: inner_pis.hash,
        }
        .register_as_public_inputs(&mut builder);

        log::trace!("GenericBlake2; ProofWithCircuitData::prove_from_builder(builder, witness)");
        ProofWithCircuitData::prove_from_builder(builder, witness)
    }
}

lazy_static! {
    /// Cached `VerifierOnlyCircuitData`s, each corresponding to a specific blake2 block count.
    static ref VERIFIER_DATA_BY_BLOCK_COUNT: [VerifierOnlyCircuitData<C, D>; MAX_BLOCK_COUNT] = {
        let path = env::var("PATH_DATA").expect("PATH_DATA");

        let mut verifier_data = Vec::with_capacity(MAX_BLOCK_COUNT);
        let serializer_gate = CustomGateSerializer::default();

        for i in 1..=MAX_BLOCK_COUNT {
            let serialized = std::fs::read(format!("{path}/verifier_circuit_data-{i}")).expect("Good file with serialized data");
            let data = VerifierCircuitData::<F, C, D>::from_bytes(serialized, &serializer_gate).expect("Correctly formed serialized data");
            verifier_data.push(data.verifier_only);
        }

        verifier_data
            .try_into()
            .expect("Correct max block count")
    };
}

// lazy_static! {
//     static ref PROVER_DATA_BY_BLOCK_COUNT: [(ProverCircuitData<F, C, D>, Vec<Target>); MAX_BLOCK_COUNT] = {
//         let path = env::var("PATH_DATA").expect("PATH_DATA");

//         let mut result = Vec::with_capacity(MAX_BLOCK_COUNT);
//         let serializer_gate = CustomGateSerializer::default();
//         let serializer_generator = CustomGeneratorSerializer::<C, D>::default();

//         for i in 1..=MAX_BLOCK_COUNT {
//             let serialized = std::fs::read(format!("{path}/prover_circuit_data-{i}")).expect("Good file with serialized data");
//             let data = ProverCircuitData::<F, C, D>::from_bytes(&serialized, &serializer_gate, &serializer_generator).expect("Correctly formed serialized data");

//             let serialized = std::fs::read(format!("{path}/prover_circuit_data-targets-{i}")).expect("Good file with serialized data");
//             let mut buffer_read = Buffer::new(&serialized[..]);
//             let targets = buffer_read.read_target_vec()
//                 .expect("buffer_read.read_target_vec()");

//             result.push((data, targets));
//         }

//         match result
//             .try_into()
//             {
//                 Ok(r) => r,
//                 Err(_) => panic!("size is correct"),
//             }
//     };
// }

fn blake2_circuit_verifier_data(num_blocks: usize) -> VerifierOnlyCircuitData<C, D> {
    VariativeBlake2 {
        data: vec![0; BLOCK_BYTES * num_blocks],
    }
    .prove()
    .circuit_data()
    .verifier_only
    .clone()
}

impl_target_set! {
    struct VariativeBlake2Target {
        data: ArrayTarget<ByteTarget, MAX_DATA_BYTES>,
        length: Target,
        hash: Blake2Target
    }
}

/// Inner circuit that will have different `VerifierOnlyCircuitData` for each block count.
/// This circuit asserts that data padding is zeroed(it applies to targets, not the `data` field).
struct VariativeBlake2 {
    data: Vec<u8>,
}

impl VariativeBlake2 {
    pub fn prove(self) -> ProofWithCircuitData<VariativeBlake2Target> {
        log::trace!("VariativeBlake2; fn prove; self.data.len = {}", self.data.len());

        // *if self.data.len() <= BLOCK_BYTES
        {
            let index = self.data.len().div_ceil(BLOCK_BYTES).max(1);
            log::debug!("index = {index}, self.data.len() = {}", self.data.len());

            let path = env::var("PATH_DATA").expect("PATH_DATA");
            let serializer_gate = CustomGateSerializer::default();
            let serializer_generator = CustomGeneratorSerializer::<C, D>::default();

            let now = Instant::now();
            let prover_data = {
                let file = std::fs::OpenOptions::new().read(true).open(format!("{path}/prover_circuit_data-{index}")).expect("Open a file with correctly formed data for read");
                let mut reader = std::io::BufReader::with_capacity(get_buffer_size(), file);

                let mut read_adapter = ReadAdapter::new(reader, None);

                read_adapter.
                    read_prover_circuit_data::<F, C, D>(
                        &serializer_gate,
                        &serializer_generator).expect("Correctly formed serialized data")
            };

            log::info!("Loading circuit data directly time: {}ms", now.elapsed().as_millis());

            let now = Instant::now();

            let serialized = std::fs::read(format!("{path}/prover_circuit_data-targets-{index}")).expect("Good file with serialized data");
            let mut buffer_read = Buffer::new(&serialized[..]);
            let targets = buffer_read.read_target_vec()
                .expect("buffer_read.read_target_vec()");

            log::info!("Loading circuit data time: {}ms", now.elapsed().as_millis());

            // let (prover_data, targets) = &PROVER_DATA_BY_BLOCK_COUNT[index];

            let mut witness = PartialWitness::new();
            witness.set_target(targets[0], F::from_canonical_usize(self.data.len()));
            for i in 0..self.data.len() {
                witness.set_target(targets[i + 1], F::from_canonical_u8(self.data[i]));
            }
            // zero the remaining tail
            for i in (1 + self.data.len())..targets.len() {
                witness.set_target(targets[i], F::from_canonical_u8(0));
            }

            let now = Instant::now();

            let ProofWithPublicInputs {
                proof,
                public_inputs,
            } = prover_data.prove(witness).unwrap();

            log::info!("VariativeBlake2 prove time: {}ms", now.elapsed().as_millis());

            return ProofWithCircuitData {
                proof,
                circuit_data: Arc::from(VerifierCircuitData {
                    verifier_only: VERIFIER_DATA_BY_BLOCK_COUNT[index - 1].clone(),
                    common: prover_data.common.clone(),
                }),
                public_inputs,
                public_inputs_parser: PhantomData,
            };
        }

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);
        let mut witness = PartialWitness::new();

        log::trace!("VariativeBlake2; 111");

        let block_count = self.data.len().div_ceil(BLOCK_BYTES).max(1);

        let length_target = builder.add_virtual_target();
        witness.set_target(length_target, F::from_canonical_usize(self.data.len()));

        log::trace!("VariativeBlake2; 222");

        let data_target: [ByteTarget; MAX_DATA_BYTES] = pad_byte_vec(self.data).map(|byte| {
            let target = builder.add_virtual_target();
            witness.set_target(target, F::from_canonical_u8(byte));
            ByteTarget::from_target_safe(target, &mut builder)
        });

        log::trace!("VariativeBlake2; 333");

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

        log::trace!("VariativeBlake2; 444");

        // Assert upper bound for length.
        let length_is_max = builder.is_equal(current_idx, length_target);
        let length_valid = builder.or(length_is_max, data_end);
        builder.assert_one(length_valid.target);

        log::trace!("VariativeBlake2; 555");

        // Assert lower bound for length.
        let max_length = builder.constant(F::from_canonical_usize(block_count * BLOCK_BYTES));
        let padded_length = builder.sub(max_length, length_target);
        let block_bytes_target = builder.constant(F::from_canonical_usize(BLOCK_BYTES));
        let compare_with_zero = builder.sub(block_bytes_target, padded_length);
        builder.range_check(compare_with_zero, 32);

        log::trace!("VariativeBlake2; 666");

        let data_target = ArrayTarget(data_target);
        let data_target_bits = data_target
            .0
            .iter()
            .flat_map(|t| t.as_bit_targets(&mut builder).0.into_iter().rev());

        log::trace!("VariativeBlake2; 777");

        let hasher_input = data_target_bits
            .take(BLOCK_BITS * block_count)
            .collect::<Vec<_>>();

        log::trace!("VariativeBlake2; 888");

        let hash = blake2_circuit_from_message_targets_and_length_target(
            &mut builder,
            hasher_input,
            length_target,
        );
        let hash = Blake2Target::parse_exact(&mut hash.into_iter().map(|t| t.target));

        log::trace!("VariativeBlake2; 999");

        VariativeBlake2Target {
            data: data_target,
            length: length_target,
            hash,
        }
        .register_as_public_inputs(&mut builder);

        log::trace!("VariativeBlake2; 10 builder.num_gates() = {}", builder.num_gates());

        // Standardize degree.
        // (2^19 + 2^17)
        while builder.num_gates() < 655_360 {
            builder.add_gate(NoopGate, vec![]);
        }

        log::trace!("VariativeBlake2; 11");

        let result = ProofWithCircuitData::prove_from_builder(builder, witness);

        log::trace!("VariativeBlake2; 12");

        result
    }

    pub fn build_circuit_data(&self) -> (CircuitData<F, C, D>, Vec<Target>) {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::new(config);

        let block_count = self.data.len().div_ceil(BLOCK_BYTES).max(1);

        let length_target = builder.add_virtual_target();

        let mut targets = Vec::with_capacity(MAX_DATA_BYTES + 1);
        targets.push(length_target);

        let data_target: [ByteTarget; MAX_DATA_BYTES] = pad_byte_vec(self.data.clone()).map(|_byte| {
            let target = builder.add_virtual_target();
            targets.push(target);

            ByteTarget::from_target_safe(target, &mut builder)
        });

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

        log::trace!("VariativeBlake2; 10 builder.num_gates() = {}", builder.num_gates());

        // Standardize degree.
        // (2^19 + 2^17)
        while builder.num_gates() < 655_360 {
            builder.add_gate(NoopGate, vec![]);
        }

        (builder.build::<C>(), targets)
    }
}

use plonky2::{get_generator_tag_impl, impl_generator_serializer, read_generator_impl, read_gate_impl, get_gate_tag_impl, impl_gate_serializer};
use plonky2::plonk::config::GenericConfig;
use plonky2::util::serialization::{WitnessGeneratorSerializer, GateSerializer, Write as _, Read, Buffer};
use plonky2::hash::hash_types::RichField;
use plonky2_field::extension::Extendable;
use plonky2::plonk::config::AlgebraicHasher;
use plonky2::gadgets::split_join::WireSplitGenerator;
use plonky2::gadgets::split_join::SplitGenerator;
use plonky2::gates::reducing::ReducingGenerator;
use plonky2::gates::reducing_extension::ReducingGenerator as ReducingExtensionGenerator;
use plonky2::iop::generator::RandomValueGenerator;
use plonky2::gates::random_access::RandomAccessGenerator;
use plonky2::gadgets::arithmetic_extension::QuotientGeneratorExtension;
use plonky2::gates::poseidon_mds::PoseidonMdsGenerator;
use plonky2::gates::poseidon::PoseidonGenerator;
use plonky2::iop::generator::NonzeroTestGenerator;
use plonky2::gates::multiplication_extension::MulExtensionGenerator;
use plonky2::gadgets::range_check::LowHighGenerator;
use plonky2::gates::lookup_table::LookupTableGenerator;
use plonky2::gates::lookup::LookupGenerator;
use plonky2::gates::coset_interpolation::InterpolationGenerator;
use plonky2::gates::exponentiation::ExponentiationGenerator;
use plonky2::gadgets::arithmetic::EqualityGenerator;
use plonky2::recursion::dummy_circuit::DummyProofGenerator;
use plonky2::iop::generator::CopyGenerator;
use plonky2::iop::generator::ConstantGenerator;
use plonky2::gadgets::split_base::BaseSumGenerator;
use plonky2::gates::base_sum::BaseSplitGenerator;
use plonky2::gates::arithmetic_extension::ArithmeticExtensionGenerator;
use plonky2::gates::arithmetic_base::ArithmeticBaseGenerator;
use plonky2::gates::reducing::ReducingGate;
use plonky2::gates::reducing_extension::ReducingExtensionGate;
use plonky2::gates::random_access::RandomAccessGate;
use plonky2::gates::public_input::PublicInputGate;
use plonky2::gates::poseidon::PoseidonGate;
use plonky2::gates::poseidon_mds::PoseidonMdsGate;
use plonky2::gates::multiplication_extension::MulExtensionGate;
use plonky2::gates::lookup_table::LookupTableGate;
use plonky2::gates::lookup::LookupGate;
use plonky2::gates::exponentiation::ExponentiationGate;
use plonky2::gates::coset_interpolation::CosetInterpolationGate;
use plonky2::gates::constant::ConstantGate;
use plonky2::gates::base_sum::BaseSumGate;
use plonky2::gates::arithmetic_extension::ArithmeticExtensionGate;
use plonky2::gates::arithmetic_base::ArithmeticGate;

#[derive(Debug, Default)]
pub struct CustomGeneratorSerializer<C: GenericConfig<D>, const D: usize> {
    pub _phantom: PhantomData<C>,
}

impl<F, C, const D: usize> WitnessGeneratorSerializer<F, D> for CustomGeneratorSerializer<C, D>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F> + 'static,
    C::Hasher: AlgebraicHasher<F>,
{
    impl_generator_serializer! {
        CustomGeneratorSerializer,
        ArithmeticBaseGenerator<F, D>,
        ArithmeticExtensionGenerator<F, D>,
        BaseSplitGenerator<2>,
        BaseSumGenerator<2>,
        ConstantGenerator<F>,
        CopyGenerator,
        DummyProofGenerator<F, C, D>,
        EqualityGenerator,
        ExponentiationGenerator<F, D>,
        InterpolationGenerator<F, D>,
        LookupGenerator,
        LookupTableGenerator,
        LowHighGenerator,
        MulExtensionGenerator<F, D>,
        NonzeroTestGenerator,
        PoseidonGenerator<F, D>,
        PoseidonMdsGenerator<D>,
        QuotientGeneratorExtension<D>,
        RandomAccessGenerator<F, D>,
        RandomValueGenerator,
        ReducingGenerator<D>,
        ReducingExtensionGenerator<D>,
        SplitGenerator,
        WireSplitGenerator,
        U32AddManyGenerator<F, D>,
        U32ArithmeticGenerator<F, D>
    }
}

#[derive(Debug, Default)]
pub struct CustomGateSerializer;

impl<F: RichField + Extendable<D>, const D: usize> GateSerializer<F, D> for CustomGateSerializer {
    impl_gate_serializer! {
        CustomGateSerializer,
        ArithmeticGate,
        ArithmeticExtensionGate<D>,
        BaseSumGate<2>,
        ConstantGate,
        CosetInterpolationGate<F, D>,
        ExponentiationGate<F, D>,
        LookupGate,
        LookupTableGate,
        MulExtensionGate<D>,
        NoopGate,
        PoseidonMdsGate<F, D>,
        PoseidonGate<F, D>,
        PublicInputGate,
        RandomAccessGate<F, D>,
        ReducingExtensionGate<D>,
        ReducingGate<D>,
        U32AddManyGate<F, D>,
        U32ArithmeticGate<F, D>
    }
}

use plonky2::util::serialization::IoResult;
use plonky2::plonk::circuit_data::CommonCircuitData;
use plonky2::gates::gate::GateRef;
use plonky2::iop::generator::WitnessGeneratorRef;
use plonky2::util::serialization::Remaining;

pub struct ReadAdapter<Reader: std::io::Read + std::io::Seek> {
    pub reader: Reader,
    pub buffer: Vec<u8>,
}

impl<Reader: std::io::Read + std::io::Seek> ReadAdapter<Reader> {
    pub fn new(reader: Reader, size: Option<usize>) -> Self {
        Self {
            reader,
            buffer: vec![0; size.unwrap_or(1_024)],
        }
    }
}

impl<Reader: std::io::Read + std::io::Seek> plonky2::util::serialization::Read for ReadAdapter<Reader> {
    fn read_exact(&mut self, bytes: &mut [u8]) -> IoResult<()> {
        // log::trace!("self.read_exact(bytes) entered");

        self.reader.read_exact(bytes)
            .map_err(|e| {
                log::trace!("self.read_exact(bytes) failed: {e:?}");

                plonky2::util::serialization::IoError
            })
    }

    fn read_gate<F: RichField + Extendable<D>, const D: usize>(
        &mut self,
        gate_serializer: &dyn GateSerializer<F, D>,
        common_data: &CommonCircuitData<F, D>,
    ) -> IoResult<GateRef<F, D>> {
        // log::trace!("fn read_gate<F: RichField + Extendable<D>, const D: usize>( entered");

        let bytes_read = self.reader.read(&mut self.buffer)
            .map_err(|e| {
                log::trace!("self.reader.read(&mut buffer) failed: {e:?}");

                plonky2::util::serialization::IoError
            })?;
        // log::trace!("fn read_gate<F: RichField + Extendable<D>, const D: usize>( bytes_read = {bytes_read}");

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        match gate_serializer.read_gate(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("gate_serializer.read_gate(&mut buffer, common_data) failed: {e:?}");
            })
        {
            Ok(result) => {
                // log::trace!("fn read_generator<F: RichField + Extendable<D>, const D: usize>( remaining = {}", buffer_plonky.remaining());

                self.reader.seek_relative(-(buffer_plonky.remaining() as i64))
                    .map_err(|e| {
                        log::trace!("self.reader.seek_relative(-(buffer.remaining() as i64)) failed: {e:?}");

                        plonky2::util::serialization::IoError
                    })?;

                return Ok(result);
            }

            // implementation of Read may be buffered so we have to try to read next buffer and
            // deserialize
            Err(e) if bytes_read < self.buffer.len() => {}

            result => return result,
        }

        let bytes_read = bytes_read + self.reader.read(&mut self.buffer[bytes_read..])
            .map_err(|e| {
                log::trace!("self.reader.read(&mut buffer[bytes_read]) failed: {e:?}");

                plonky2::util::serialization::IoError
            })?;
        // log::trace!("fn read_gate<F: RichField + Extendable<D>, const D: usize>( bytes_read = {bytes_read}");

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        let result = gate_serializer.read_gate(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("gate_serializer.read_gate(&mut buffer, common_data) failed: {e:?}");
            })?;
            
        // log::trace!("fn read_generator<F: RichField + Extendable<D>, const D: usize>( remaining = {}", buffer_plonky.remaining());

        self.reader.seek_relative(-(buffer_plonky.remaining() as i64))
            .map_err(|e| {
                log::trace!("self.reader.seek_relative(-(buffer.remaining() as i64)) failed: {e:?}");

                plonky2::util::serialization::IoError
            })?;

        Ok(result)
    }

    fn read_generator<F: RichField + Extendable<D>, const D: usize>(
        &mut self,
        generator_serializer: &dyn WitnessGeneratorSerializer<F, D>,
        common_data: &CommonCircuitData<F, D>,
    ) -> IoResult<WitnessGeneratorRef<F, D>> {
        // log::trace!("fn read_generator<F: RichField + Extendable<D>, const D: usize>( entered");

        let bytes_read = self.reader.read(&mut self.buffer)
            .map_err(|e| {
                log::trace!("self.reader.read(&mut buffer) failed: {e:?}");

                plonky2::util::serialization::IoError
            })?;
        // log::trace!("fn read_generator<F: RichField + Extendable<D>, const D: usize>( bytes_read = {bytes_read}");

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        match generator_serializer.read_generator(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("generator_serializer.read_generator(&mut buffer_plonky, common_data) failed: {e:?}");
            })
        {
            Ok(result) => {
                // log::trace!("fn read_generator<F: RichField + Extendable<D>, const D: usize>( remaining = {}", buffer_plonky.remaining());

                self.reader.seek_relative(-(buffer_plonky.remaining() as i64))
                    .map_err(|e| {
                        log::trace!("self.reader.seek_relative(-(buffer.remaining() as i64)) failed: {e:?}");

                        plonky2::util::serialization::IoError
                    })?;

                return Ok(result);
            }

            // implementation of Read may be buffered so we have to try to read next buffer and
            // deserialize
            Err(e) if bytes_read < self.buffer.len() => {}

            result => return result,
        }

        let bytes_read = bytes_read + self.reader.read(&mut self.buffer[bytes_read..])
            .map_err(|e| {
                log::trace!("self.reader.read(&mut buffer[bytes_read]) failed: {e:?}");

                plonky2::util::serialization::IoError
            })?;
        // log::trace!("fn read_gate<F: RichField + Extendable<D>, const D: usize>( bytes_read = {bytes_read}");

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        let result = generator_serializer.read_generator(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("gate_serializer.read_gate(&mut buffer, common_data) failed: {e:?}");
            })?;
            
        // log::trace!("fn read_generator<F: RichField + Extendable<D>, const D: usize>( remaining = {}", buffer_plonky.remaining());

        self.reader.seek_relative(-(buffer_plonky.remaining() as i64))
            .map_err(|e| {
                log::trace!("self.reader.seek_relative(-(buffer.remaining() as i64)) failed: {e:?}");

                plonky2::util::serialization::IoError
            })?;

        Ok(result)
    }
}

    fn get_buffer_size() -> usize {
        // 8KiB
        const DEFAULT: usize = 8_192;

        let Ok(buffer_size) = std::env::var("BUFFER_SIZE") else {
            log::debug!(r#""BUFFER_SIZE" is not set. Use default value ({DEFAULT})."#);

            return DEFAULT;
        };

        let Ok(buffer_size) = buffer_size.parse::<usize>() else {
            log::debug!(r#""BUFFER_SIZE" is not a number. Use default value ({DEFAULT})."#);

            return DEFAULT;
        };

        buffer_size
    }

#[cfg(test)]
mod tests {
    use blake2::{
        digest::{Update, VariableOutput},
        Blake2bVar,
    };
    use plonky2::plonk::circuit_data::VerifierCircuitData;

    use super::*;
    use crate::common::{array_to_bits, targets::ParsableTargetSet};
    use std::{env, fs};

    #[test]
    fn generate_circuit_data() {
        let _ = pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Info)
            .format_target(false)
            .format_timestamp_secs()
            .parse_default_env()
            .try_init();

        let path_out = env::var("PATH_OUT").expect("PATH_OUT");

        let mut buffer = Vec::with_capacity(2_000_000_000);
        for count_block in 1..=MAX_BLOCK_COUNT {
            let (circuit_data, targets) = VariativeBlake2 { data: vec![0; count_block * BLOCK_BYTES ] }.build_circuit_data();

            let serializer_gate = CustomGateSerializer::default();
            let path_verifier_circuit_data = format!("{path_out}/verifier_circuit_data-{count_block}");
            let serialized = circuit_data.verifier_data().to_bytes(&serializer_gate)
                .expect("circuit_data.verifier_data().to_bytes(&serializer_gate)");
            fs::write(path_verifier_circuit_data, serialized).expect("fs::write(path_verifier_circuit_data, serialized)");

            let serializer_generator = CustomGeneratorSerializer::<C, D>::default();
            let prover_data = circuit_data.prover_data();
            buffer.clear();
            buffer.write_prover_circuit_data(
                &prover_data,
                &serializer_gate,
                &serializer_generator,
            ).expect("write_prover_circuit_data");
            let path_prover_circuit_data = format!("{path_out}/prover_circuit_data-{count_block}");
            fs::write(path_prover_circuit_data, &buffer).expect("fs::write(path_prover_circuit_data, buffer)");

            buffer.clear();
            buffer.write_target_vec(&targets).expect("buffer.write_target_vec(&targets)");
            let path = format!("{path_out}/prover_circuit_data-targets-{count_block}");
            fs::write(path, &buffer).expect("fs::write(path_prover_circuit_data, buffer)");
        }
    }

    #[test]
    fn test_generic_blake2_hasher() {
        let _ = pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Info)
            .format_target(false)
            .format_timestamp_secs()
            .parse_default_env()
            .try_init();

        let test_data = vec![
            vec![0],
            vec![],
            vec![1, 3, 7, 11, 200, 103, 255, 0, 11],
            vec![10; BLOCK_BYTES - 1],
            vec![10; BLOCK_BYTES],
            vec![10; BLOCK_BYTES + 1],
            vec![0xA; BLOCK_BYTES * MAX_BLOCK_COUNT - 1],
            vec![0xA; BLOCK_BYTES * MAX_BLOCK_COUNT],
        ];

        let verifier_data = test_data.into_iter().map(test_case).collect::<Vec<_>>();

        for i in 1..verifier_data.len() {
            assert_eq!(
                verifier_data[i - 1],
                verifier_data[i],
                "Verifier data at {} and {} don't match",
                i - 1,
                i
            );
        }
    }

    fn test_case(data: Vec<u8>) -> VerifierCircuitData<F, C, D> {
        log::debug!("test_case; data.len = {}", data.len());

        let mut hasher = Blake2bVar::new(32).expect("Blake2bVar instantiated");
        hasher.update(&data);
        let mut real_hash = [0; 32];
        hasher
            .finalize_variable(&mut real_hash)
            .expect("Hash of correct length");

        let proof = GenericBlake2 { data: data.clone() }.prove();
        let public_inputs =
            GenericBlake2Target::parse_public_inputs_exact(&mut proof.public_inputs().into_iter());

        assert_eq!(public_inputs.hash.to_vec(), array_to_bits(&real_hash));
        assert_eq!(public_inputs.length as usize, data.len());
        assert_eq!(&public_inputs.data[..data.len()], &data[..]);

        proof.circuit_data().clone()
    }

    fn get_index() -> usize {
        const DEFAULT: usize = 1;

        let Ok(buffer_size) = std::env::var("INDEX") else {
            log::debug!(r#""INDEX" is not set. Use default value ({DEFAULT})."#);

            return DEFAULT;
        };

        let Ok(buffer_size) = buffer_size.parse::<usize>() else {
            log::debug!(r#""INDEX" is not a number. Use default value ({DEFAULT})."#);

            return DEFAULT;
        };

        buffer_size
    }

    #[test]
    fn bench_deserialization() {
        use std::io::prelude::*;
        use std::io::BufReader;
        use std::fs::File;
        use std::fs::OpenOptions;

        let _ = pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Info)
            .format_target(false)
            .format_timestamp_secs()
            .parse_default_env()
            .try_init();

        let path = env::var("PATH_DATA").expect("PATH_DATA");
        log::debug!(r#"bench_deserialization started; PATH_DATA = "{path}""#);

        let buffer_size = get_buffer_size();

        let serializer_gate = CustomGateSerializer::default();
        let serializer_generator = CustomGeneratorSerializer::<C, D>::default();

        for index in 1..=MAX_BLOCK_COUNT {
            {
                let path = format!("{path}/verifier_circuit_data-{index}");
                log::trace!(r#"path = "{path}""#);

                let file = OpenOptions::new().read(true).open(path).expect("Open a file with correctly formed data for read");
                let mut reader = BufReader::with_capacity(buffer_size, file);

                let mut read_adapter = ReadAdapter::new(reader, None);

                let _data = read_adapter.
                    read_verifier_circuit_data::<F, C, D>(&serializer_gate).expect("Correctly formed serialized data");
            }

            {
                let path = format!("{path}/verifier_only_circuit_data-{index}");
                log::trace!(r#"path = "{path}""#);

                let file = OpenOptions::new().read(true).open(path).expect("Open a file with correctly formed data for read");
                let mut reader = BufReader::with_capacity(buffer_size, file);

                let mut read_adapter = ReadAdapter::new(reader, None);

                let _data = read_adapter.
                    read_verifier_only_circuit_data::<F, C, D>().expect("Correctly formed serialized data");
            }

            let path = format!("{path}/prover_circuit_data-{index}");
            log::trace!(r#"path = "{path}""#);

            let file = OpenOptions::new().read(true).open(path).expect("Open a file with correctly formed data for read");
            let mut reader = BufReader::with_capacity(buffer_size, file);

            let mut read_adapter = ReadAdapter::new(reader, None);

            let now = Instant::now();

            let prover_data = read_adapter.
                read_prover_circuit_data::<F, C, D>(
                    &serializer_gate,
                    &serializer_generator).expect("Correctly formed serialized data");

            log::info!("Loading circuit data time: {}ms", now.elapsed().as_millis());
        }
    }

    #[test]
    fn bench_deserialization2() {
        use std::io::prelude::*;
        use std::io::BufReader;
        use std::fs::File;
        use std::fs::OpenOptions;

        let _ = pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Info)
            .format_target(false)
            .format_timestamp_secs()
            .parse_default_env()
            .try_init();

        let path = env::var("PATH_DATA").expect("PATH_DATA");
        log::debug!(r#"bench_deserialization started; PATH_DATA = "{path}""#);

        let buffer_size = get_buffer_size();

        let serializer_gate = CustomGateSerializer::default();
        let serializer_generator = CustomGeneratorSerializer::<C, D>::default();

        let index = get_index();

        let path = format!("{path}/prover_circuit_data-{index}");
        log::trace!(r#"path = "{path}""#);

        {
            let now = Instant::now();

            let serialized = std::fs::read(&path).expect("Good file with serialized data");
            let prover_data = ProverCircuitData::<F, C, D>::from_bytes(&serialized, &serializer_gate, &serializer_generator).expect("Correctly formed serialized data");

            log::info!("Loading circuit data time: {}ms", now.elapsed().as_millis());
        }

        let file = OpenOptions::new().read(true).open(path).expect("Open a file with correctly formed data for read");
        let mut reader = BufReader::with_capacity(buffer_size, file);

        let mut read_adapter = ReadAdapter::new(reader, None);

        let now = Instant::now();

        let prover_data = read_adapter.
            read_prover_circuit_data::<F, C, D>(
                &serializer_gate,
                &serializer_generator).expect("Correctly formed serialized data");

        log::info!("Loading circuit data directly time: {}ms", now.elapsed().as_millis());
    }
}
