//! Blake2b-256 plonky2 circuit.
//!
//! Implementation is based on RFC7693
//! https://datatracker.ietf.org/doc/html/rfc7693.

use plonky2::{
    field::extension::Extendable,
    hash::hash_types::RichField,
    iop::target::{BoolTarget, Target},
    plonk::circuit_builder::CircuitBuilder,
};

use crate::utils::*;

const IV: [Word; 8] = [
    0x6A09E667F3BCC908,
    0xBB67AE8584CAA73B,
    0x3C6EF372FE94F82B,
    0xA54FF53A5F1D36F1,
    0x510E527FADE682D1,
    0x9B05688C2B3E6C1F,
    0x1F83D9ABFB41BD6B,
    0x5BE0CD19137E2179,
];

const SIGMA: [[usize; 16]; 12] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
];

const R1: usize = 32;
const R2: usize = 24;
const R3: usize = 16;
const R4: usize = 63;

pub const BLOCK_BYTES: usize = 128;
pub const BLOCK_BITS: usize = BLOCK_BYTES * 8;
const BLOCK_WORDS: usize = 16;

const INTERNAL_STATE_WORDS: usize = 8;
const V_WORDS: usize = 16;

const HASH_BYTES: u8 = 32;
const HASH_BITS: usize = HASH_BYTES as usize * 8;
const KEY_BYTES: u8 = 0;

#[derive(Debug, Clone)]
pub struct Blake2Targets {
    pub message: Vec<BoolTarget>,
    pub digest: [BoolTarget; HASH_BITS],
}

pub fn blake2_circuit<F: RichField + Extendable<D>, const D: usize>(
    builder: &mut CircuitBuilder<F, D>,
    msg_len: usize,
) -> Blake2Targets {
    let message = std::iter::repeat_n((), msg_len * 8)
        .map(|_| builder.add_virtual_bool_target_safe())
        .collect::<Vec<_>>();

    let digest = blake2_circuit_from_targets(builder, message.clone());

    for bit in &message {
        builder.register_public_input(bit.target);
    }

    for bit in &digest {
        builder.register_public_input(bit.target);
    }

    Blake2Targets { message, digest }
}

pub fn blake2_circuit_from_targets<F: RichField + Extendable<D>, const D: usize>(
    builder: &mut CircuitBuilder<F, D>,
    message: Vec<BoolTarget>,
) -> [BoolTarget; HASH_BITS] {
    assert!(message.len().is_multiple_of(8));
    let length = builder.constant(F::from_canonical_usize(message.len() / 8));
    blake2_circuit_from_message_targets_and_length_target(builder, message, length)
}

/// Calling side is responsible of controlling `length`.
pub fn blake2_circuit_from_message_targets_and_length_target<
    F: RichField + Extendable<D>,
    const D: usize,
>(
    builder: &mut CircuitBuilder<F, D>,
    mut message: Vec<BoolTarget>,
    length: Target,
) -> [BoolTarget; HASH_BITS] {
    assert!(message.len().is_multiple_of(8));

    let msg_len_in_bits = message.len();
    let dd = msg_len_in_bits / BLOCK_BITS + !msg_len_in_bits.is_multiple_of(BLOCK_BITS) as usize;
    let dd = dd.max(1);

    let msg_len_rem = (dd * BLOCK_BITS) - msg_len_in_bits;

    let mut message_rem = std::iter::repeat(())
        .map(|_| builder.constant_bool(false))
        .take(msg_len_rem)
        .collect();
    message.append(&mut message_rem);

    let message_blocks: Vec<[WordTargets; BLOCK_WORDS]> = message
        .chunks(BLOCK_BITS)
        .map(|block| {
            block
                .to_vec()
                .chunks(WORD_BITS)
                .map(|bits| bits.to_vec().try_into().expect("Chunks of correct size"))
                .collect::<Vec<_>>()
                .try_into()
                .expect("Chunks of correct size")
        })
        .collect();

    assert_eq!(message_blocks.len(), dd);

    let mut initial_h_value = IV;
    initial_h_value[0] =
        initial_h_value[0] ^ 0x01010000 ^ ((KEY_BYTES as Word) << 8) ^ HASH_BYTES as Word;

    let mut h = word_array_to_word_targets(initial_h_value, builder);
    let iv = word_array_to_word_targets(IV, builder);

    #[allow(clippy::needless_range_loop)]
    for i in 0..dd - 1 {
        let t = builder.constant(F::from_canonical_usize((i + 1) * BLOCK_BYTES));
        h = F(builder, &iv, h, &message_blocks[i], t, false);
    }

    if KEY_BYTES == 0 {
        let t = length;
        h = F(builder, &iv, h, &message_blocks[dd - 1], t, true);
    } else {
        unimplemented!("Hashing with key is not implemented");
    }

    h.into_iter()
        .flatten()
        .take(HASH_BITS)
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct array length")
}

#[allow(non_snake_case)]
fn F<F: RichField + Extendable<D>, const D: usize>(
    builder: &mut CircuitBuilder<F, D>,
    iv: &[WordTargets; IV.len()],
    mut h: [WordTargets; INTERNAL_STATE_WORDS],
    m: &[WordTargets; BLOCK_WORDS],
    t: Target,
    f: bool,
) -> [[BoolTarget; WORD_BITS]; INTERNAL_STATE_WORDS] {
    let mut v: [WordTargets; V_WORDS] = h
        .iter()
        .chain(iv.iter())
        .copied()
        .collect::<Vec<_>>()
        .try_into()
        .expect("Correct word count");

    // Offset is bounded by Word, so high word == 0.
    let offset_low_word = builder.split_target_to_word_targets(t);
    let offset_high_word = word_array_to_word_targets([0], builder)[0];

    v[12] = builder.xor_words(v[12], offset_low_word);
    v[13] = builder.xor_words(v[13], offset_high_word);

    if f {
        v[14] = builder.not_word(v[14]);
    }

    for s in SIGMA {
        v = G(builder, v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        v = G(builder, v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        v = G(builder, v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        v = G(builder, v, 3, 7, 11, 15, m[s[6]], m[s[7]]);

        v = G(builder, v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        v = G(builder, v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        v = G(builder, v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        v = G(builder, v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }

    static_assertions::const_assert_eq!(INTERNAL_STATE_WORDS * 2, V_WORDS);
    for i in 0..INTERNAL_STATE_WORDS {
        h[i] = builder.triple_xor_words(h[i], v[i], v[i + INTERNAL_STATE_WORDS]);
    }

    h
}

#[allow(non_snake_case)]
#[allow(clippy::too_many_arguments)]
fn G<F: RichField + Extendable<D>, const D: usize>(
    builder: &mut CircuitBuilder<F, D>,
    mut v: [WordTargets; V_WORDS],
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    x: WordTargets,
    y: WordTargets,
) -> [WordTargets; V_WORDS] {
    v[a] = builder.triple_add_words_wrapping(v[a], v[b], x);
    v[d] = rright_word(builder.xor_words(v[d], v[a]), R1);
    v[c] = builder.add_words_wrapping(v[c], v[d]);
    v[b] = rright_word(builder.xor_words(v[b], v[c]), R2);
    v[a] = builder.triple_add_words_wrapping(v[a], v[b], y);
    v[d] = rright_word(builder.xor_words(v[d], v[a]), R3);
    v[c] = builder.add_words_wrapping(v[c], v[d]);
    v[b] = rright_word(builder.xor_words(v[b], v[c]), R4);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    use blake2::{
        digest::{Update, VariableOutput},
        Blake2bVar,
    };
    use plonky2::{
        iop::witness::{PartialWitness, WitnessWrite},
        plonk::{circuit_data::CircuitConfig, config::PoseidonGoldilocksConfig},
    };
    use plonky2_field::{goldilocks_field::GoldilocksField, types::PrimeField64};

    pub type F = GoldilocksField;
    pub type C = PoseidonGoldilocksConfig;
    pub const D: usize = 2;

    #[test]
    fn test_correct_hashing() {
        let test_inputs = vec![
            [10; 128].to_vec(),
            [10; 4].to_vec(),
            vec![1, 70, 4, 100, 28, 130, 12, 120],
        ];

        for data in test_inputs {
            let digest_1 = compute_digest_using_library(&data);
            let digest_2 = compute_digest_using_circuit(&data);

            assert_eq!(digest_1, digest_2, "test input: {data:?}");
        }
    }

    fn compute_digest_using_library(data: &[u8]) -> [u8; 32] {
        let mut hasher = Blake2bVar::new(32).expect("Instantiate Blake2bVar");
        hasher.update(data);
        let mut hash = [0; 32];
        hasher
            .finalize_variable(&mut hash)
            .expect("Hash of correct size");
        hash
    }

    fn compute_digest_using_circuit(data: &[u8]) -> [u8; 32] {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());
        let targets = blake2_circuit(&mut builder, data.len());

        let mut pw = PartialWitness::new();
        let data_bits = data
            .iter()
            .flat_map(|byte| (0..8).rev().map(move |bit_idx| (byte >> bit_idx) % 2 == 1))
            .collect::<Vec<_>>();

        for (i, bit) in data_bits.into_iter().enumerate() {
            pw.set_bool_target(targets.message[i], bit);
        }

        let circuit = builder.build::<C>();
        let proof = circuit.prove(pw).expect("Proven true");
        let digest = &proof.public_inputs[data.len() * 8..];

        let digest = digest
            .chunks(8)
            .map(|byte_out| {
                byte_out
                    .iter()
                    .enumerate()
                    .map(|(bit_no, bit)| bit.to_canonical_u64() * (1u64 << (7 - bit_no)))
                    .sum::<u64>() as u8
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("Correct hash size");

        circuit.verify(proof).expect("Verified true");

        digest
    }
}
