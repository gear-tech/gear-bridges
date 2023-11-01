use log::{Level, LevelFilter};
use plonky2::hash::hash_types::RichField;
use plonky2::iop::witness::PartialWitness;
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::plonk::circuit_data::{CircuitConfig, CircuitData};
use plonky2::plonk::config::GenericConfig;
use plonky2::plonk::proof::ProofWithPublicInputs;
use plonky2::util::timing::TimingTree;
use plonky2_ed25519::gadgets::eddsa::{ed25519_circuit, fill_ecdsa_targets, EDDSATargets};
use plonky2_field::extension::Extendable;
use std::time::Instant;

use crate::prelude::*;

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
            .reduce(|acc, x| compose_circuits(&acc.0, &acc.1, &x.0, &x.1))
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
