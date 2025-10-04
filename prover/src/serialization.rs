use plonky2::{
    gadgets::{
        arithmetic::EqualityGenerator,
        arithmetic_extension::QuotientGeneratorExtension,
        range_check::LowHighGenerator,
        split_base::BaseSumGenerator,
        split_join::{SplitGenerator, WireSplitGenerator},
    },
    gates::{
        arithmetic_base::{ArithmeticBaseGenerator, ArithmeticGate},
        arithmetic_extension::{ArithmeticExtensionGate, ArithmeticExtensionGenerator},
        base_sum::{BaseSplitGenerator, BaseSumGate},
        constant::ConstantGate,
        coset_interpolation::{CosetInterpolationGate, InterpolationGenerator},
        exponentiation::{ExponentiationGate, ExponentiationGenerator},
        gate::GateRef,
        lookup::{LookupGate, LookupGenerator},
        lookup_table::{LookupTableGate, LookupTableGenerator},
        multiplication_extension::{MulExtensionGate, MulExtensionGenerator},
        noop::NoopGate,
        poseidon::{PoseidonGate, PoseidonGenerator},
        poseidon_mds::{PoseidonMdsGate, PoseidonMdsGenerator},
        public_input::PublicInputGate,
        random_access::{RandomAccessGate, RandomAccessGenerator},
        reducing::{ReducingGate, ReducingGenerator},
        reducing_extension::{
            ReducingExtensionGate, ReducingGenerator as ReducingExtensionGenerator,
        },
    },
    get_gate_tag_impl, get_generator_tag_impl,
    hash::hash_types::RichField,
    impl_gate_serializer, impl_generator_serializer,
    iop::generator::{
        ConstantGenerator, CopyGenerator, NonzeroTestGenerator, RandomValueGenerator,
        WitnessGeneratorRef,
    },
    plonk::{
        circuit_data::CommonCircuitData,
        config::{AlgebraicHasher, GenericConfig},
    },
    read_gate_impl, read_generator_impl,
    recursion::dummy_circuit::DummyProofGenerator,
    util::serialization::{Buffer, IoError, IoResult, Remaining, WitnessGeneratorSerializer},
};
use plonky2_field::extension::Extendable;
use plonky2_u32::gates::{
    add_many_u32::{U32AddManyGate, U32AddManyGenerator},
    arithmetic_u32::{U32ArithmeticGate, U32ArithmeticGenerator},
};
use std::{
    io::{Read, Seek},
    marker::PhantomData,
};

/// The same as `plonky2::util::serialization::generator_serialization::default::DefaultGeneratorSerializer`
/// but with added `U32AddManyGenerator` and `U32ArithmeticGenerator`.
#[derive(Debug, Default)]
pub struct GeneratorSerializer<C: GenericConfig<D>, const D: usize> {
    pub _phantom: PhantomData<C>,
}

impl<F, C, const D: usize> WitnessGeneratorSerializer<F, D> for GeneratorSerializer<C, D>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F> + 'static,
    C::Hasher: AlgebraicHasher<F>,
{
    impl_generator_serializer! {
        GeneratorSerializer,
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

/// The same as `plonky2::util::serialization::gate_serialization::default::DefaultGateSerializer`
/// but with added `U32AddManyGate` and `U32ArithmeticGate`.
#[derive(Debug, Default)]
pub struct GateSerializer;

impl<F: RichField + Extendable<D>, const D: usize>
    plonky2::util::serialization::GateSerializer<F, D> for GateSerializer
{
    impl_gate_serializer! {
        GateSerializer,
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

/// The structure adapts `plonky2::util::serialization::Read` trait
/// to any type implementing std::io::Read + std::io::Seek.
///
/// In practice it is useful to deserialize Plonky2 entities directly
/// from files.
pub struct ReadAdapter<Reader: Read + Seek> {
    pub reader: Reader,
    pub buffer: Vec<u8>,
}

impl<Reader: Read + Seek> ReadAdapter<Reader> {
    pub fn new(reader: Reader, size: Option<usize>) -> Self {
        Self {
            reader,
            buffer: vec![0; size.unwrap_or(1_024)],
        }
    }
}

impl<Reader: Read + Seek> plonky2::util::serialization::Read for ReadAdapter<Reader> {
    fn read_exact(&mut self, bytes: &mut [u8]) -> IoResult<()> {
        self.reader.read_exact(bytes).map_err(|e| {
            log::trace!("read_exact(bytes) failed: {e:?}");

            IoError
        })
    }

    fn read_gate<F: RichField + Extendable<D>, const D: usize>(
        &mut self,
        gate_serializer: &dyn plonky2::util::serialization::gate_serialization::GateSerializer<
            F,
            D,
        >,
        common_data: &CommonCircuitData<F, D>,
    ) -> IoResult<GateRef<F, D>> {
        let bytes_read = self.reader.read(&mut self.buffer).map_err(|e| {
            log::trace!("read_gate: read(&mut buffer) failed: {e:?}");

            IoError
        })?;

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        match gate_serializer
            .read_gate(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("gate_serializer.read_gate(&mut buffer, common_data) failed: {e:?}");
            }) {
            Ok(result) => {
                self.reader
                    .seek_relative(-(buffer_plonky.remaining() as i64))
                    .map_err(|e| {
                        log::trace!(
                            "read_gate: seek_relative(-(buffer.remaining() as i64)) failed: {e:?}"
                        );

                        IoError
                    })?;

                return Ok(result);
            }

            // implementation of Read may be buffered so we have to try to read next buffer and
            // deserialize
            Err(_e) if bytes_read < self.buffer.len() => {}

            result => return result,
        }

        let bytes_read = bytes_read
            + self
                .reader
                .read(&mut self.buffer[bytes_read..])
                .map_err(|e| {
                    log::trace!("read_gate: read(&mut buffer[bytes_read]) failed 2: {e:?}");

                    IoError
                })?;

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        let result = gate_serializer
            .read_gate(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("gate_serializer.read_gate(&mut buffer, common_data) failed 2: {e:?}");
            })?;

        self.reader
            .seek_relative(-(buffer_plonky.remaining() as i64))
            .map_err(|e| {
                log::trace!(
                    "read_gate: seek_relative(-(buffer.remaining() as i64)) failed 2: {e:?}"
                );

                IoError
            })?;

        Ok(result)
    }

    fn read_generator<F: RichField + Extendable<D>, const D: usize>(
        &mut self,
        generator_serializer: &dyn WitnessGeneratorSerializer<F, D>,
        common_data: &CommonCircuitData<F, D>,
    ) -> IoResult<WitnessGeneratorRef<F, D>> {
        let bytes_read = self.reader.read(&mut self.buffer).map_err(|e| {
            log::trace!("read_generator: read(&mut buffer) failed: {e:?}");

            IoError
        })?;

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        match generator_serializer.read_generator(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("generator_serializer.read_generator(&mut buffer_plonky, common_data) failed: {e:?}");
            })
        {
            Ok(result) => {
                self.reader.seek_relative(-(buffer_plonky.remaining() as i64))
                    .map_err(|e| {
                        log::trace!("read_generator: seek_relative(-(buffer.remaining() as i64)) failed: {e:?}");

                        IoError
                    })?;

                return Ok(result);
            }

            // implementation of Read may be buffered so we have to try to read next buffer and
            // deserialize
            Err(_e) if bytes_read < self.buffer.len() => {}

            result => return result,
        }

        let bytes_read = bytes_read
            + self
                .reader
                .read(&mut self.buffer[bytes_read..])
                .map_err(|e| {
                    log::trace!("read_generator: read(&mut buffer[bytes_read]) failed 2: {e:?}");

                    IoError
                })?;

        let mut buffer_plonky = Buffer::new(&self.buffer[..bytes_read]);
        let result = generator_serializer.read_generator(&mut buffer_plonky, common_data)
            .inspect_err(|e| {
                log::trace!("generator_serializer.read_generator(&mut buffer_plonky, common_data) failed 2: {e:?}");
            })?;

        self.reader.seek_relative(-(buffer_plonky.remaining() as i64))
            .map_err(|e| {
                log::trace!("generator_serializer: seek_relative(-(buffer.remaining() as i64)) failed 2: {e:?}");

                IoError
            })?;

        Ok(result)
    }
}
