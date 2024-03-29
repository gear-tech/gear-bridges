use super::*;

impl TargetSet for Target {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        raw.next().unwrap()
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self)
    }
}

impl ParsableTargetSet for Target {
    type PublicInputsData = u64;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs.next().unwrap().to_canonical_u64()
    }
}

pub trait TargetBitOperations {
    fn from_bool_targets_le_precomputed_exp<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        bit_exp_targets: &[Target; B],
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target;

    fn from_bool_targets_le<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target;

    fn from_u52_bits_le(
        bits: ArrayTarget<BoolTarget, 52>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        Self::from_bool_targets_le(bits, builder)
    }

    /// Bits are sorted from less to most significant.
    fn from_u8_bits_le(
        bits: ArrayTarget<BoolTarget, 8>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        Self::from_bool_targets_le(bits, builder)
    }

    fn from_u64_bits_le_lossy(
        bits: ArrayTarget<BoolTarget, 64>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        Self::from_bool_targets_le(bits, builder)
    }
}

// TODO: Use BaseSumGate.
impl TargetBitOperations for Target {
    // TODO: Specify exact behaviour when `little-endian` is not appliable
    // like in case with B = 52
    fn from_bool_targets_le_precomputed_exp<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        bit_exp_targets: &[Target; B],
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        assert!(B <= 64);

        let mut result = builder.zero();
        for (bit, exp) in bits.0.chunks(8).rev().flatten().zip(bit_exp_targets.iter()) {
            result = builder.mul_add(bit.target, *exp, result);
        }
        result
    }

    fn from_bool_targets_le<const B: usize>(
        bits: ArrayTarget<BoolTarget, B>,
        builder: &mut CircuitBuilder<F, D>,
    ) -> Target {
        let bit_exp_targets = (0..B)
            .rev()
            .map(|bit_no| builder.constant(GoldilocksField::from_noncanonical_u64(1 << bit_no)))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        Self::from_bool_targets_le_precomputed_exp(bits, &bit_exp_targets, builder)
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
            Target::from_u64_bits_le_lossy(ArrayTarget(bit_targets), &mut builder);
        builder.register_public_input(resulting_target);

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
