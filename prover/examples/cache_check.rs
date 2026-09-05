//! Roundtrip validation for the VBLAKE2 cache: loads prover_circuit_data-{index}
//! through the UNMODIFIED production read path (VariativeBlake2::prove ->
//! ReadAdapter/Buffer) and proves. Run after every cache generation and before
//! pointing a relayer at a new cache.
//!
//! Run: VBLAKE2_CACHE_PATH=/path/to/cache cargo run --release --example cache_check -p prover
//! Exit 0 = cache is loadable and usable by the production prover.

use prover::VariativeBlake2;

fn main() {
    // index==2 is short-circuited to an in-binary constant by prove(), so avoid 2*128.
    // These two lengths hit cache indices 1 and 12.
    for n_bytes in [100usize, 1500] {
        let now = std::time::Instant::now();
        let p = VariativeBlake2::prove(&vec![9u8; n_bytes]);
        println!(
            "prove(len={n_bytes}) OK — loaded prover_circuit_data + targets through the \
             production read path in {:?}",
            now.elapsed()
        );
        let _ = p;
    }
    println!("CACHE LOAD/PROVE ROUNDTRIP OK");
}
