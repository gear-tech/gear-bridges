#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(type_alias_impl_trait)]
#![feature(concat_idents)]

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

pub mod block_finality;
pub mod common;
pub mod final_proof;
pub mod latest_validator_set;
pub mod merkle_proof;
pub mod message_sent;
pub mod next_validator_set;
pub mod validator_set_hash;

pub use common::ProofWithCircuitData;

mod prelude {
    use plonky2::field::goldilocks_field::GoldilocksField;
    use plonky2::plonk::config::PoseidonGoldilocksConfig;

    pub type F = GoldilocksField;
    pub type C = PoseidonGoldilocksConfig;
    pub const D: usize = 2;

    pub(crate) use super::consts;
}

pub(crate) mod consts {
    pub const CIRCUIT_DIGEST_SIZE: usize = 4;

    pub const SHA256_DIGEST_SIZE: usize = 32;
    pub const SHA256_DIGEST_SIZE_IN_BITS: usize = SHA256_DIGEST_SIZE * 8;
    /// If we pack `BoolTargets` into `Targets` by groups of 52 then
    /// 5 Goldilocks field elements are required.
    pub const SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS: usize = 5;

    // For now we send a single Keccak256 hash.
    pub const MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS: usize = 5;

    pub const BLAKE2_DIGEST_SIZE: usize = 32;
    pub const BLAKE2_DIGEST_SIZE_IN_BITS: usize = BLAKE2_DIGEST_SIZE * 8;

    pub const ED25519_PUBLIC_KEY_SIZE: usize = 32;
    pub const ED25519_PUBLIC_KEY_SIZE_IN_BITS: usize = ED25519_PUBLIC_KEY_SIZE * 8;

    pub const ED25519_SIGNATURE_SIZE: usize = 64;
    pub const ED25519_SIGNATURE_SIZE_IN_BITS: usize = ED25519_SIGNATURE_SIZE * 8;

    pub const GRANDPA_VOTE_LENGTH: usize = 53;

    pub const VALIDATOR_COUNT: usize = 6;
    pub const PROCESSED_VALIDATOR_COUNT: usize = 2;

    pub const GENESIS_AUTHORITY_SET_ID: u64 = 270;
    pub const GENESIS_VALIDATOR_SET_HASH: [u64; SHA256_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS] = [
        2787997088524558,
        914341688072726,
        3440393019007615,
        3418656939423883,
        276187037400784,
    ];
}
