use std::fmt::Debug;
use std::ops::Deref;

use self::consts::VALIDATOR_COUNT;

use crate::{
    common::{array_to_bits, bits_to_byte},
    consts::*,
    prelude::*,
};
use itertools::Itertools;
use plonky2::{
    hash::hash_types::{HashOut, HashOutTarget, NUM_HASH_OUT_ELTS},
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{circuit_builder::CircuitBuilder, circuit_data::VerifierOnlyCircuitData},
};
use plonky2_field::{
    goldilocks_field::GoldilocksField,
    types::{Field, PrimeField64},
};
use std::iter;

pub trait TargetSet: Clone + Debug {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self;
    fn into_targets_iter(self) -> impl Iterator<Item = Target>;

    fn parse_exact(raw: &mut impl Iterator<Item = Target>) -> Self {
        let out = Self::parse(raw);
        assert_eq!(raw.next(), None);
        out
    }

    fn connect(&self, other: &Self, builder: &mut CircuitBuilder<F, D>) {
        self.clone()
            .into_targets_iter()
            .zip(other.clone().into_targets_iter())
            .for_each(|(t_0, t_1)| builder.connect(t_0, t_1));
    }

    fn register_as_public_inputs(&self, builder: &mut CircuitBuilder<F, D>) {
        self.clone()
            .into_targets_iter()
            .for_each(|t| builder.register_public_input(t));
    }
}

pub trait ParsableTargetSet: TargetSet {
    type PublicInputsData;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData;
}

impl TargetSet for Target {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        raw.next().unwrap()
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self)
    }
}

impl TargetSet for BoolTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self::new_unsafe(raw.next().unwrap())
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self.target)
    }
}

impl TargetSet for HashOutTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        let target = HashOutTarget::from_vec(raw.take(CIRCUIT_DIGEST_SIZE).collect());
        assert_eq!(target.elements.len(), CIRCUIT_DIGEST_SIZE);
        target
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        self.elements.into_iter()
    }
}

impl ParsableTargetSet for HashOutTarget {
    type PublicInputsData = HashOut<F>;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        HashOut {
            elements: public_inputs
                .take(NUM_HASH_OUT_ELTS)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        }
    }
}

pub(crate) use crate::impl_target_set;

#[macro_export]
macro_rules! impl_target_set {
    (
        $vis:vis struct $struct_name:ident {
            $($field_vis:vis $field_name:ident: $field_type:ty),*
            $(,)?
        }
    ) => {
        #[derive(Clone, Debug)]
        $vis struct $struct_name {
            $($field_vis $field_name: $field_type),*
        }

        impl $crate::common::targets::TargetSet for $struct_name {
            fn parse(raw: &mut impl Iterator<Item = plonky2::iop::target::Target>) -> Self {
                Self {
                    $($field_name: TargetSet::parse(raw)),*
                }
            }

            fn into_targets_iter(self) -> impl Iterator<Item = plonky2::iop::target::Target> {
                ::std::iter::empty()
                $(.chain(self.$field_name.into_targets_iter()))*
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct VerifierDataTarget<const NUM_CAP_ELEMENTS: usize> {
    pub circuit_digest: HashOutTarget,
    pub merkle_caps: [HashOutTarget; NUM_CAP_ELEMENTS],
}

impl<const NUM_CAP_ELEMENTS: usize> TargetSet for VerifierDataTarget<NUM_CAP_ELEMENTS> {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self {
            circuit_digest: HashOutTarget::parse(raw),
            merkle_caps: (0..NUM_CAP_ELEMENTS)
                .map(|_| HashOutTarget::parse(raw))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        }
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        self.circuit_digest.into_targets_iter().chain(
            self.merkle_caps
                .into_iter()
                .map(|hash| hash.into_targets_iter())
                .flatten(),
        )
    }
}

// TODO REFACTOR: remove pub on inner type.
#[derive(Clone, Debug, Copy)]
pub struct ArrayTarget<T: TargetSet, const N: usize>(pub [T; N]);

pub type BitArrayTarget<const N: usize> = ArrayTarget<BoolTarget, N>;

impl<T: TargetSet, const N: usize> TargetSet for ArrayTarget<T, N> {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self(
            (0..N)
                .map(|_| TargetSet::parse(raw))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        self.0
            .into_iter()
            .flat_map(|element| element.into_targets_iter())
    }
}

impl<T: TargetSet, const N: usize> ArrayTarget<T, N> {
    pub fn constant_read(&self, at: usize) -> T {
        self.0[at].clone()
    }

    pub fn random_read(&self, at: SingleTarget, builder: &mut CircuitBuilder<F, D>) -> T {
        let self_targets = self
            .0
            .clone()
            .into_iter()
            .map(|ts| ts.into_targets_iter().collect::<Vec<_>>())
            .collect::<Vec<_>>();

        let inner_target_len = self_targets[0].len();
        assert!(self_targets.iter().all(|t| t.len() == inner_target_len));

        let self_targets_padded_len = (self_targets.len() * inner_target_len).next_power_of_two();
        let zero_target = builder.zero();
        let self_targets = self_targets
            .into_iter()
            .flatten()
            .chain(iter::repeat(zero_target))
            .take(self_targets_padded_len)
            .collect::<Vec<_>>();

        let access_targets = (0..inner_target_len)
            .map(|offset| {
                let offset_const = builder.constant(F::from_canonical_usize(offset));
                builder.add(at.0, offset_const)
            })
            .collect::<Vec<_>>();

        let mut result_targets = access_targets
            .into_iter()
            .map(|access_at| builder.random_access(access_at, self_targets.clone()));

        T::parse_exact(&mut result_targets)
    }

    pub fn random_read_array<const R: usize>(
        &self,
        at: SingleTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> ArrayTarget<T, R> {
        ArrayTarget(
            (0..R)
                .map(|offset| {
                    let offset = builder.constant(F::from_canonical_usize(offset));
                    let read_at = builder.add(at.to_target(), offset);
                    self.random_read(read_at.into(), builder)
                })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )
    }
}

pub trait TargetSetWitnessOperations {
    fn set_partial_witness(&self, data: &[u8], witness: &mut PartialWitness<F>);
}

impl<T, const N: usize> TargetSetWitnessOperations for T
where
    T: Deref<Target = ArrayTarget<BoolTarget, N>>,
{
    fn set_partial_witness(&self, data: &[u8], witness: &mut PartialWitness<F>) {
        let data = array_to_bits(data);
        for (target, bit) in self.into_targets_iter().zip(data.into_iter()) {
            witness.set_bool_target(BoolTarget::new_unsafe(target), bit);
        }
    }
}

trait BoolTargetsArrayToSingleTargets<const PACK_BY: usize> {
    fn compress_to_goldilocks(&self, builder: &mut CircuitBuilder<F, D>) -> Vec<SingleTarget>;
}

impl<const N: usize, const PACK_BY: usize> BoolTargetsArrayToSingleTargets<PACK_BY>
    for ArrayTarget<BoolTarget, N>
{
    fn compress_to_goldilocks(&self, builder: &mut CircuitBuilder<F, D>) -> Vec<SingleTarget> {
        assert_eq!(N % PACK_BY, 0);
        assert!(PACK_BY <= 64);

        let bit_exp_targets = (0..PACK_BY)
            .rev()
            .map(|bit_no| builder.constant(GoldilocksField::from_noncanonical_u64(1 << bit_no)))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        self.0
            .chunks(PACK_BY)
            .map(|bits| {
                let bits: [BoolTarget; PACK_BY] = bits.try_into().unwrap();

                SingleTarget::from_bool_targets_le_precomputed_exp::<PACK_BY>(
                    ArrayTarget(bits),
                    &bit_exp_targets,
                    builder,
                )
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SingleTarget(Target);

impl TargetSet for SingleTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self(raw.next().unwrap())
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self.0)
    }
}

impl ParsableTargetSet for SingleTarget {
    type PublicInputsData = u64;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs.next().unwrap().to_canonical_u64()
    }
}

impl From<Target> for SingleTarget {
    fn from(value: Target) -> Self {
        Self(value)
    }
}

impl SingleTarget {
    pub fn to_target(&self) -> Target {
        self.0
    }
}

#[derive(Clone, Debug)]
pub struct ByteTarget(Target);

impl TargetSet for ByteTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self(raw.next().unwrap())
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self.0)
    }
}

impl ByteTarget {
    pub fn constant(value: u8, builder: &mut CircuitBuilder<F, D>) -> ByteTarget {
        Self(builder.constant(F::from_canonical_u8(value)))
    }

    pub fn from_target_safe(target: Target, builder: &mut CircuitBuilder<F, D>) -> ByteTarget {
        builder.range_check(target, 8);
        Self(target)
    }

    pub fn from_target_unsafe(target: Target) -> ByteTarget {
        Self(target)
    }

    pub fn to_target(&self) -> Target {
        self.0
    }

    /// Splits byte into `(least_significant, most_significant)` half-bytes.
    pub fn to_half_byte_targets(
        &self,
        builder: &mut CircuitBuilder<F, D>,
    ) -> (HalfByteTarget, HalfByteTarget) {
        let bits = self.to_bit_targets(builder);

        let l_bits = &bits.0[..4];
        let h_bits = &bits.0[4..];

        (
            HalfByteTarget::from_target_unsafe(builder.le_sum(l_bits.iter())),
            HalfByteTarget::from_target_unsafe(builder.le_sum(h_bits.iter())),
        )
    }

    /// Arranged from less to most significant bit.
    pub fn to_bit_targets(&self, builder: &mut CircuitBuilder<F, D>) -> ArrayTarget<BoolTarget, 8> {
        ArrayTarget(builder.low_bits(self.0, 8, 8).try_into().unwrap())
    }
}

#[derive(Clone, Debug)]
pub struct HalfByteTarget(Target);

impl TargetSet for HalfByteTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self(raw.next().unwrap())
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self.0)
    }
}

impl HalfByteTarget {
    pub fn constant(value: u8, builder: &mut CircuitBuilder<F, D>) -> HalfByteTarget {
        assert!(value < 16);
        Self(builder.constant(F::from_canonical_u8(value)))
    }

    pub fn from_target_safe(target: Target, builder: &mut CircuitBuilder<F, D>) -> HalfByteTarget {
        builder.range_check(target, 4);
        Self(target)
    }

    pub fn from_target_unsafe(target: Target) -> HalfByteTarget {
        Self(target)
    }

    pub fn to_target(&self) -> Target {
        self.0
    }
}

pub(crate) use crate::impl_array_target_wrapper;

#[macro_export]
macro_rules! impl_array_target_wrapper {
    // TODO: Add access modifier to params.
    ($name:ident, $target_ty:ty, $len:ident) => {
        #[derive(::std::clone::Clone, ::std::fmt::Debug)]
        pub struct $name($crate::common::targets::ArrayTarget<$target_ty, $len>);

        impl ::std::ops::Deref for $name {
            type Target = $crate::common::targets::ArrayTarget<$target_ty, $len>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl TargetSet for $name {
            fn parse(raw: &mut impl ::std::iter::Iterator<Item = Target>) -> Self {
                Self($crate::common::targets::TargetSet::parse(raw))
            }

            fn into_targets_iter(self) -> impl ::std::iter::Iterator<Item = Target> {
                self.0.into_targets_iter()
            }
        }
    };
}

impl_array_target_wrapper!(Sha256Target, BoolTarget, SHA256_DIGEST_SIZE_IN_BITS);
impl_array_target_wrapper!(
    Sha256TargetGoldilocks,
    Target,
    SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS
);
impl_array_target_wrapper!(
    MessageTargetGoldilocks,
    Target,
    MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS
);

impl_array_target_wrapper!(Blake2Target, BoolTarget, BLAKE2_DIGEST_SIZE_IN_BITS);

impl Blake2Target {
    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let mut targets = (0..BLAKE2_DIGEST_SIZE_IN_BITS).map(|_| builder.add_virtual_target());
        Blake2Target::parse_exact(&mut targets)
    }

    pub fn set_witness(
        &self,
        data: &[bool; BLAKE2_DIGEST_SIZE_IN_BITS],
        witness: &mut PartialWitness<F>,
    ) {
        self.0
             .0
            .iter()
            .zip_eq(data.iter())
            .for_each(|(target, value)| witness.set_bool_target(*target, *value));
    }
}

impl_array_target_wrapper!(
    Ed25519PublicKeyTarget,
    BoolTarget,
    ED25519_PUBLIC_KEY_SIZE_IN_BITS
);
impl_array_target_wrapper!(
    Ed25519SignatreTarget,
    BoolTarget,
    ED25519_SIGNATURE_SIZE_IN_BITS
);
impl_array_target_wrapper!(ValidatorSetTarget, Ed25519PublicKeyTarget, VALIDATOR_COUNT);

impl ParsableTargetSet for Sha256TargetGoldilocks {
    type PublicInputsData = [u8; 32];

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs
            .take(SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|f| array_to_bits(&f.to_canonical_u64().to_le_bytes())[64 - 52..].to_vec())
            .flatten()
            .skip(SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS * 52 - SHA256_DIGEST_SIZE_IN_BITS)
            .collect::<Vec<_>>()
            .chunks(8)
            .map(|bits| bits_to_byte(bits.try_into().unwrap()))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}

impl TargetSetWitnessOperations for ValidatorSetTarget {
    fn set_partial_witness(&self, data: &[u8], witness: &mut PartialWitness<F>) {
        self.0
             .0
            .iter()
            .zip(data.chunks(ED25519_PUBLIC_KEY_SIZE))
            .for_each(|(validator, data)| validator.set_partial_witness(data, witness));
    }
}

impl SingleTarget {
    // TODO: Specify exact behaviour when `little-endian` is not appliable
    // like in case with B = 52
    fn from_bool_targets_le_precomputed_exp<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        bit_exp_targets: &[Target; B],
        builder: &mut CircuitBuilder<F, D>,
    ) -> SingleTarget {
        assert!(B <= 64);

        let mut result = builder.zero();
        for (bit, exp) in bits.0.chunks(8).rev().flatten().zip(bit_exp_targets.iter()) {
            result = builder.mul_add(bit.target, *exp, result);
        }
        SingleTarget(result)
    }

    fn from_bool_targets_le<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> SingleTarget {
        let bit_exp_targets = (0..B)
            .rev()
            .map(|bit_no| builder.constant(GoldilocksField::from_noncanonical_u64(1 << bit_no)))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        Self::from_bool_targets_le_precomputed_exp(bits, &bit_exp_targets, builder)
    }

    pub fn from_u52_bits_le(
        bits: ArrayTarget<BoolTarget, 52>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> SingleTarget {
        Self::from_bool_targets_le(bits, builder)
    }

    /// Bits are sorted from less to most significant.
    pub fn from_u8_bits_le(
        bits: ArrayTarget<BoolTarget, 8>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> SingleTarget {
        Self::from_bool_targets_le(bits, builder)
    }

    pub fn from_u64_bits_le_lossy(
        bits: ArrayTarget<BoolTarget, 64>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> SingleTarget {
        Self::from_bool_targets_le(bits, builder)
    }
}

impl Sha256TargetGoldilocks {
    /// Packs underlying `BoolTarget`s to `Target`s by groups of 52.
    pub fn from_sha256_target(
        sha256_target: Sha256Target,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        const BITS_FOR_SINGLE_TARGET: usize = 52;
        const PADDED_SIZE: usize =
            SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS * BITS_FOR_SINGLE_TARGET;
        const PADDING: usize = PADDED_SIZE - SHA256_DIGEST_SIZE_IN_BITS;

        let padding_targets = (0..PADDING).map(|_| builder._false());
        let bit_targets: [_; PADDED_SIZE] = sha256_target
            .0
             .0
            .into_iter()
            .chain(padding_targets)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        let targets: [_; SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS] =
            BoolTargetsArrayToSingleTargets::<BITS_FOR_SINGLE_TARGET>::compress_to_goldilocks(
                &ArrayTarget(bit_targets),
                builder,
            )
            .into_iter()
            .map(|t| t.0)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        Self(ArrayTarget(targets))
    }
}

const PACK_MESSAGE_BY: usize = 52;
const MESSAGE_INPUT_IN_BITS: usize = MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS * PACK_MESSAGE_BY;

impl MessageTargetGoldilocks {
    pub fn from_bit_array(
        bits: BitArrayTarget<MESSAGE_INPUT_IN_BITS>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let targets: [_; MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS] =
            BoolTargetsArrayToSingleTargets::<PACK_MESSAGE_BY>::compress_to_goldilocks(
                &bits, builder,
            )
            .into_iter()
            .map(|t| t.0)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        Self(ArrayTarget(targets))
    }
}

#[test]
fn test_single_target_from_u64_bits_le_lossy() {
    use plonky2::plonk::circuit_data::CircuitConfig;

    fn test_case(num: u64) {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_ecc_config());

        let bits = array_to_bits(&num.to_le_bytes());
        let bit_targets: [BoolTarget; 64] = (0..bits.len())
            .map(|_| builder.add_virtual_bool_target_safe())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        let resulting_target =
            SingleTarget::from_u64_bits_le_lossy(ArrayTarget(bit_targets), &mut builder);
        builder.register_public_input(resulting_target.0);

        let mut pw = PartialWitness::new();

        for (value, target) in bits.iter().zip(bit_targets.iter()) {
            pw.set_bool_target(*target, *value);
        }

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).unwrap();

        assert_eq!(proof.public_inputs.len(), 1);

        let result = proof.public_inputs[0];

        println!("{}", num);

        assert_eq!(result, GoldilocksField::from_noncanonical_u64(num));
        assert!(circuit.verify(proof).is_ok());
    }

    test_case(0);
    test_case(100_000);
    test_case(u32::MAX as u64);
    test_case(1 << 48);
    test_case(u64::MAX - (u32::MAX as u64) * 8);
    test_case(u64::MAX);
}
