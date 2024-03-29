use plonky2::{iop::target::Target, plonk::circuit_builder::CircuitBuilder};
use plonky2_field::types::Field;

use crate::{
    common::targets::{impl_target_set, ByteTarget},
    prelude::*,
};

impl_target_set! {
    pub struct ScaleCompactIntegerParserInputTarget {
        pub first_byte: ByteTarget
    }
}

impl_target_set! {
    pub struct ScaleCompactIntegerParserOutputTarget {
        pub decoded: Target
    }
}

/// Supports only single-byte mode values for now.
pub fn define(
    input: ScaleCompactIntegerParserInputTarget,
    builder: &mut CircuitBuilder<F, D>,
) -> ScaleCompactIntegerParserOutputTarget {
    let bits = input.first_byte.to_bit_targets(builder);

    builder.assert_zero(bits.0[0].target);
    builder.assert_zero(bits.0[1].target);

    let shift = builder.constant(F::from_canonical_u8(4));
    let decoded = builder.div(input.first_byte.to_target(), shift).into();

    ScaleCompactIntegerParserOutputTarget { decoded }
}

#[cfg(test)]
mod tests {
    use plonky2::{iop::witness::PartialWitness, plonk::circuit_data::CircuitConfig};

    use super::*;

    #[test]
    fn test_scale_compact_integer_parser() {
        test_case(0b00_00_00_00, Some(0));
        test_case(0b11_11_11_00, Some(63));
        test_case(0b00_00_11_00, Some(3));
    }

    #[test]
    #[should_panic(
        expected = "Partition containing Wire(Wire { row: 0, column: 2 }) was set twice with different values: 0 != 1"
    )]
    fn test_scale_compact_integer_parser_fails_on_unsupported_prefix() {
        test_case(0b00_00_00_10, None);
    }

    fn test_case(first_byte: u8, expected: Option<usize>) {
        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let pw = PartialWitness::new();

        let first_byte = ByteTarget::constant(first_byte, &mut builder);
        let input = ScaleCompactIntegerParserInputTarget { first_byte };
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
