use plonky2::{
    iop::witness::{PartialWitness, WitnessWrite},
    plonk::{circuit_builder::CircuitBuilder, circuit_data::CircuitConfig},
};

use plonky2_ed25519::gadgets::eddsa::make_verify_circuits as ed25519_circuit;

use super::GrandpaVoteTarget;
use crate::{
    common::{
        array_to_bits,
        targets::{impl_target_set, Ed25519PublicKeyTarget},
        CircuitImplBuilder, ProofWithCircuitData,
    },
    consts::GRANDPA_VOTE_LENGTH,
    prelude::*,
};

use lazy_static::lazy_static;
use plonky2::{iop::target::BoolTarget, plonk::circuit_data::CircuitData};

use crate::common::CircuitDataCache;

impl_target_set! {
    pub struct PublicInputsTarget {
        pub message: GrandpaVoteTarget,
        pub public_key: Ed25519PublicKeyTarget,
    }
}

pub struct SingleValidatorSign {
    pub public_key: [u8; consts::ED25519_PUBLIC_KEY_SIZE],
    pub signature: [u8; consts::ED25519_SIGNATURE_SIZE],
    pub message: [u8; GRANDPA_VOTE_LENGTH],
}

impl SingleValidatorSign {
    pub fn prove(self) -> ProofWithCircuitData<PublicInputsTarget> {
        log::debug!("        Proving single validator sign...");
        let res = CACHE.prove(self);
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
