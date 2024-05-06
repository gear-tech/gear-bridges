//! ### `VerifierDataTarget` implementation.

use super::*;

#[derive(Clone, Debug)]
pub struct VerifierDataTarget<const NUM_CAP_ELEMENTS: usize> {
    pub circuit_digest: HashOutTarget,
    pub merkle_caps: [HashOutTarget; NUM_CAP_ELEMENTS],
}

impl<const NUM_CAP_ELEMENTS: usize> TargetSet for VerifierDataTarget<NUM_CAP_ELEMENTS> {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self {
            circuit_digest: HashOutTarget::parse(raw),
            merkle_caps: [(); NUM_CAP_ELEMENTS].map(|_| HashOutTarget::parse(raw)),
        }
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        self.circuit_digest.into_targets_iter().chain(
            self.merkle_caps
                .into_iter()
                .flat_map(|hash| hash.into_targets_iter()),
        )
    }
}
