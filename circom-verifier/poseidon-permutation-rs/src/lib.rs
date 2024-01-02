#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

pub mod bindings;

#[cfg(test)]
mod tests {
    use bindings::{permute};

    #[test]
    fn poseidon_permutation_test() {
        unsafe {
            let res = permute(
                8917524657281059100u64,
                13029010200779371910u64,
                16138660518493481604u64,
                17277322750214136960u64,
                1441151880423231822u64,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            );
            assert_eq!(res.r0, 16736853722845225729u64);
            assert_eq!(res.r1, 1446699130810517790u64);
            assert_eq!(res.r2, 15445626857806971868u64);
            assert_eq!(res.r3, 6331160477881736675u64);
        }
    }
}
