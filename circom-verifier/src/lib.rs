#![feature(generic_const_exprs)]

use plonky2::{
    field::goldilocks_field::GoldilocksField,
    plonk::{
        circuit_data::{CircuitConfig, CommonCircuitData, VerifierOnlyCircuitData},
        config::PoseidonGoldilocksConfig,
        proof::ProofWithPublicInputs,
    },
};
use std::{
    fs::{create_dir_all, write},
    path::{Path, PathBuf},
};

mod config;
mod verifier;

use config::PoseidonBN128GoldilocksConfig as CBn128;
use verifier::{
    generate_circom_verifier_inner, generate_proof_base64, generate_verifier_config,
    recursive_proof,
};

type F = GoldilocksField;
type C = PoseidonGoldilocksConfig;
const D: usize = 2;

pub struct CircomVerifierFilePaths {
    pub constants: PathBuf,
    pub gates: PathBuf,
    pub proof: PathBuf,
    pub config: PathBuf,
}

pub fn write_circom_verifier_files(
    paths: CircomVerifierFilePaths,
    common_circuit_data: CommonCircuitData<F, D>,
    verifier_only_circuit_data: VerifierOnlyCircuitData<C, D>,
    proof_with_public_inputs: ProofWithPublicInputs<F, C, D>,
) {
    let standard_config = CircuitConfig::standard_recursion_config();
    let (proof, vd, cd) = recursive_proof::<F, CBn128, C, 2>(
        proof_with_public_inputs,
        verifier_only_circuit_data,
        common_circuit_data,
        &standard_config,
        None,
        true,
        true,
    )
    .unwrap();

    let conf = generate_verifier_config(&proof).unwrap();
    let (constants_circom, gates_circom) = generate_circom_verifier_inner(&conf, &cd, &vd).unwrap();

    let proof = generate_proof_base64(&proof, &conf).unwrap();
    let config = serde_json::to_string(&conf).unwrap();

    write_file(paths.constants, constants_circom);
    write_file(paths.gates, gates_circom);
    write_file(paths.proof, proof);
    write_file(paths.config, config);
}

fn write_file(path: PathBuf, contents: String) {
    create_dir_all(path.parent().unwrap()).unwrap();
    write(path, contents).unwrap();
}
