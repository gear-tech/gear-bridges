use super::{
    common::{
        blake2::{CircuitTargets as Blake2CircuitTargets, GenericBlake2Target},
        common_data_for_recursion,
        targets::{impl_parsable_target_set, Blake2Target, ParsableTargetSet, TargetSet},
        ProofWithCircuitData,
    },
    prelude::*,
};
use plonky2::{
    field::types::Field,
    iop::{
        target::{BoolTarget, Target},
        witness::{PartialWitness, WitnessWrite},
    },
    plonk::{
        circuit_builder::CircuitBuilder,
        circuit_data::{
            CircuitConfig, CircuitData, CommonCircuitData, VerifierCircuitTarget,
            VerifierOnlyCircuitData,
        },
        proof::ProofWithPublicInputsTarget,
    },
    recursion::dummy_circuit,
};
use std::time::Instant;

impl_parsable_target_set! {
    pub struct HeaderChainTarget {
        pub hash_parent: Blake2Target,
        pub hash_header: Blake2Target,
        pub hash_header_start: Blake2Target,
        pub counter: Target,
    }
}

pub struct BuilderTargets {
    builder: CircuitBuilder<F, D>,
    target_verifier_circuit: VerifierCircuitTarget,
    target_proof_inner: ProofWithPublicInputsTarget<D>,
    target_condition: BoolTarget,
    target_inner_cyclic_proof: ProofWithPublicInputsTarget<D>,
}

impl Default for BuilderTargets {
    fn default() -> Self {
        Self::new()
    }
}

impl BuilderTargets {
    pub fn new() -> Self {
        log::trace!("BuilderTargets::new enter");

        let now = Instant::now();

        let config = CircuitConfig::standard_recursion_config();
        let mut builder = CircuitBuilder::<F, D>::new(config);
        let one = builder.one();

        let (circuit, ..) = Blake2CircuitTargets::new().into_inner();

        let target_proof_inner = builder.add_virtual_proof_with_pis(&circuit.common);
        let target_verifier = builder.constant_verifier_data(&circuit.verifier_only);

        builder.verify_proof::<C>(&target_proof_inner, &target_verifier, &circuit.common);

        let mut iter_public_inputs = target_proof_inner.public_inputs.iter().copied();
        let GenericBlake2Target {
            data,
            hash: target_inner_header_hash_in,
            ..
        } = GenericBlake2Target::parse_exact(&mut iter_public_inputs);

        let target_hash_start = Blake2Target::add_virtual_safe(&mut builder);
        let counter = builder.add_virtual_target();
        // we interpret input data to hash-proof as an encoded block header. The first field is
        // a hash to parent block (header).
        let mut data_target_bits = data
            .0
            .iter()
            .take(32)
            .flat_map(|t| t.as_bit_targets(&mut builder).0.into_iter().rev())
            .map(|target_bool| target_bool.target);
        let target_parent_hash = Blake2Target::parse_exact(&mut data_target_bits);
        HeaderChainTarget {
            hash_parent: target_parent_hash,
            hash_header: target_inner_header_hash_in.clone(),
            hash_header_start: target_hash_start.clone(),
            counter,
        }
        .register_as_public_inputs(&mut builder);

        let target_verifier_circuit = builder.add_verifier_data_public_inputs();

        let common_data = common_data_for_recursion(builder.num_public_inputs(), 1 << 13);

        let target_inner_cyclic_proof = builder.add_virtual_proof_with_pis(&common_data);
        // Unpack inner proof's public inputs.
        let mut iter_public_inputs = target_inner_cyclic_proof.public_inputs.iter().copied();
        let HeaderChainTarget {
            hash_header_start: target_inner_cyclic_hash_start,
            hash_parent: target_inner_cyclic_parent_hash,
            counter: inner_cyclic_counter,
            ..
        } = HeaderChainTarget::parse(&mut iter_public_inputs);

        target_inner_cyclic_parent_hash.connect(&target_inner_header_hash_in, &mut builder);

        let target_condition = builder.add_virtual_bool_target_safe();

        let target_hash_start_actual = Blake2Target::select(
            &mut builder,
            target_condition,
            target_inner_cyclic_hash_start,
            target_inner_header_hash_in,
        );
        target_hash_start.connect(&target_hash_start_actual, &mut builder);

        // Our chain length will be inner_counter + 1 if we have an inner proof, or 1 if not.
        let new_counter = builder.mul_add(target_condition.target, inner_cyclic_counter, one);
        builder.connect(counter, new_counter);

        builder
            .conditionally_verify_cyclic_proof_or_dummy::<C>(
                target_condition,
                &target_inner_cyclic_proof,
                &common_data,
            )
            .expect("Common circuit data is correct");

        log::trace!(
            "BuilderTargets::new exit. Time: {}ms",
            now.elapsed().as_millis()
        );

        BuilderTargets {
            builder,
            target_verifier_circuit,
            target_proof_inner,
            target_condition,
            target_inner_cyclic_proof,
        }
    }
}

pub struct CircuitTargets {
    circuit: CircuitData<F, C, D>,
    target_verifier_circuit: VerifierCircuitTarget,
    target_proof_inner: ProofWithPublicInputsTarget<D>,
    target_condition: BoolTarget,
    target_inner_cyclic_proof: ProofWithPublicInputsTarget<D>,
}

impl From<BuilderTargets> for CircuitTargets {
    fn from(value: BuilderTargets) -> Self {
        log::trace!("From<BuilderTargets> for CircuitTargets enter");
        let now = Instant::now();

        let circuit = value.builder.build::<C>();

        log::trace!(
            "From<BuilderTargets> for CircuitTargets exit. Time: {}ms",
            now.elapsed().as_millis()
        );

        Self {
            circuit,
            target_verifier_circuit: value.target_verifier_circuit,
            target_proof_inner: value.target_proof_inner,
            target_condition: value.target_condition,
            target_inner_cyclic_proof: value.target_inner_cyclic_proof,
        }
    }
}

impl CircuitTargets {
    pub fn prove(
        &self,
        proof_inner: &ProofWithCircuitData<GenericBlake2Target>,
        maybe_proof_recursive: Option<&ProofWithCircuitData<HeaderChainTarget>>,
    ) -> ProofWithCircuitData<HeaderChainTarget> {
        log::trace!("CircuitTargets::prove enter");

        let now = Instant::now();

        let mut witness = PartialWitness::new();
        witness
            .set_verifier_data_target(&self.target_verifier_circuit, &self.circuit.verifier_only);

        let proof_inner = proof_inner.proof();
        witness.set_proof_with_pis_target(&self.target_proof_inner, &proof_inner);

        if let Some(proof_recursive) = maybe_proof_recursive {
            witness.set_bool_target(self.target_condition, true);
            witness.set_proof_with_pis_target::<C, D>(
                &self.target_inner_cyclic_proof,
                &proof_recursive.proof(),
            );
        } else {
            witness.set_bool_target(self.target_condition, false);

            let mut iter_public_inputs = proof_inner.public_inputs.iter().copied();
            let output = GenericBlake2Target::parse_public_inputs(&mut iter_public_inputs);
            let initial_hash_pis = output
                .hash
                .into_iter()
                .map(F::from_bool)
                .enumerate()
                .collect();
            witness.set_proof_with_pis_target::<C, D>(
                &self.target_inner_cyclic_proof,
                &dummy_circuit::cyclic_base_proof(
                    &self.circuit.common,
                    &self.circuit.verifier_only,
                    initial_hash_pis,
                ),
            );
        }

        let proof = self.circuit.prove(witness).unwrap();

        log::trace!(
            "CircuitTargets::prove exit. Time: {}ms",
            now.elapsed().as_millis()
        );

        ProofWithCircuitData::from_proof_and_circuit_data(proof, self.circuit.verifier_data())
    }

    pub fn common(&self) -> &CommonCircuitData<F, D> {
        &self.circuit.common
    }

    pub fn verifier_only(&self) -> &VerifierOnlyCircuitData<C, D> {
        &self.circuit.verifier_only
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{array_to_bits, blake2::MAX_DATA_BYTES};
    use hex_literal::hex;
    use parity_scale_codec::Encode;
    use plonky2::recursion::cyclic_recursion::check_cyclic_proof_verifier_data;
    use sp_runtime::generic::{Digest, DigestItem};

    type GearHeader = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;

    #[test]
    fn proof_header_hash_list() {
        let _ = pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Debug)
            .format_target(false)
            .format_timestamp_secs()
            .parse_default_env()
            .try_init();

        log::info!("proof_header_hash_list started");

        let circuit_data_cyclic2 = CircuitTargets::from(BuilderTargets::new());

        let parent_hash =
            hex!("f068fc703857a04f29e7e67802eb9aef1fb49391aa2aa841e1052852f1cf1d74").into();
        let header_21_190_857 = GearHeader {
            parent_hash,
            number: 21_190_857,
            state_root: hex!("548c78243ba27058bd64314c07d9c1f797aed634ff2b827ffe9d18054b39fa68").into(),
            extrinsics_root: hex!("958dc405a31234face428760208a55d9fa27b71c427ec1d43833991b06174afd").into(),
            digest: Digest {
              logs: vec![
                DigestItem::PreRuntime(
                    *b"BABE",
                    hex::decode("02010000006fbcf42200000000").unwrap()
                ),
                DigestItem::Seal(
                    *b"BABE",
                    hex::decode("2e65b249a66e1a21686563dd5798443438695b2e023f68e68ed48888935d482e03b585fd816814f977817f21f2ad988aa4d0944e607d708e0089b558ab707680").unwrap()
                )
              ],
            },
        };

        let hash_start = hex!("be9446844275e99084cfc015b42fa061f6318890cd858becdc5d8a3255055735");
        assert_eq!(hash_start, header_21_190_857.hash().0);

        let circuit_blake2 = Blake2CircuitTargets::new();

        let proof_21_190_857 =
            circuit_blake2.prove::<MAX_DATA_BYTES>(header_21_190_857.encode().as_ref());

        log::debug!("Create proof of a header list");

        let proof_cyclic_1 = circuit_data_cyclic2.prove(&proof_21_190_857, None);

        check_cyclic_proof_verifier_data(
            &proof_cyclic_1.proof(),
            &circuit_data_cyclic2.circuit.verifier_only,
            &circuit_data_cyclic2.circuit.common,
        )
        .unwrap();

        circuit_data_cyclic2
            .circuit
            .verify(proof_cyclic_1.proof())
            .unwrap();

        let binding = proof_cyclic_1.proof();
        let mut iter_public_inputs = binding.public_inputs.iter().copied();
        let output = HeaderChainTarget::parse_public_inputs(&mut iter_public_inputs);

        assert_eq!(
            &output.hash_parent[..],
            &array_to_bits(parent_hash.as_ref())
        );
        assert_eq!(
            &output.hash_header_start[..],
            &array_to_bits(hash_start.as_ref())
        );
        assert_eq!(&output.hash_header[..], &array_to_bits(hash_start.as_ref()));
        assert_eq!(output.counter, 1);

        log::debug!("Add previous header to the list");

        let hash = parent_hash;
        let parent_hash =
            hex!("3736bc637a62e0957c686d041715dab0b155a9a5928e145f355e9ef79a15db69").into();
        let header_21_190_856 = GearHeader {
            parent_hash,
            number: 21_190_856,
            state_root: hex!("07cfc1ad2dc961d54f55b88717d3de5fc6e1388677d75f63c6e266849fbde258").into(),
            extrinsics_root: hex!("9684aa2a18b06858c55569acebda20a890ce4b27ebc98e499ac2fad1636fccd1").into(),
            digest: Digest {
              logs: vec![
                DigestItem::PreRuntime(
                    *b"BABE",
                    hex::decode("02010000006ebcf42200000000").unwrap()
                ),
                DigestItem::Seal(
                    *b"BABE",
                    hex::decode("c4cc63d677599d5d12f4dfec80e8ed0e10f1a8b51c797a63d88a87d7c044f96d9f51e39ccde86c8c4abdc9dc7676b326c5a465f8a9a875b9d9779919c361ce80").unwrap()
                )
              ],
            },
        };

        assert_eq!(hash, header_21_190_856.hash());

        let proof_21_190_856 =
            circuit_blake2.prove::<MAX_DATA_BYTES>(header_21_190_856.encode().as_ref());

        let proof_cyclic_2 = circuit_data_cyclic2.prove(&proof_21_190_856, Some(&proof_cyclic_1));

        check_cyclic_proof_verifier_data(
            &proof_cyclic_2.proof(),
            &circuit_data_cyclic2.circuit.verifier_only,
            &circuit_data_cyclic2.circuit.common,
        )
        .unwrap();

        circuit_data_cyclic2
            .circuit
            .verify(proof_cyclic_2.proof())
            .unwrap();

        let binding = proof_cyclic_2.proof();
        let mut iter_public_inputs = binding.public_inputs.iter().copied();
        let output = HeaderChainTarget::parse_public_inputs(&mut iter_public_inputs);

        assert_eq!(
            &output.hash_parent[..],
            &array_to_bits(parent_hash.as_ref())
        );
        assert_eq!(
            &output.hash_header_start[..],
            &array_to_bits(hash_start.as_ref())
        );
        assert_eq!(&output.hash_header[..], &array_to_bits(hash.as_ref()));
        assert_eq!(output.counter, 2);
    }
}
