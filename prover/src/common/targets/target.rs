//! ### Contains extensions for `Target`.

use super::*;

impl TargetSet for Target {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        raw.next().expect("Non-empty iterator")
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self)
    }
}

impl ParsableTargetSet for Target {
    type PublicInputsData = u64;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs
            .next()
            .expect("Non-empty iterator")
            .to_canonical_u64()
    }
}

pub trait TargetBitOperations {
    /// Compute little-endian sum of provided bits.
    ///
    /// Note: If 64 bits are provided result may overflow. This case is not processed.
    fn from_bool_targets_le<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        bits_to_target(bits, builder, true)
    }

    /// Compute big-endian sum of provided bits.
    ///
    /// Note: If 64 bits are provided result may overflow. This case is not processed.
    fn from_bool_targets_be<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        bits_to_target(bits, builder, false)
    }

    /// Compute little-endian sum of provided bits.
    ///
    /// Note: Result may overflow. This case is not processed.
    fn from_u64_bits_le_lossy(
        bits: ArrayTarget<BoolTarget, 64>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        Self::from_bool_targets_le(bits, builder)
    }
}

fn bits_to_target<const B: usize>(
    bits: ArrayTarget<BoolTarget, B>,
    builder: &mut CircuitBuilder<F, D>,
    little_endian: bool,
) -> Target {
    assert!(B <= 64);
    assert!(B % 8 == 0);

    let bits = bits.0.chunks(8);
    let mut bits: Vec<_> = if little_endian {
        bits.rev().flatten().rev().collect()
    } else {
        bits.flatten().rev().collect()
    };

    if B == 64 {
        let most_significant_bit = bits.pop().expect("bits mustn't be empty").target;
        let partial_sum = builder.le_sum(bits.into_iter());
        let most_significant_exp = builder.constant(F::from_canonical_u64(1 << (B - 1)));
        builder.mul_add(most_significant_exp, most_significant_bit, partial_sum)
    } else {
        builder.le_sum(bits.into_iter())
    }
}

impl TargetBitOperations for Target {}

#[test]
fn test_single_target_from_u64_bits_le_lossy() {
    use plonky2::plonk::circuit_data::CircuitConfig;

    fn test_case(num: u64) {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_ecc_config());

        let bits = crate::common::array_to_bits(&num.to_le_bytes());
        let bit_targets = [(); 64].map(|_| builder.add_virtual_bool_target_safe());

        let resulting_target =
            Target::from_u64_bits_le_lossy(ArrayTarget(bit_targets), &mut builder);
        builder.register_public_input(resulting_target);

        let mut pw = PartialWitness::new();

        for (value, target) in bits.iter().zip(bit_targets.iter()) {
            pw.set_bool_target(*target, *value);
        }

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Proven true");

        assert_eq!(proof.public_inputs.len(), 1);

        let result = proof.public_inputs[0];

        println!("{num}");

        assert_eq!(result, F::from_noncanonical_u64(num));
        assert!(circuit.verify(proof).is_ok());
    }

    test_case(0);
    test_case(100_000);
    test_case(u32::MAX as u64);
    test_case(1 << 48);
    test_case(u64::MAX - (u32::MAX as u64) * 8);
    test_case(u64::MAX);
}
