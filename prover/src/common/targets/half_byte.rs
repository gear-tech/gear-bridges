//! ### `HalfByteTarget` implementation.

use super::*;

/// Represents 4-bit value.
#[derive(Clone, Copy, Debug)]
pub struct HalfByteTarget(Target);

impl TargetSet for HalfByteTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self(
            raw.next()
                .expect("Insufficient targets to construct HalfByteTarget"),
        )
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self.0)
    }
}

impl ParsableTargetSet for HalfByteTarget {
    type PublicInputsData = u8;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        let value = public_inputs
            .next()
            .expect("Insufficient public input count to counstruct HalfByteTarget public inputs")
            .to_canonical_u64();
        assert!(value < 16);
        value as u8
    }
}

impl HalfByteTarget {
    /// Create constant `HalfByteTarget`.
    pub fn constant(value: u8, builder: &mut CircuitBuilder<F, D>) -> HalfByteTarget {
        assert!(value < 16);
        Self(builder.constant(F::from_canonical_u8(value)))
    }

    /// Create `HalfByteTarget` from `Target` and perform range check on it.
    pub fn from_target_safe(target: Target, builder: &mut CircuitBuilder<F, D>) -> HalfByteTarget {
        builder.range_check(target, 4);
        Self(target)
    }

    /// Create `HalfByteTarget` from `Target` without performing range check on it.
    pub fn from_target_unsafe(target: Target) -> HalfByteTarget {
        Self(target)
    }

    /// Convert `HalfByteTarget` to `Target`,
    pub fn to_target(self) -> Target {
        self.0
    }
}
