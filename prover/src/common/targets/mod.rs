//! ### Contains newtypes that structurize target arrays.

use itertools::Itertools;
use plonky2::{
    hash::hash_types::{HashOut, HashOutTarget, NUM_HASH_OUT_ELTS},
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::circuit_builder::CircuitBuilder,
};
use plonky2_field::types::{Field, PrimeField64};
use std::{fmt::Debug, iter};

use crate::prelude::{consts::*, *};

mod array;
mod bool;
mod byte;
mod half_byte;
mod target;
mod verifier_data;

pub use array::*;
pub use byte::*;
pub use half_byte::*;
pub use target::*;
pub use verifier_data::*;

/// Common trait that all the target array wrappers implement.
pub trait TargetSet: Clone + Debug {
    /// Parse `TargetSet` from iterator over raw targets. This function will read as much elements
    /// from an iterator as it requires. If amount of targets is insufficient it will panic.
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self;

    /// Erase structure of current `TargetSet` and return iterator over raw targets.
    fn into_targets_iter(self) -> impl Iterator<Item = Target>;

    /// Parse `TargetSet` from iterator over raw targets, checking that provided iterator contains
    /// exact amount of targets.
    fn parse_exact(raw: &mut impl Iterator<Item = Target>) -> Self {
        let out = Self::parse(raw);
        assert_eq!(
            raw.next(),
            None,
            "Too much targets are provided to parse_exact. Found {} excess targets.",
            raw.collect::<Vec<_>>().len() + 1
        );
        out
    }

    /// Create constraint that will force two `TargetSet`s to be equal.
    fn connect(&self, other: &Self, builder: &mut CircuitBuilder<F, D>) {
        self.clone()
            .into_targets_iter()
            .zip(other.clone().into_targets_iter())
            .for_each(|(t_0, t_1)| builder.connect(t_0, t_1));
    }

    /// Register all the underlying targets as public inputs in order defined by
    /// `TargetSet::into_targets_iter`.
    fn register_as_public_inputs(&self, builder: &mut CircuitBuilder<F, D>) {
        self.clone()
            .into_targets_iter()
            .for_each(|t| builder.register_public_input(t));
    }
}

/// Extension for `TargetSet` that allows to parse public input values structure defined as
/// `PublicInputsData`.
pub trait ParsableTargetSet: TargetSet {
    type PublicInputsData;

    /// Parse public input values into `PublicInputsData`. This function will read as much public
    /// input values as it needs. In case of insufficient amount of public inputs it will panic.
    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData;

    /// Parse public input values into `PublicInputsData`. This function will assert that amount of
    /// items in iterator is the same as required to fill `PublicInputsData`. If this don't hold,
    /// this function will panic.
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
                .expect(
                    "Insufficient public input count to counstruct HashOutTarget public inputs",
                ),
        }
    }
}

pub(crate) use crate::{impl_parsable_target_set, impl_target_set};

/// Implement `TargetSet` for structure. Order in which targets will be parsed and converted back
/// into iterator over `Target`s is guaranteed to be the same as order that fields are defined in
/// provided structure definition.
#[macro_export]
macro_rules! impl_target_set {
    (
        $(#[$($struct_attributes:tt)*])*
        $vis:vis struct $struct_name:ident {
            $(
                $(#[$($attributes:tt)*])*
                $field_vis:vis $field_name:ident: $field_type:ty
            ),*
            $(,)?
        }
    ) => {
        #[derive(Clone, Debug)]
        $(#[$($struct_attributes)*])*
        $vis struct $struct_name {
            $(
                $(#[$($attributes)*])*
                $field_vis $field_name: $field_type
            ),*
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

/// Implement `TargetSet` and `ParsableTargetSet` for structure. Order in which targets/public
/// input values will be parsed and converted back into iterator over `Target`s/`GoldilocksField`
/// elements is guaranteed to be the same as order that fields are defined in provided structure
/// definition.
#[macro_export]
macro_rules! impl_parsable_target_set {
    (
        $(#[$($struct_attributes:tt)*])*
        $vis:vis struct $struct_name:ident {
            $(
                $(#[$($field_attributes:tt)*])*
                $field_vis:vis $field_name:ident: $field_type:ty
            ),*
            $(,)?
        }
    ) => {
        $crate::common::targets::impl_target_set! {
            $(#[$($struct_attributes)*])*
            $vis struct $struct_name {
                $(
                    $(#[$($field_attributes)*])*
                    $field_vis $field_name: $field_type
                ),*
            }
        }

        ::paste::paste! {
            #[derive(Clone, Debug)]
            #[allow(dead_code)]
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

pub(crate) use crate::{impl_array_target_wrapper, impl_parsable_array_target_wrapper};

/// Implement newtype wrapper over `ArrayTarget`.
#[macro_export]
macro_rules! impl_array_target_wrapper {
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

/// Implement newtype wrapper over `ArrayTarget` and auto-implement `ParsableTargetSet` for it.
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

impl_array_target_wrapper!(
    Blake2TargetGoldilocks,
    Target,
    BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS
);
impl_array_target_wrapper!(
    MessageTargetGoldilocks,
    Target,
    MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS
);

impl_parsable_array_target_wrapper!(Blake2Target, BoolTarget, BLAKE2_DIGEST_SIZE_IN_BITS);

impl Blake2Target {
    /// Add virtual `Blake2Target` without inserting assertions that underlying targets are binary.
    pub fn add_virtual_unsafe(builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let mut targets = (0..BLAKE2_DIGEST_SIZE_IN_BITS).map(|_| builder.add_virtual_target());
        Blake2Target::parse_exact(&mut targets)
    }

    /// Add virtual `Blake2Target` and insert assertions that each underlying bit is, in fact, binary.
    pub fn add_virtual_safe(builder: &mut CircuitBuilder<F, D>) -> Blake2Target {
        let mut targets =
            (0..BLAKE2_DIGEST_SIZE_IN_BITS).map(|_| builder.add_virtual_bool_target_safe().target);
        Blake2Target::parse_exact(&mut targets)
    }

    /// Set witness for `Blake2Target`,
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

    /// Check if two `Blake2Target`s are equal. Unlike `connect` this check doesn't add constraint
    /// on each pair of corresponding bits, so it's correct situation to have outcome of this
    /// comparison `false`.
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
impl_array_target_wrapper!(
    PaddedValidatorSetTarget,
    Ed25519PublicKeyTarget,
    MAX_VALIDATOR_COUNT
);

impl ParsableTargetSet for Blake2TargetGoldilocks {
    type PublicInputsData = [u8; BLAKE2_DIGEST_SIZE];

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs
            .take(BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS)
            .flat_map(|f| (f.to_canonical_u64() as u32).to_le_bytes())
            .collect::<Vec<_>>()
            .try_into()
            .expect(
                "Insufficient public input count to construct Blake2TargetGoldilocks public inputs",
            )
    }
}

impl Blake2TargetGoldilocks {
    /// Packs underlying `BoolTarget`s to `Target`s by groups of 32.
    pub fn from_blake2_target(target: Blake2Target, builder: &mut CircuitBuilder<F, D>) -> Self {
        const BITS_FOR_SINGLE_TARGET: usize = 32;
        let targets = BitArrayTarget::compress_to_goldilocks::<
            BITS_FOR_SINGLE_TARGET,
            BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS,
        >(&target.0, builder);

        Self(ArrayTarget(targets))
    }
}

const PACK_MESSAGE_BY: usize = MESSAGE_SIZE_IN_BITS / MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS;
static_assertions::const_assert_eq!(
    MESSAGE_SIZE_IN_BITS % MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS,
    0
);

impl MessageTargetGoldilocks {
    /// Pack 256 bits of message into by grouops of 32 to get 8 `Targets`. That will be the
    /// representation of message in compact form.
    pub fn from_bit_array(
        bits: BitArrayTarget<MESSAGE_SIZE_IN_BITS>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Self {
        let targets = BitArrayTarget::compress_to_goldilocks::<
            PACK_MESSAGE_BY,
            MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS,
        >(&bits, builder);

        Self(ArrayTarget(targets))
    }
}
