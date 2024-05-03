//! ### `ByteTarget` implementation.

use super::*;

#[derive(Clone, Copy, Debug)]
pub struct ByteTarget(Target);

impl TargetSet for ByteTarget {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self(
            raw.next()
                .expect("Insufficient targets to construct ByteTarget"),
        )
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        std::iter::once(self.0)
    }
}

impl ParsableTargetSet for ByteTarget {
    type PublicInputsData = u8;

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        public_inputs
            .next()
            .map(|pi| pi.to_canonical_u64() as u8)
            .expect("Insufficient public inputs to construct ByteTarget public inputs")
    }
}

impl ByteTarget {
    /// Create `HalfByteTarget` from `Target` and perform range check on it.
    pub fn from_target_safe(target: Target, builder: &mut CircuitBuilder<F, D>) -> ByteTarget {
        builder.range_check(target, 8);
        Self(target)
    }

    /// Create `HalfByteTarget` from `Target` without performing range check on it.
    pub fn from_target_unsafe(target: Target) -> ByteTarget {
        Self(target)
    }

    /// Convert `HalfByteTarget` to `Target`.
    pub fn to_target(&self) -> Target {
        self.0
    }

    /// Split `ByteTarget` into `(least_significant, most_significant)` `HalfByteTarget`s.
    pub fn to_half_byte_targets(
        &self,
        builder: &mut CircuitBuilder<F, D>,
    ) -> (HalfByteTarget, HalfByteTarget) {
        let bits = self.to_bit_targets(builder);

        let l_bits = &bits.0[..4];
        let h_bits = &bits.0[4..];

        (
            HalfByteTarget::from_target_unsafe(builder.le_sum(l_bits.iter())),
            HalfByteTarget::from_target_unsafe(builder.le_sum(h_bits.iter())),
        )
    }

    /// Convert `HalfByteTarget` to bits arranged from less to most significant bit.
    pub fn to_bit_targets(&self, builder: &mut CircuitBuilder<F, D>) -> ArrayTarget<BoolTarget, 8> {
        ArrayTarget(
            builder
                .low_bits(self.0, 8, 8)
                .try_into()
                .expect("8 bits to be returned from CircuitBuilder::low_bits"),
        )
    }

    /// Create constant `HalfByteTarget`.
    #[cfg(test)]
    pub fn constant(value: u8, builder: &mut CircuitBuilder<F, D>) -> ByteTarget {
        Self(builder.constant(F::from_canonical_u8(value)))
    }
}
