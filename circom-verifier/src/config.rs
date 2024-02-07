use plonky2::field::extension::quadratic::QuadraticExtension;
use plonky2::field::extension::Extendable;
use plonky2::field::goldilocks_field::GoldilocksField;

use plonky2::field::types::PrimeField64;
use plonky2::hash::hash_types::{HashOut, HashOutTarget, RichField};
use plonky2::hash::hashing::{compress, hash_n_to_hash_no_pad, PlonkyPermutation};
use plonky2::hash::poseidon::{PoseidonHash, PoseidonPermutation};
use plonky2::iop::target::{BoolTarget, Target};
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::plonk::config::{AlgebraicHasher, GenericConfig, Hasher};
use poseidon_permutation::bindings::permute;
use std::fmt::Debug;

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct PoseidonBN128Permutation<T: Eq> {
    // TODO: remove const
    state: [T; 12],
}

impl<T: Eq> AsRef<[T]> for PoseidonBN128Permutation<T> {
    fn as_ref(&self) -> &[T] {
        &self.state
    }
}

impl PlonkyPermutation<GoldilocksField> for PoseidonBN128Permutation<GoldilocksField> {
    // TODO: remove consts
    const RATE: usize = 8;
    const WIDTH: usize = 12;

    fn permute(&mut self) {
        assert_eq!(Self::WIDTH, 12);
        unsafe {
            let h = permute(
                self.state[0].to_canonical_u64(),
                self.state[1].to_canonical_u64(),
                self.state[2].to_canonical_u64(),
                self.state[3].to_canonical_u64(),
                self.state[4].to_canonical_u64(),
                self.state[5].to_canonical_u64(),
                self.state[6].to_canonical_u64(),
                self.state[7].to_canonical_u64(),
                self.state[8].to_canonical_u64(),
                self.state[9].to_canonical_u64(),
                self.state[10].to_canonical_u64(),
                self.state[11].to_canonical_u64(),
            );

            fn u64_to_f<F: RichField>(x: u64) -> F {
                let x = if x >= F::ORDER { x - F::ORDER } else { x };
                F::from_canonical_u64(x)
            }

            self.state = [
                u64_to_f(h.r0),
                u64_to_f(h.r1),
                u64_to_f(h.r2),
                u64_to_f(h.r3),
                u64_to_f(h.r4),
                u64_to_f(h.r5),
                u64_to_f(h.r6),
                u64_to_f(h.r7),
                u64_to_f(h.r8),
                u64_to_f(h.r9),
                u64_to_f(h.r10),
                u64_to_f(h.r11),
            ];
        }
    }

    fn new<I: IntoIterator<Item = GoldilocksField>>(iter: I) -> Self {
        Self {
            state: iter
                .into_iter()
                .take(12)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        }
    }

    fn set_elt(&mut self, elt: GoldilocksField, idx: usize) {
        self.state[idx] = elt;
    }

    fn set_from_iter<I: IntoIterator<Item = GoldilocksField>>(
        &mut self,
        elts: I,
        start_idx: usize,
    ) {
        for (s, e) in self.state[start_idx..].iter_mut().zip(elts) {
            *s = e;
        }
    }

    fn set_from_slice(&mut self, elts: &[GoldilocksField], start_idx: usize) {
        let begin = start_idx;
        let end = start_idx + elts.len();
        self.state[begin..end].copy_from_slice(elts);
    }

    fn squeeze(&self) -> &[GoldilocksField] {
        &self.state[..Self::RATE]
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PoseidonBN128Hash;
impl Hasher<GoldilocksField> for PoseidonBN128Hash {
    const HASH_SIZE: usize = 4 * 8;
    type Hash = HashOut<GoldilocksField>;
    type Permutation = PoseidonBN128Permutation<GoldilocksField>;

    fn hash_no_pad(input: &[GoldilocksField]) -> Self::Hash {
        hash_n_to_hash_no_pad::<GoldilocksField, Self::Permutation>(input)
    }

    fn hash_public_inputs(input: &[GoldilocksField]) -> Self::Hash {
        PoseidonHash::hash_no_pad(input)
    }

    fn two_to_one(left: Self::Hash, right: Self::Hash) -> Self::Hash {
        compress::<GoldilocksField, Self::Permutation>(left, right)
    }
}

// TODO: this is a work around. Still use Goldilocks based Poseidon for algebraic PoseidonBN128Hash.
impl AlgebraicHasher<GoldilocksField> for PoseidonBN128Hash {
    type AlgebraicPermutation = PoseidonPermutation<Target>;

    fn permute_swapped<const D: usize>(
        inputs: PoseidonPermutation<Target>,
        swap: BoolTarget,
        builder: &mut CircuitBuilder<GoldilocksField, D>,
    ) -> PoseidonPermutation<Target>
    where
        GoldilocksField: Extendable<D>,
    {
        PoseidonHash::permute_swapped(inputs, swap, builder)
    }

    fn public_inputs_hash<const D: usize>(
        inputs: Vec<Target>,
        builder: &mut CircuitBuilder<GoldilocksField, D>,
    ) -> HashOutTarget
    where
        GoldilocksField: RichField + Extendable<D>,
    {
        PoseidonHash::public_inputs_hash(inputs, builder)
    }
}

/// Configuration using Poseidon over the Goldilocks field.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct PoseidonBN128GoldilocksConfig;

impl GenericConfig<2> for PoseidonBN128GoldilocksConfig {
    type F = GoldilocksField;
    type FE = QuadraticExtension<Self::F>;
    type Hasher = PoseidonBN128Hash;
    type InnerHasher = PoseidonBN128Hash;
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use plonky2::field::types::Field;
    use plonky2::hash::poseidon::{Poseidon, PoseidonHash, PoseidonPermutation};
    use plonky2::plonk::config::{GenericConfig, Hasher, PoseidonGoldilocksConfig};

    use super::super::config::PoseidonBN128Hash;

    #[test]
    fn test_poseidon_bn128() -> Result<()> {
        const D: usize = 2;
        type C = PoseidonGoldilocksConfig;
        type F = <C as GenericConfig<D>>::F;

        let mut v = Vec::new();
        v.push(F::from_canonical_u64(8917524657281059100u64));
        v.push(F::from_canonical_u64(13029010200779371910u64));
        v.push(F::from_canonical_u64(16138660518493481604u64));
        v.push(F::from_canonical_u64(17277322750214136960u64));
        v.push(F::from_canonical_u64(1441151880423231822u64));
        let h = PoseidonBN128Hash::hash_no_pad(&v);
        assert_eq!(h.elements[0].0, 16736853722845225729u64);
        assert_eq!(h.elements[1].0, 1446699130810517790u64);
        assert_eq!(h.elements[2].0, 15445626857806971868u64);
        assert_eq!(h.elements[3].0, 6331160477881736675u64);

        Ok(())
    }
}
