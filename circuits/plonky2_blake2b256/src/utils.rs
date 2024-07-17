use plonky2::hash::hash_types::RichField;
use plonky2::iop::target::BoolTarget;
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::{field::extension::Extendable, iop::target::Target};
use plonky2_u32::gadgets::arithmetic_u32::{CircuitBuilderU32, U32Target};

pub type Word = u64;
pub const WORD_BITS: usize = 64;

/// Little-endian representation of `Word` as `BoolTarget`s.
pub type WordTargets = [BoolTarget; WORD_BITS];

pub trait CircuitBuilderExt {
    fn not_word(&mut self, w: WordTargets) -> WordTargets;

    fn triple_xor_words(&mut self, a: WordTargets, b: WordTargets, c: WordTargets) -> WordTargets {
        let a_b = self.xor_words(a, b);
        self.xor_words(a_b, c)
    }

    fn xor_words(&mut self, a: WordTargets, b: WordTargets) -> WordTargets;

    fn xor(&mut self, a: BoolTarget, b: BoolTarget) -> BoolTarget;

    fn triple_add_words_wrapping(
        &mut self,
        a: WordTargets,
        b: WordTargets,
        c: WordTargets,
    ) -> WordTargets {
        let a_b = self.add_words_wrapping(a, b);
        self.add_words_wrapping(a_b, c)
    }

    fn add_words_wrapping(&mut self, a: WordTargets, b: WordTargets) -> WordTargets;

    fn split_target_to_word_targets(&mut self, target: Target) -> WordTargets;
}

impl<F: RichField + Extendable<D>, const D: usize> CircuitBuilderExt for CircuitBuilder<F, D> {
    fn not_word(&mut self, w: WordTargets) -> WordTargets {
        w.map(|w| self.not(w))
    }

    fn xor_words(&mut self, a: WordTargets, b: WordTargets) -> WordTargets {
        a.into_iter()
            .zip(b)
            .map(|(a, b)| self.xor(a, b))
            .collect::<Vec<_>>()
            .try_into()
            .expect("Correct bit count")
    }

    // !(!a & !b) & !(a & b)
    fn xor(&mut self, a: BoolTarget, b: BoolTarget) -> BoolTarget {
        let not_a = self.not(a);
        let not_b = self.not(b);

        let c = self.and(not_a, not_b);
        let c = self.not(c);
        let d = self.and(a, b);
        let d = self.not(d);
        self.and(c, d)
    }

    fn add_words_wrapping(&mut self, a: WordTargets, b: WordTargets) -> WordTargets {
        // Assert that word can be represented as 2x u32.
        static_assertions::const_assert_eq!(WORD_BITS, 32 * 2);

        let (a_l, a_h) = split_word(a);
        let a_l = U32Target(self.le_sum(a_l.into_iter()));
        let a_h = U32Target(self.le_sum(a_h.into_iter()));

        let (b_l, b_h) = split_word(b);
        let b_l = U32Target(self.le_sum(b_l.into_iter()));
        let b_h = U32Target(self.le_sum(b_h.into_iter()));

        let (res_l, carry) = self.add_u32(a_l, b_l);
        let (res_h, _) = self.add_u32s_with_carry(&[a_h, b_h], carry);

        let res_l = self
            .split_le(res_l.0, WORD_BITS / 2)
            .try_into()
            .expect("WORD_BITS / 2 bits");
        let res_h = self
            .split_le(res_h.0, WORD_BITS / 2)
            .try_into()
            .expect("WORD_BITS / 2 bits");

        collect_word(res_l, res_h)
    }

    fn split_target_to_word_targets(&mut self, target: Target) -> WordTargets {
        self.split_le(target, WORD_BITS)
            .chunks(8)
            .flat_map(|bits| bits.iter().rev())
            .cloned()
            .collect::<Vec<_>>()
            .try_into()
            .expect("Correct word bit count")
    }
}

fn collect_word(l: [BoolTarget; WORD_BITS / 2], h: [BoolTarget; WORD_BITS / 2]) -> WordTargets {
    l.into_iter()
        .chain(h)
        .collect::<Vec<_>>()
        .chunks(8)
        .flat_map(|bits| bits.iter().rev())
        .copied()
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct bit count")
}

fn split_word(w: WordTargets) -> ([BoolTarget; WORD_BITS / 2], [BoolTarget; WORD_BITS / 2]) {
    let w_bits: Vec<_> = w
        .chunks(8)
        .flat_map(|bits| bits.iter().rev())
        .copied()
        .collect();

    let (a_l_bits, a_h_bits) = w_bits.split_at(WORD_BITS / 2);

    (
        a_l_bits.to_vec().try_into().expect("Correct bit count"),
        a_h_bits.to_vec().try_into().expect("Correct bit count"),
    )
}

pub fn rright_word(inp: WordTargets, amount: usize) -> WordTargets {
    assert!(amount > 0 && amount < WORD_BITS);

    let mut inp: WordTargets = inp
        .chunks(8)
        .rev()
        .flatten()
        .copied()
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct bit count");

    inp.rotate_right(amount);

    inp.chunks(8)
        .rev()
        .flatten()
        .copied()
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct bit count")
}

pub fn word_array_to_word_targets<const N: usize, F: RichField + Extendable<D>, const D: usize>(
    arr: [Word; N],
    builder: &mut CircuitBuilder<F, D>,
) -> [WordTargets; N] {
    arr.map(|w| word_to_bits_le(w).map(|bit| builder.constant_bool(bit)))
}

fn word_to_bits_le(w: Word) -> [bool; WORD_BITS] {
    w.to_le_bytes()
        .into_iter()
        .flat_map(|byte| (0..8).rev().map(move |bit_idx| (byte >> bit_idx) % 2 == 1))
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct amount of bits")
}

#[cfg(test)]
mod tests {
    use super::*;

    use plonky2::field::goldilocks_field::GoldilocksField;
    use plonky2::iop::witness::{PartialWitness, WitnessWrite};
    use plonky2::plonk::{circuit_data::CircuitConfig, config::PoseidonGoldilocksConfig};

    type F = GoldilocksField;
    type C = PoseidonGoldilocksConfig;
    const D: usize = 2;

    #[test]
    fn test_rright_word() {
        let input = vec![
            (4, 1),
            (1, 10),
            (1 + (1 << 63), 1),
            ((1 << 63) + (1 << 15), 10),
        ];

        for inp in input {
            test_rright_word_inner(inp.0, inp.1);
        }
    }

    fn test_rright_word_inner(val: u64, shift: usize) {
        let a_rotate = (val >> shift) | (val << (64 - shift));
        let a_rotate_bits = word_to_bits_le(a_rotate);

        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());
        let mut pw = PartialWitness::new();

        let a_targets = word_array_to_word_targets([val], &mut builder)[0];

        let a_bits = word_to_bits_le(val);
        for i in 0..WORD_BITS {
            builder.register_public_input(a_targets[i].target);
            pw.set_bool_target(a_targets[i], a_bits[i]);
        }

        let a_rotate_targets = rright_word(a_targets, shift);
        for target in a_rotate_targets {
            builder.register_public_input(target.target);
        }

        let data = builder.build::<C>();
        let proof = data.prove(pw).expect("Proven succesfully");

        let a_rotate_res_bits = &proof.public_inputs[WORD_BITS..WORD_BITS * 2];

        for i in 0..WORD_BITS {
            assert_eq!(a_rotate_res_bits[i].0, a_rotate_bits[i] as u64);
        }
    }

    #[test]
    fn test_add_words_wrapping() {
        let inputs = vec![
            (1, 1, 2),
            (200, 300, 500),
            (u32::MAX as u64, u32::MAX as u64, 2 * (u32::MAX as u64)),
            (u64::MAX, 1234, 1233),
        ];

        for inp in inputs {
            test_add_words_wrapping_internal(inp.0, inp.1, inp.2);
        }
    }

    fn test_add_words_wrapping_internal(a: u64, b: u64, c: u64) {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());
        let mut pw = PartialWitness::new();

        let a_targets = word_array_to_word_targets([a], &mut builder)[0];
        let a_bits = word_to_bits_le(a);
        for i in 0..WORD_BITS {
            builder.register_public_input(a_targets[i].target);
            pw.set_bool_target(a_targets[i], a_bits[i]);
        }

        let b_targets = word_array_to_word_targets([b], &mut builder)[0];
        let b_bits = word_to_bits_le(b);
        for i in 0..WORD_BITS {
            builder.register_public_input(b_targets[i].target);
            pw.set_bool_target(b_targets[i], b_bits[i]);
        }

        let c_targets = builder.add_words_wrapping(a_targets, b_targets);
        for target in c_targets {
            builder.register_public_input(target.target);
        }

        let data = builder.build::<C>();
        let proof = data.prove(pw).expect("Proven succesfully");

        let result_bits = &proof.public_inputs[WORD_BITS * 2..WORD_BITS * 3];
        let c_bits = word_to_bits_le(c);

        for i in 0..WORD_BITS {
            assert_eq!(result_bits[i].0, c_bits[i] as u64);
        }
    }
}
