use itertools::Itertools;
use plonky2::{
    hash::hash_types::{HashOut, HashOutTarget, NUM_HASH_OUT_ELTS},
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::circuit_builder::CircuitBuilder,
};
use plonky2_field::{
    goldilocks_field::GoldilocksField,
    types::{Field, PrimeField64},
};
use std::{fmt::Debug, iter, ops::Deref};

use crate::{
    common::{array_to_bits, bits_to_byte},
    prelude::{consts::*, *},
};

mod array;
mod bool;
mod byte;
mod half_byte;
mod target;
mod verifier_data;

pub use array::*;
pub use bool::*;
pub use byte::*;
pub use half_byte::*;
pub use target::*;
pub use verifier_data::*;

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

    fn parse_public_inputs_exact(
        public_inputs: &mut impl Iterator<Item = F>,
    ) -> Self::PublicInputsData {
        let data = Self::parse_public_inputs(public_inputs);
        assert_eq!(public_inputs.next(), None);
        data
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

pub(crate) use crate::impl_parsable_target_set;
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
            fn parse(_raw: &mut impl ::std::iter::Iterator<Item = plonky2::iop::target::Target>) -> Self {
                Self {
                    $($field_name: $crate::common::targets::TargetSet::parse(_raw)),*
                }
            }

            fn into_targets_iter(self) -> impl ::std::iter::Iterator<Item = ::plonky2::iop::target::Target> {
                ::std::iter::empty()
                $(.chain(self.$field_name.into_targets_iter()))*
            }
        }
    }
}

#[macro_export]
macro_rules! impl_parsable_target_set {
    (
        $vis:vis struct $struct_name:ident {
            $($field_vis:vis $field_name:ident: $field_type:ty),*
            $(,)?
        }
    ) => {
        $crate::common::targets::impl_target_set! {
            $vis struct $struct_name {
                $($field_vis $field_name: $field_type),*
            }
        }

        ::paste::paste! {
            #[derive(Clone, Debug)]
            $vis struct [<$struct_name PublicInputs>] {
                $($field_vis $field_name: <$field_type as $crate::common::targets::ParsableTargetSet>::PublicInputsData),*
            }

            impl $crate::common::targets::ParsableTargetSet for $struct_name {
                type PublicInputsData = [<$struct_name PublicInputs>];

                fn parse_public_inputs(_public_inputs: &mut impl ::std::iter::Iterator<Item = F>) -> Self::PublicInputsData {
                    Self::PublicInputsData {
                        $($field_name: <$field_type as $crate::common::targets::ParsableTargetSet>::parse_public_inputs(_public_inputs)),*
                    }
                }
            }
        }
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

impl<const N: usize> ArrayTarget<BoolTarget, N> {
    fn compress_to_goldilocks<const PACK_BY: usize>(
        &self,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Vec<Target> {
        assert_eq!(N % PACK_BY, 0);
        assert!(PACK_BY <= 64);

        self.0
            .chunks(PACK_BY)
            .map(|bits| {
                let bits: [BoolTarget; PACK_BY] = bits.try_into().unwrap();

                Target::from_bool_targets_le::<PACK_BY>(ArrayTarget(bits), builder)
            })
            .collect()
    }
}

pub(crate) use crate::impl_array_target_wrapper;
pub(crate) use crate::impl_parsable_array_target_wrapper;

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

        impl $crate::common::targets::TargetSet for $name {
            fn parse(raw: &mut impl ::std::iter::Iterator<Item = Target>) -> Self {
                Self($crate::common::targets::TargetSet::parse(raw))
            }

            fn into_targets_iter(self) -> impl ::std::iter::Iterator<Item = Target> {
                self.0.into_targets_iter()
            }
        }
    };
}

#[macro_export]
macro_rules! impl_parsable_array_target_wrapper {
    ($name:ident, $target_ty:ty, $len:ident) => {
        $crate::common::targets::impl_array_target_wrapper!($name, $target_ty, $len);

        impl $crate::common::targets::ParsableTargetSet for $name {
            type PublicInputsData = <$crate::common::targets::ArrayTarget<$target_ty, $len> as ParsableTargetSet>::PublicInputsData;

            fn parse_public_inputs(public_inputs: &mut impl ::std::iter::Iterator<Item = F>) -> Self::PublicInputsData {
                $crate::common::targets::ArrayTarget::<$target_ty, $len>::parse_public_inputs(public_inputs)
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

impl_parsable_array_target_wrapper!(Blake2Target, BoolTarget, BLAKE2_DIGEST_SIZE_IN_BITS);

impl Blake2Target {
    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let mut targets = (0..BLAKE2_DIGEST_SIZE_IN_BITS).map(|_| builder.add_virtual_target());
        Blake2Target::parse_exact(&mut targets)
    }

    pub fn add_virtual_safe(builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let mut targets =
            (0..BLAKE2_DIGEST_SIZE_IN_BITS).map(|_| builder.add_virtual_bool_target_safe().target);
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

    pub fn check_equal(
        &self,
        other: &Blake2Target,
        builder: &mut CircuitBuilder<F, D>,
    ) -> BoolTarget {
        let mut equal = builder._true();
        for (self_bit, other_bit) in self.0 .0.iter().zip_eq(other.0 .0.iter()) {
            let bits_equal = builder.is_equal(self_bit.target, other_bit.target);
            equal = builder.and(equal, bits_equal);
        }

        equal
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
            .flat_map(|f| array_to_bits(&f.to_canonical_u64().to_le_bytes())[64 - 52..].to_vec())
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
            BitArrayTarget::compress_to_goldilocks::<BITS_FOR_SINGLE_TARGET>(
                &ArrayTarget(bit_targets),
                builder,
            )
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
            BitArrayTarget::compress_to_goldilocks::<PACK_MESSAGE_BY>(&bits, builder)
                .try_into()
                .unwrap();

        Self(ArrayTarget(targets))
    }
}
