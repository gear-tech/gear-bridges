use std::marker::PhantomData;

use itertools::Itertools;
use plonky2::{
    hash::hash_types::RichField, iop::target::Target, plonk::circuit_builder::CircuitBuilder,
};
use plonky2_field::{extension::Extendable, types::Field};
use plonky2_u32::gadgets::arithmetic_u32::{CircuitBuilderU32, U32Target};

use crate::gadgets::nonnative::NonNativeTarget;
use plonky2_ecdsa::gadgets::biguint::BigUintTarget;

pub trait CircuitBuilderSplit<F: RichField + Extendable<D>, const D: usize> {
    fn split_u32_to_4_bit_limbs(&mut self, val: U32Target) -> Vec<Target>;

    fn split_nonnative_to_4_bit_limbs<FF: Field>(
        &mut self,
        val: &NonNativeTarget<FF>,
    ) -> Vec<Target>;

    fn split_nonnative_to_2_bit_limbs<FF: Field>(
        &mut self,
        val: &NonNativeTarget<FF>,
    ) -> Vec<Target>;

    // Note: assumes its inputs are 4-bit limbs, and does not range-check.
    fn recombine_nonnative_4_bit_limbs<FF: Field>(
        &mut self,
        limbs: Vec<Target>,
    ) -> NonNativeTarget<FF>;
}

impl<F: RichField + Extendable<D>, const D: usize> CircuitBuilderSplit<F, D>
    for CircuitBuilder<F, D>
{
    fn split_u32_to_4_bit_limbs(&mut self, val: U32Target) -> Vec<Target> {
        let two_bit_limbs = self.split_le_base::<4>(val.0, 16);
        let four = self.constant(F::from_canonical_usize(4));
        let combined_limbs = two_bit_limbs
            .iter()
            .tuples()
            .map(|(&a, &b)| self.mul_add(b, four, a))
            .collect();

        combined_limbs
    }

    fn split_nonnative_to_4_bit_limbs<FF: Field>(
        &mut self,
        val: &NonNativeTarget<FF>,
    ) -> Vec<Target> {
        val.value
            .limbs
            .iter()
            .flat_map(|&l| self.split_u32_to_4_bit_limbs(l))
            .collect()
    }

    fn split_nonnative_to_2_bit_limbs<FF: Field>(
        &mut self,
        val: &NonNativeTarget<FF>,
    ) -> Vec<Target> {
        val.value
            .limbs
            .iter()
            .flat_map(|&l| self.split_le_base::<4>(l.0, 16))
            .collect()
    }

    // Note: assumes its inputs are 4-bit limbs, and does not range-check.
    fn recombine_nonnative_4_bit_limbs<FF: Field>(
        &mut self,
        limbs: Vec<Target>,
    ) -> NonNativeTarget<FF> {
        let base = self.constant_u32(1 << 4);
        let u32_limbs = limbs
            .chunks(8)
            .map(|chunk| {
                let mut combined_chunk = self.zero_u32();
                for i in (0..8).rev() {
                    let (low, _high) = self.mul_add_u32(combined_chunk, base, U32Target(chunk[i]));
                    combined_chunk = low;
                }
                combined_chunk
            })
            .collect();

        NonNativeTarget {
            value: BigUintTarget { limbs: u32_limbs },
            _phantom: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use plonky2::{
        iop::witness::PartialWitness,
        plonk::{
            circuit_builder::CircuitBuilder,
            circuit_data::CircuitConfig,
            config::{GenericConfig, PoseidonGoldilocksConfig},
        },
    };
    use plonky2_field::types::Sample;

    use crate::{
        field::ed25519_scalar::Ed25519Scalar,
        gadgets::{
            nonnative::{CircuitBuilderNonNative, NonNativeTarget},
            split_nonnative::CircuitBuilderSplit,
        },
    };

    #[test]
    fn test_split_nonnative() -> Result<()> {
        type FF = Ed25519Scalar;
        const D: usize = 2;
        type C = PoseidonGoldilocksConfig;
        type F = <C as GenericConfig<D>>::F;

        let config = CircuitConfig::standard_ecc_config();
        let pw = PartialWitness::new();
        let mut builder = CircuitBuilder::<F, D>::new(config);

        let x = FF::rand();
        let x_target = builder.constant_nonnative(x);
        let split = builder.split_nonnative_to_4_bit_limbs(&x_target);
        let combined: NonNativeTarget<Ed25519Scalar> =
            builder.recombine_nonnative_4_bit_limbs(split);
        builder.connect_nonnative(&x_target, &combined);

        let data = builder.build::<C>();
        let proof = data.prove(pw).unwrap();
        data.verify(proof)
    }
}
