#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use log::{Level, LevelFilter};
use plonky2::field::goldilocks_field::GoldilocksField;
use plonky2::hash::hash_types::RichField;
use plonky2::iop::witness::{PartialWitness, WitnessWrite};
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::plonk::circuit_data::{CircuitConfig, CircuitData, VerifierCircuitTarget};
use plonky2::plonk::config::{GenericConfig, PoseidonGoldilocksConfig};
use plonky2::plonk::proof::ProofWithPublicInputs;
use plonky2::util::timing::TimingTree;
use plonky2_ed25519::gadgets::eddsa::{ed25519_circuit, fill_ecdsa_targets, EDDSATargets};
use plonky2_field::extension::Extendable;
use std::time::Instant;

type F = GoldilocksField;
type C = PoseidonGoldilocksConfig;

const D: usize = 2;

pub struct PreCommit {
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}

pub struct BlockJustification {
    pub pre_commits: Vec<PreCommit>,
    pub msg: Vec<u8>,
}

impl BlockJustification {
    pub fn prove(&self) {
        let mut logger = env_logger::Builder::from_default_env();
        logger.format_timestamp(None);
        logger.filter_level(LevelFilter::Info);
        logger.try_init().unwrap();

        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());
        let targets = ed25519_circuit(&mut builder, self.msg.len() * 8);
        let data = builder.build::<C>();

        let timing = TimingTree::new("Map-reduce", Level::Info);
        let now = Instant::now();
        let (proof, data) = self
            .pre_commits
            .iter()
            .map(|pc| {
                prove_ed25519_with_targets::<F, C, D>(
                    self.msg.as_slice(),
                    &pc.signature,
                    &pc.public_key,
                    &targets,
                    &data,
                )
            })
            .reduce(|acc, x| compose(&acc.0, &acc.1, &x.0, &x.1))
            .unwrap();
        timing.print();
        println!("Map-reduce {}", now.elapsed().as_secs());
        data.verify(proof).unwrap();
    }
}

fn prove_ed25519_with_targets<
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>,
    const D: usize,
>(
    msg: &[u8],
    sigv: &[u8],
    pkv: &[u8],
    targets: &EDDSATargets,
    data: &CircuitData<F, C, D>,
) -> (ProofWithPublicInputs<F, C, D>, CircuitData<F, C, D>) {
    let mut pw = PartialWitness::new();
    fill_ecdsa_targets::<F, D>(&mut pw, msg, sigv, pkv, targets);

    let timing = TimingTree::new("prove", Level::Info);
    let proof = data.prove(pw).unwrap();
    timing.print();

    let timing = TimingTree::new("verify", Level::Info);
    data.verify(proof.clone()).expect("verify error");
    timing.print();

    (proof, data.clone())
}

fn compose(
    proof_with_pis_1: &ProofWithPublicInputs<F, C, D>,
    circuit_data_1: &CircuitData<F, C, D>,
    proof_with_pis_2: &ProofWithPublicInputs<F, C, D>,
    circuit_data_2: &CircuitData<F, C, D>,
) -> (ProofWithPublicInputs<F, C, D>, CircuitData<F, C, D>) {
    println!("composing...");
    let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::standard_recursion_config());
    let proof_with_pis_target_1 = builder.add_virtual_proof_with_pis(&circuit_data_1.common);
    let proof_with_pis_target_2 = builder.add_virtual_proof_with_pis(&circuit_data_2.common);

    let verifier_circuit_target_1 = VerifierCircuitTarget {
        constants_sigmas_cap: builder
            .add_virtual_cap(circuit_data_1.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };
    let verifier_circuit_target_2 = VerifierCircuitTarget {
        constants_sigmas_cap: builder
            .add_virtual_cap(circuit_data_2.common.config.fri_config.cap_height),
        circuit_digest: builder.add_virtual_hash(),
    };

    let mut pw = PartialWitness::new();
    pw.set_proof_with_pis_target(&proof_with_pis_target_1, proof_with_pis_1);
    pw.set_proof_with_pis_target(&proof_with_pis_target_2, proof_with_pis_2);
    pw.set_cap_target(
        &verifier_circuit_target_1.constants_sigmas_cap,
        &circuit_data_1.verifier_only.constants_sigmas_cap,
    );
    pw.set_cap_target(
        &verifier_circuit_target_2.constants_sigmas_cap,
        &circuit_data_2.verifier_only.constants_sigmas_cap,
    );
    pw.set_hash_target(
        verifier_circuit_target_1.circuit_digest,
        circuit_data_1.verifier_only.circuit_digest,
    );
    pw.set_hash_target(
        verifier_circuit_target_2.circuit_digest,
        circuit_data_2.verifier_only.circuit_digest,
    );

    builder.verify_proof::<C>(
        &proof_with_pis_target_1,
        &verifier_circuit_target_1,
        &circuit_data_1.common,
    );
    builder.verify_proof::<C>(
        &proof_with_pis_target_2,
        &verifier_circuit_target_2,
        &circuit_data_2.common,
    );

    let circuit_data = builder.build::<C>();
    let proof = circuit_data.prove(pw).unwrap();
    (proof, circuit_data)
}
