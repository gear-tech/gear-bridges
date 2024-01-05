#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(type_alias_impl_trait)]
#![feature(concat_idents)]

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

pub mod block_finality;
pub mod common;
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
    pub const SHA256_DIGEST_SIZE: usize = 32;
    pub const SHA256_DIGEST_SIZE_IN_BITS: usize = SHA256_DIGEST_SIZE * 8;

    pub const BLAKE2_DIGEST_SIZE: usize = 32;
    pub const BLAKE2_DIGEST_SIZE_IN_BITS: usize = BLAKE2_DIGEST_SIZE * 8;

    pub const ED25519_PUBLIC_KEY_SIZE: usize = 32;
    pub const ED25519_PUBLIC_KEY_SIZE_IN_BITS: usize = ED25519_PUBLIC_KEY_SIZE * 8;

    pub const ED25519_SIGNATURE_SIZE: usize = 64;
    pub const ED25519_SIGNATURE_SIZE_IN_BITS: usize = ED25519_SIGNATURE_SIZE * 8;

    pub const GRANDPA_VOTE_LENGTH: usize = 53;
    pub const GRANDPA_VOTE_LENGTH_IN_BITS: usize = GRANDPA_VOTE_LENGTH * 8;

    pub const VALIDATOR_COUNT: usize = 55;
    pub const PROCESSED_VALIDATOR_COUNT: usize = 5;
}
