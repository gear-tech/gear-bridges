#[macro_export]
macro_rules! test_field_arithmetic {
    ($field:ty) => {
        mod field_arithmetic {
            use std::vec::Vec;

            use num::bigint::BigUint;
            use plonky2_field::types::{Field, Sample};
            use rand::{rngs::OsRng, Rng};

            #[test]
            fn batch_inversion() {
                for n in 0..20 {
                    let xs = (1..=n as u64)
                        .map(|i| <$field>::from_canonical_u64(i))
                        .collect::<Vec<_>>();
                    let invs = <$field>::batch_multiplicative_inverse(&xs);
                    assert_eq!(invs.len(), n);
                    for (x, inv) in xs.into_iter().zip(invs) {
                        assert_eq!(x * inv, <$field>::ONE);
                    }
                }
            }

            #[test]
            fn primitive_root_order() {
                let max_power = 8.min(<$field>::TWO_ADICITY);
                for n_power in 0..max_power {
                    let root = <$field>::primitive_root_of_unity(n_power);
                    let order = <$field>::generator_order(root);
                    assert_eq!(order, 1 << n_power, "2^{}'th primitive root", n_power);
                }
            }

            #[test]
            fn negation() {
                type F = $field;

                for x in [F::ZERO, F::ONE, F::TWO, F::NEG_ONE] {
                    assert_eq!(x + -x, F::ZERO);
                }
            }

            #[test]
            fn exponentiation() {
                type F = $field;

                assert_eq!(F::ZERO.exp_u64(0), <F>::ONE);
                assert_eq!(F::ONE.exp_u64(0), <F>::ONE);
                assert_eq!(F::TWO.exp_u64(0), <F>::ONE);

                assert_eq!(F::ZERO.exp_u64(1), <F>::ZERO);
                assert_eq!(F::ONE.exp_u64(1), <F>::ONE);
                assert_eq!(F::TWO.exp_u64(1), <F>::TWO);

                assert_eq!(F::ZERO.kth_root_u64(1), <F>::ZERO);
                assert_eq!(F::ONE.kth_root_u64(1), <F>::ONE);
                assert_eq!(F::TWO.kth_root_u64(1), <F>::TWO);

                for power in 1..10 {
                    if F::is_monomial_permutation_u64(power) {
                        let x = F::rand();
                        assert_eq!(x.exp_u64(power).kth_root_u64(power), x);
                    }
                }
            }

            #[test]
            fn exponentiation_large() {
                type F = $field;

                let mut rng = OsRng;

                let base = F::rand();
                let pow = BigUint::from(rng.gen::<u64>());
                let cycles = rng.gen::<u32>();
                let mul_group_order = F::order() - 1u32;
                let big_pow = &pow + &mul_group_order * cycles;
                let big_pow_wrong = &pow + &mul_group_order * cycles + 1u32;

                assert_eq!(base.exp_biguint(&pow), base.exp_biguint(&big_pow));
                assert_ne!(base.exp_biguint(&pow), base.exp_biguint(&big_pow_wrong));
            }

            #[test]
            fn inverses() {
                type F = $field;

                let x = F::rand();
                let x1 = x.inverse();
                let x2 = x1.inverse();
                let x3 = x2.inverse();

                assert_eq!(x, x2);
                assert_eq!(x1, x3);
            }
        }
    };
}

#[macro_export]
macro_rules! test_field_extension {
    ($field:ty, $d:expr) => {
        mod field_extension {
            #[test]
            fn test_add_neg_sub_mul() {
                $crate::field_testing::test_add_neg_sub_mul::<$field, $d>();
            }
            #[test]
            fn test_inv_div() {
                $crate::field_testing::test_inv_div::<$field, $d>();
            }
            #[test]
            fn test_frobenius() {
                $crate::field_testing::test_frobenius::<$field, $d>();
            }
            #[test]
            fn test_field_order() {
                $crate::field_testing::test_field_order::<$field, $d>();
            }
            #[test]
            fn test_power_of_two_gen() {
                $crate::field_testing::test_power_of_two_gen::<$field, $d>();
            }
        }
    };
}
