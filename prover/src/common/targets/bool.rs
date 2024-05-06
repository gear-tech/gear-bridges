//! ### `BoolTarget` implementation.

use super::*;

impl TargetSet for BoolTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self::new_unsafe(
            raw.next()
                .expect("Insufficient targets to construct BoolTarget"),
        )
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self.target)
    }
}

impl ParsableTargetSet for BoolTarget {
    type PublicInputsData = bool;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        let data = public_inputs
            .next()
            .expect("Insufficient public input count to construct BoolTarget public inputs")
            .to_canonical_u64();
        assert!(data == 0 || data == 1);
        data == 1
    }
}
