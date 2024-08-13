//! ### Circuits that're used to parse SCALE-encoded compact integers.
//!
//! There're 2 circuits present: `single_byte` and `full`.
//!
//! - `single_byte` allows to parse only integers encoded in so-called single-byte mode, having values
//!   in range (0..=63).
//! - `full` allows to parse integers encoded in single-, two- and four-byte modes, having values in
//!   range (0..=2^30 - 1).

use crate::{
    common::{
        targets::{impl_target_set, ArrayTarget, ByteTarget},
        BuilderExt,
    },
    prelude::*,
};
use plonky2::{iop::target::Target, plonk::circuit_builder::CircuitBuilder};
use plonky2_field::types::Field;

/// Supports decoding SCALE-encoded single-byte mode compact integers.
/// In this mode integer value can be in range \[0..=63\].
pub mod single_byte {
    use super::*;

    impl_target_set! {
        pub struct InputTarget {
            /// First byte of parsed integer.
            pub first_byte: ByteTarget
        }
    }

    impl_target_set! {
        pub struct OutputTarget {
            /// Decoded integer.
            pub decoded: Target
        }
    }

    pub fn define(input: InputTarget, builder: &mut CircuitBuilder<F, D>) -> OutputTarget {
        let bits = input.first_byte.as_bit_targets(builder);

        builder.assert_zero(bits.0[0].target);
        builder.assert_zero(bits.0[1].target);

        let decoded = builder.le_sum(bits.0[2..8].iter());

        OutputTarget { decoded }
    }

    #[cfg(test)]
    mod tests {
        use parity_scale_codec::{Compact, Encode};
        use plonky2::{iop::witness::PartialWitness, plonk::circuit_data::CircuitConfig};

        use super::*;

        #[test]
        fn test_scale_compact_integer_parser_single_byte() {
            positive_test_case(0);
            positive_test_case(63);
            positive_test_case(3);
        }

        #[test]
        #[should_panic(
            expected = "Partition containing Wire(Wire { row: 0, column: 1 }) was set twice with different values: 0 != 1"
        )]
        fn test_scale_compact_integer_parser_single_byte_fails_on_unsupported_prefix() {
            test_case(Compact::<u32>(100).encode()[0], None);
        }

        fn positive_test_case(value: u32) {
            let encoded = Compact::<u32>(value).encode();
            assert_eq!(encoded.len(), 1);
            test_case(encoded[0], Some(value as usize));
        }

        fn test_case(first_byte: u8, expected: Option<usize>) {
            let config = CircuitConfig::standard_recursion_config();
            let mut builder = CircuitBuilder::<F, D>::new(config);
            let pw = PartialWitness::new();

            let first_byte = ByteTarget::constant(first_byte, &mut builder);
            let input = InputTarget { first_byte };
            let output = define(input, &mut builder);

            if let Some(expected) = expected {
                let expected = builder.constant(F::from_canonical_usize(expected));
                builder.connect(output.decoded, expected);
            }

            let circuit = builder.build::<C>();
            let proof = circuit.prove(pw).expect("Failed to prove");
            circuit.verify(proof).expect("Failed to verify");
        }
    }
}

/// Supports decoding of compact integers in single-, two- and four- byte modes.
/// In these modes integer can take values in range \[0..=2^30 - 1\].
pub mod full {
    use super::*;

    impl_target_set! {
        pub struct InputTarget {
            /// First 4 bytes of parsed integer, padded if there're less than 4.
            pub padded_bytes: ArrayTarget<ByteTarget, 4>
        }
    }

    impl_target_set! {
        pub struct OutputTarget {
            /// Decoded integer.
            pub decoded: Target,
            /// Length of encoded integer in bytes.
            pub length: Target
        }
    }

    pub fn define(input: InputTarget, builder: &mut CircuitBuilder<F, D>) -> OutputTarget {
        let first_byte_bits = input.padded_bytes.constant_read(0).as_bit_targets(builder);

        let mode = builder.mul_const(F::TWO, first_byte_bits.constant_read(1).target);
        let mode = builder.add(mode, first_byte_bits.constant_read(0).target);

        let refined_first_byte =
            ByteTarget::from_target_unsafe(builder.le_sum(first_byte_bits.0[2..8].iter()));

        let big_int_mode = builder.constant(F::from_canonical_usize(4));
        let big_int_mode = builder.is_equal(mode, big_int_mode);
        builder.assert_zero(big_int_mode.target);

        let single_byte_mode = builder.zero();
        let is_single_byte_mode = builder.is_equal(mode, single_byte_mode);
        let single_byte_out = try_parse_single_byte(refined_first_byte, builder);

        let two_byte_mode = builder.one();
        let is_two_byte_mode = builder.is_equal(mode, two_byte_mode);
        let two_byte_out = try_parse_two_byte(
            [refined_first_byte, input.padded_bytes.constant_read(1)],
            builder,
        );

        let four_byte_out = try_parse_four_byte(
            [
                refined_first_byte,
                input.padded_bytes.constant_read(1),
                input.padded_bytes.constant_read(2),
                input.padded_bytes.constant_read(3),
            ],
            builder,
        );

        let out = builder.select_target_set(is_single_byte_mode, &single_byte_out, &four_byte_out);
        builder.select_target_set(is_two_byte_mode, &two_byte_out, &out)
    }

    fn try_parse_single_byte(
        refined_byte: ByteTarget,
        builder: &mut CircuitBuilder<F, D>,
    ) -> OutputTarget {
        OutputTarget {
            decoded: refined_byte.as_target(),
            length: builder.one(),
        }
    }

    fn try_parse_two_byte(
        refined_bytes: [ByteTarget; 2],
        builder: &mut CircuitBuilder<F, D>,
    ) -> OutputTarget {
        let decoded = builder.mul_const(
            F::from_canonical_usize(1 << 6),
            refined_bytes[1].as_target(),
        );
        let decoded = builder.add(decoded, refined_bytes[0].as_target());

        OutputTarget {
            decoded,
            length: builder.two(),
        }
    }

    fn try_parse_four_byte(
        refined_bytes: [ByteTarget; 4],
        builder: &mut CircuitBuilder<F, D>,
    ) -> OutputTarget {
        let decoded = refined_bytes[0].as_target();

        let val = builder.mul_const(
            F::from_canonical_usize(1 << 6),
            refined_bytes[1].as_target(),
        );
        let decoded = builder.add(decoded, val);

        let val = builder.mul_const(
            F::from_canonical_usize(1 << 14),
            refined_bytes[2].as_target(),
        );
        let decoded = builder.add(decoded, val);

        let val = builder.mul_const(
            F::from_canonical_usize(1 << 22),
            refined_bytes[3].as_target(),
        );
        let decoded = builder.add(decoded, val);

        let length = builder.constant(F::from_canonical_usize(4));

        OutputTarget { decoded, length }
    }

    #[cfg(test)]
    mod tests {
        use std::iter;

        use parity_scale_codec::{Compact, Encode};
        use plonky2::{iop::witness::PartialWitness, plonk::circuit_data::CircuitConfig};
        use plonky2_field::types::PrimeField64;

        use super::*;

        #[test]
        fn test_scale_compact_integer_parser_full() {
            positive_test_case(0);
            positive_test_case(1);
            positive_test_case(23);
            positive_test_case(63);

            positive_test_case(64);
            positive_test_case(5731);
            positive_test_case((1 << 14) - 1);

            positive_test_case(1 << 14);
            positive_test_case((1 << 18) + 123);
            positive_test_case((1 << 30) - 1);
        }

        #[test]
        #[should_panic(
            expected = "Partition containing Wire(Wire { row: 3, column: 63 }) was set twice with different values: 5 != 4"
        )]
        fn test_scale_compact_integer_parser_fails_on_unsupported_prefix() {
            test_case(Compact::<u32>(u32::MAX).encode(), None);
        }

        fn positive_test_case(value: u32) {
            test_case(Compact::<u32>(value).encode(), Some(value));
        }

        fn test_case(bytes: Vec<u8>, expected: Option<u32>) {
            let config = CircuitConfig::standard_recursion_config();
            let mut builder = CircuitBuilder::<F, D>::new(config);
            let pw = PartialWitness::new();

            let expected_len = builder.constant(F::from_canonical_usize(bytes.len()));

            let bytes: [ByteTarget; 4] = bytes
                .into_iter()
                .chain(iter::repeat(0))
                .map(|val| ByteTarget::constant(val, &mut builder))
                .take(4)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();

            let input = InputTarget {
                padded_bytes: ArrayTarget(bytes),
            };
            let output = define(input, &mut builder);

            builder.connect(output.length, expected_len);

            builder.register_public_input(output.decoded);
            let circuit = builder.build::<C>();
            let proof = circuit.prove(pw).expect("Failed to prove");

            if let Some(expected) = expected {
                let decoded = proof.public_inputs[0].to_canonical_u64();
                assert_eq!(decoded, expected as u64);
            }

            circuit.verify(proof).expect("Failed to verify");
        }
    }
}
