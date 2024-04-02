#![allow(incomplete_features)]
#![feature(return_position_impl_trait_in_trait)]

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

pub mod block_finality;
pub mod common;
pub mod final_proof;
pub mod latest_validator_set;
pub mod storage_inclusion;

pub use common::ProofWithCircuitData;

pub mod prelude {
    use plonky2::field::goldilocks_field::GoldilocksField;
    use plonky2::plonk::config::PoseidonGoldilocksConfig;

    pub type F = GoldilocksField;
    pub type C = PoseidonGoldilocksConfig;
    pub const D: usize = 2;

    pub(crate) use super::consts;

    pub use super::consts::GENESIS_AUTHORITY_SET_ID;
}

pub(crate) mod consts {
    pub const CIRCUIT_DIGEST_SIZE: usize = 4;

    // For now we send a single Keccak256 hash.
    pub const MESSAGE_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS: usize = 8;

    pub const BLAKE2_DIGEST_SIZE: usize = 32;
    pub const BLAKE2_DIGEST_SIZE_IN_BITS: usize = BLAKE2_DIGEST_SIZE * 8;
    /// If we pack `BoolTargets` into `Targets` by groups of 32 then
    /// 8 Goldilocks field elements are required.
    pub const BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS: usize =
        BLAKE2_DIGEST_SIZE_IN_BITS / 32;

    pub const ED25519_PUBLIC_KEY_SIZE: usize = 32;
    pub const ED25519_PUBLIC_KEY_SIZE_IN_BITS: usize = ED25519_PUBLIC_KEY_SIZE * 8;

    pub const ED25519_SIGNATURE_SIZE: usize = 64;
    pub const ED25519_SIGNATURE_SIZE_IN_BITS: usize = ED25519_SIGNATURE_SIZE * 8;

    pub const GRANDPA_VOTE_LENGTH: usize = 53;

    pub const MAX_VALIDATOR_COUNT: usize = 6;

    pub const GENESIS_AUTHORITY_SET_ID: u64 = 272;
    pub const GENESIS_VALIDATOR_SET_HASH: [u64; BLAKE2_DIGEST_SIZE_IN_GOLDILOCKS_FIELD_ELEMENTS] = [
        0x3E453535, 0x39A1043B, 0x8D1265E7, 0xB47E1B16, 0x3E263D42, 0x9EF6888C, 0x137A20F1,
        0x6696DBA1,
    ];
}
