//! ### Circuit that's used to prove that a single validator have signed GRANDPA message.
//!
//! Proving this circuit is the most time-consuming proof among all the others, so the circuit
//! is built only on first call to `prove` and taken from cache on the next calls.

use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use plonky2_ed25519::gadgets::eddsa::make_verify_circuits as ed25519_circuit;

use super::GrandpaMessageTarget;
use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Ed25519PublicKeyTarget},
        CircuitImplBuilder, ProofWithCircuitData,
    },
    consts::GRANDPA_VOTE_LENGTH,
    prelude::*,
};
use std::time::Instant;

use lazy_static::lazy_static;
use plonky2::{iop::target::BoolTarget, plonk::circuit_data::CircuitData};

use crate::common::CircuitDataCache;

impl_target_set! {
    /// Public inputs for `SingleValidatorSign`.
    pub struct PublicInputsTarget {
        /// GRANDPA message.
        pub message: GrandpaMessageTarget,
        /// Validator public key.
        pub public_key: Ed25519PublicKeyTarget,
    }
}

pub struct SingleValidatorSign {
    /// Public key of validator that've signed the message.
    pub public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    /// Signature of validator that've signed the message.
    pub signature: [u8; consts::ED25519_SIGNATURE_SIZE],
    /// GRANDPA message.
    pub message: [u8; GRANDPA_VOTE_LENGTH],
}

impl SingleValidatorSign {
    pub fn prove(self) -> ProofWithCircuitData<PublicInputsTarget> {
        log::debug!("        Proving single validator sign...");

        let now = Instant::now();

        let res = CACHE.prove(self);

        log::info!("SingleValidatorSign time: {}ms", now.elapsed().as_millis());
        log::debug!("        Proven single validator sign...");
        res
    }
}

lazy_static! {
    static ref CACHE: CircuitDataCache<SingleValidatorSign> = CircuitDataCache::new();
}

#[derive(Clone)]
pub struct WitnessTargets {
    public_key: [BoolTarget; consts::ED25519_PUBLIC_KEY_SIZE_IN_BITS],
    signature: [BoolTarget; consts::ED25519_SIGNATURE_SIZE_IN_BITS],
    message: [BoolTarget; GRANDPA_VOTE_LENGTH * 8],
}

impl CircuitImplBuilder for SingleValidatorSign {
    type WitnessTargets = WitnessTargets;
    type PublicInputsTarget = PublicInputsTarget;

    fn build() -> (CircuitData<F, C, D>, Self::WitnessTargets) {
        let mut builder = CircuitBuilder::<F, D>::new(CircuitConfig::wide_ecc_config());

        // This fn registers public inputs as:
        //  - message contents as `BoolTarget`s
        //  - public key as `BoolTarget`s
        let targets = ed25519_circuit(&mut builder, GRANDPA_VOTE_LENGTH);
        let witness_targets = WitnessTargets {
            public_key: targets
                .pk
                .try_into()
                .expect("Incorrect amount of targets for targets.pk"),
            signature: targets
                .sig
                .try_into()
                .expect("Incorrect amount of targets for targets.sig"),
            message: targets
                .msg
                .try_into()
                .expect("Incorrect amount of targets for targets.msg"),
        };

        (builder.build(), witness_targets)
    }

    fn set_witness(&self, targets: Self::WitnessTargets, witness: &mut PartialWitness<F>) {
        let pk_bits = array_to_bits(&self.public_key).into_iter();
        for (target, value) in targets.public_key.iter().zip(pk_bits) {
            witness.set_bool_target(*target, value);
        }

        let signature_bits = array_to_bits(&self.signature).into_iter();
        for (target, value) in targets.signature.iter().zip(signature_bits) {
            witness.set_bool_target(*target, value);
        }

        let msg_bits = array_to_bits(&self.message).into_iter();
        for (target, value) in targets.message.iter().zip(msg_bits) {
            witness.set_bool_target(*target, value);
        }
    }
}
