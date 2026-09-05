//! Regenerates the VariativeBlake2 cache files that production prover code
//! (common/blake2/mod.rs:48-74, variative.rs:57-98) expects in VBLAKE2_CACHE_PATH.
//!
//! SECURITY(CR-1 release): writes the FULL file family the relayer prove() path loads —
//! prover_circuit_data-{i}, prover_circuit_data-targets-{i}, verifier_only_circuit_data-{i},
//! common_circuit_data. A verifier-only cache bricks the relayer at startup.
//!
//! Disk requirement: ~350 GB (50 × ~9 GB prover files). Resume-safe: an index is skipped
//! only when all three of its files already exist.
//!
//! Run: VBLAKE2_CACHE_PATH=/path/to/cache cargo run --release --example setup_cache -p prover

use plonky2::gates::noop::NoopGate;
use plonky2::util::serialization::Write;

use std::fs;
use prover::VariativeBlake2;
use plonky2::field::goldilocks_field::GoldilocksField;
use plonky2::plonk::config::PoseidonGoldilocksConfig;
type F = GoldilocksField;
type C = PoseidonGoldilocksConfig;
const D: usize = 2;
use prover::serialization::{GateSerializer, GeneratorSerializer};

const NUM_GATES: usize = 655_360; // blake2/mod.rs:42
const MAX_BLOCK_COUNT: usize = 50; // blake2/mod.rs:41

fn main() {
    let path = std::env::var("VBLAKE2_CACHE_PATH").expect("VBLAKE2_CACHE_PATH set");
    fs::create_dir_all(&path).expect("cache dir");
    let gate_ser = GateSerializer;
    let gen_ser = GeneratorSerializer::<C, D>::default();

    for i in 1..=MAX_BLOCK_COUNT {
        let now = std::time::Instant::now();
        // block_count = (i*128).div_ceil(128) = i — identical builder ops to
        // VariativeBlake2::prove_non_cached / create_builder_targets.
        let (mut builder, targets) = VariativeBlake2::create_builder_targets(i * 128);
        while builder.num_gates() < NUM_GATES {
            builder.add_gate(NoopGate, vec![]);
        }
        // SECURITY(CR-1 release): production prove() (variative.rs:57-98) loads
        // prover_circuit_data-{i} AND prover_circuit_data-targets-{i}; verifier_only+common
        // alone only serve the quorum route. A cache without them bricks the relayer.
        // Skip only when the FULL family is present.
        let vfile = format!("{path}/verifier_only_circuit_data-{i}");
        let pfile = format!("{path}/prover_circuit_data-{i}");
        let tfile = format!("{path}/prover_circuit_data-targets-{i}");
        if i != 1
            && [&vfile, &pfile, &tfile]
                .iter()
                .all(|f| std::path::Path::new(f).exists())
        {
            eprintln!("skip block_count={i} (cached)");
            continue;
        }

        let cd = builder.build::<C>();

        fs::write(&vfile, cd.verifier_only.to_bytes().expect("ser verifier_only")).unwrap();

        if i == 1 {
            fs::write(
                format!("{path}/common_circuit_data"),
                cd.common.to_bytes(&gate_ser).expect("ser common"),
            )
            .unwrap();
        }

        // SECURITY(CR-1 release): full prover family, byte-compatible with the ReadAdapter
        // (variative.rs:73-86) and Buffer (variative.rs:91-95) read paths. Reuses the
        // `targets` binding above (same circuit the verifier data was built from).
        let mut prover_bytes = Vec::new();
        prover_bytes
            .write_prover_circuit_data::<F, C, D>(
                &plonky2::plonk::circuit_data::ProverCircuitData {
                    prover_only: cd.prover_only,
                    common: cd.common,
                },
                &gate_ser,
                &gen_ser,
            )
            .expect("ser prover_circuit_data");
        fs::write(&pfile, prover_bytes).unwrap();

        let mut targets_bytes = Vec::new();
        targets_bytes.write_target_vec(&targets).expect("ser targets");
        fs::write(&tfile, targets_bytes).unwrap();
        eprintln!("built block_count={i} in {:?}", now.elapsed());
    }
    eprintln!("cache ready at {path}");
}
