#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

pub mod block_finality;
mod common;
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
}
