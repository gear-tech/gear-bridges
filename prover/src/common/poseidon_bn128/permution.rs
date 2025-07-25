use super::{
    constants::{C_CONSTANTS, M_MATRIX, P_MATRIX, S_CONSTANTS},
    fr::Fr,
};
use ff::Field;
use std::ops::{AddAssign, MulAssign};

pub const RATE: usize = 3;
pub const WIDTH: usize = 4;
pub const FULL_ROUNDS: usize = 8;
pub const PARTIAL_ROUNDS: usize = 56;
pub const GOLDILOCKS_ELEMENTS: usize = 3;

pub type PoseidonState = [Fr; WIDTH];

// This poseidon BN128 implementation is based on the following implementation:
// https://github.com/iden3/go-iden3-crypto/blob/e5cf066b8be3da9a3df9544c65818df189fdbebe/poseidon/poseidon.go
pub fn permution(state: &mut PoseidonState) {
    ark(state, 0);
    full_rounds(state, true);
    partial_rounds(state);
    full_rounds(state, false);
}

fn ark(state: &mut PoseidonState, it: usize) {
    for i in 0..WIDTH {
        state[i].add_assign(&C_CONSTANTS[it + i]);
    }
}

fn exp5(mut x: Fr) -> Fr {
    let aux = x;
    x = x.square();
    x = x.square();
    x.mul_assign(&aux);

    x
}

fn exp5_state(state: &mut PoseidonState) {
    for state_element in state.iter_mut().take(WIDTH) {
        *state_element = exp5(*state_element);
    }
}

fn full_rounds(state: &mut PoseidonState, first: bool) {
    for i in 0..FULL_ROUNDS / 2 - 1 {
        exp5_state(state);
        if first {
            ark(state, (i + 1) * WIDTH);
        } else {
            ark(
                state,
                (FULL_ROUNDS / 2 + 1) * WIDTH + PARTIAL_ROUNDS + i * WIDTH,
            );
        }
        mix(state, &M_MATRIX);
    }

    exp5_state(state);
    if first {
        ark(state, (FULL_ROUNDS / 2) * WIDTH);
        mix(state, &P_MATRIX);
    } else {
        mix(state, &M_MATRIX);
    }
}

fn partial_rounds(state: &mut PoseidonState) {
    for i in 0..PARTIAL_ROUNDS {
        state[0] = exp5(state[0]);
        state[0].add_assign(&C_CONSTANTS[(FULL_ROUNDS / 2 + 1) * WIDTH + i]);

        let mut mul;
        let mut new_state0 = Fr::ZERO;
        for j in 0..WIDTH {
            mul = Fr::ZERO;
            mul.add_assign(&S_CONSTANTS[(WIDTH * 2 - 1) * i + j]);
            mul.mul_assign(&state[j]);
            new_state0.add_assign(&mul);
        }

        for k in 1..WIDTH {
            mul = Fr::ZERO;
            mul.add_assign(&state[0]);
            mul.mul_assign(&S_CONSTANTS[(WIDTH * 2 - 1) * i + WIDTH + k - 1]);
            state[k].add_assign(&mul);
        }

        state[0] = new_state0;
    }
}

fn mix(state: &mut PoseidonState, constant_matrix: &[Vec<Fr>]) {
    let mut result: PoseidonState = [Fr::ZERO; WIDTH];

    let mut mul;
    for (i, result_element) in result.iter_mut().enumerate().take(WIDTH) {
        for j in 0..WIDTH {
            mul = Fr::ZERO;
            mul.add_assign(&constant_matrix[j][i]);
            mul.mul_assign(&state[j]);
            result_element.add_assign(&mul);
        }
    }

    state[..WIDTH].copy_from_slice(&result[..WIDTH]);
}

#[cfg(test)]
mod permutation_tests {
    use anyhow::Ok;
    use ff::{Field, PrimeField};

    use super::{permution, Fr, WIDTH};

    #[test]
    fn test_permuation() -> Result<(), anyhow::Error> {
        // Test inputs are:
        // 1. all zeros
        // 2. range 0..WIDTH
        // 3. all max BN128 values
        // 4. random elements of BN128.
        // Expected output calculated from this poseidon implementation:  https://github.com/iden3/go-iden3-crypto/blob/master/poseidon/poseidon.go#L65

        let max_value: Fr = Fr::from_str_vartime(
            "21888242871839275222246405745257275088548364400416034343698204186575808495616",
        )
        .unwrap();

        let test_vectors: Vec<([Fr; 4], [Fr; 4])> = vec![
            (
                [Fr::ZERO; 4],
                [
                    Fr::from_str_vartime("5317387130258456662214331362918410991734007599705406860481038345552731150762").unwrap(),
                    Fr::from_str_vartime("17768273200467269691696191901389126520069745877826494955630904743826040320364").unwrap(),
                    Fr::from_str_vartime("19413739268543925182080121099097652227979760828059217876810647045303340666757").unwrap(),
                    Fr::from_str_vartime("3717738800218482999400886888123026296874264026760636028937972004600663725187").unwrap(),
                ]
            ),
            (
                [
                    Fr::from_str_vartime("0").unwrap(),
                    Fr::from_str_vartime("1").unwrap(),
                    Fr::from_str_vartime("2").unwrap(),
                    Fr::from_str_vartime("3").unwrap(),
                ],
                [
                    Fr::from_str_vartime("6542985608222806190361240322586112750744169038454362455181422643027100751666").unwrap(),
                    Fr::from_str_vartime("3478427836468552423396868478117894008061261013954248157992395910462939736589").unwrap(),
                    Fr::from_str_vartime("1904980799580062506738911865015687096398867595589699208837816975692422464009").unwrap(),
                    Fr::from_str_vartime("11971464497515232077059236682405357499403220967704831154657374522418385384151").unwrap(),
                ]
            ),
            (
                [max_value; 4],
                [
                    Fr::from_str_vartime("13055670547682322550638362580666986963569035646873545133474324633020685301274").unwrap(),
                    Fr::from_str_vartime("19087936485076376314486368416882351797015004625427655501762827988254486144933").unwrap(),
                    Fr::from_str_vartime("10391468779200270580383536396630001155994223659670674913170907401637624483385").unwrap(),
                    Fr::from_str_vartime("17202557688472898583549180366140168198092766974201433936205272956998081177816").unwrap(),
                ]
            ),
            (
                [
                    Fr::from_str_vartime("6542985608222806190361240322586112750744169038454362455181422643027100751666").unwrap(),
                    Fr::from_str_vartime("3478427836468552423396868478117894008061261013954248157992395910462939736589").unwrap(),
                    Fr::from_str_vartime("1904980799580062506738911865015687096398867595589699208837816975692422464009").unwrap(),
                    Fr::from_str_vartime("11971464497515232077059236682405357499403220967704831154657374522418385384151").unwrap(),
                ],
                [
                    Fr::from_str_vartime("21792249080447013894140672594027696524030291802493510986509431008224624594361").unwrap(),
                    Fr::from_str_vartime("3536096706123550619294332177231935214243656967137545251021848527424156573335").unwrap(),
                    Fr::from_str_vartime("14869351042206255711434675256184369368509719143073814271302931417334356905217").unwrap(),
                    Fr::from_str_vartime("5027523131326906886284185656868809493297314443444919363729302983434650240523").unwrap(),
                ]
            ),
        ];

        for (mut input, expected_output) in test_vectors.into_iter() {
            permution(&mut input);
            for i in 0..WIDTH {
                assert_eq!(input[i], expected_output[i]);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod merkle_tree_tests {
    use anyhow::Result;
    use plonky2::{
        field::extension::Extendable,
        hash::{
            hash_types::RichField, merkle_proofs::verify_merkle_proof_to_cap,
            merkle_tree::MerkleTree,
        },
        plonk::config::GenericConfig,
    };

    use super::super::super::PoseidonBN128GoldilocksConfig;

    fn random_data<F: RichField>(n: usize, k: usize) -> Vec<Vec<F>> {
        (0..n).map(|_| F::rand_vec(k)).collect()
    }

    fn verify_all_leaves<
        F: RichField + Extendable<D>,
        C: GenericConfig<D, F = F>,
        const D: usize,
    >(
        leaves: Vec<Vec<F>>,
        cap_height: usize,
    ) -> Result<()> {
        let tree = MerkleTree::<F, C::Hasher>::new(leaves.clone(), cap_height);
        for (i, leaf) in leaves.into_iter().enumerate() {
            let proof = tree.prove(i);
            verify_merkle_proof_to_cap(leaf, i, &tree.cap, &proof)?;
        }
        Ok(())
    }

    #[test]
    #[should_panic]
    fn test_cap_height_too_big() {
        const D: usize = 2;
        type C = PoseidonBN128GoldilocksConfig;
        type F = <C as GenericConfig<D>>::F;

        let log_n = 8;
        let cap_height = log_n + 1; // Should panic if `cap_height > len_n`.

        let leaves = random_data::<F>(1 << log_n, 7);
        let _ = MerkleTree::<F, <C as GenericConfig<D>>::Hasher>::new(leaves, cap_height);
    }

    #[test]
    fn test_cap_height_eq_log2_len() -> Result<()> {
        const D: usize = 2;
        type C = PoseidonBN128GoldilocksConfig;
        type F = <C as GenericConfig<D>>::F;

        let log_n = 8;
        let n = 1 << log_n;
        let leaves = random_data::<F>(n, 7);

        verify_all_leaves::<F, C, D>(leaves, log_n)?;

        Ok(())
    }

    #[test]
    fn test_merkle_trees() -> Result<()> {
        const D: usize = 2;
        type C = PoseidonBN128GoldilocksConfig;
        type F = <C as GenericConfig<D>>::F;

        let log_n = 8;
        let n = 1 << log_n;
        let leaves = random_data::<F>(n, 7);

        verify_all_leaves::<F, C, D>(leaves, 1)?;

        Ok(())
    }
}
