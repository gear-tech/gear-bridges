use super::*;

#[derive(Clone, Debug, Copy)]
pub struct ArrayTarget<T: TargetSet, const N: usize>(pub [T; N]);

pub type BitArrayTarget<const N: usize> = ArrayTarget<BoolTarget, N>;

impl<T: TargetSet, const N: usize> TargetSet for ArrayTarget<T, N> {
    fn parse(raw: &mut impl Iterator<Item = Target>) -> Self {
        Self(
            (0..N)
                .map(|_| TargetSet::parse(raw))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )
    }

    fn into_targets_iter(self) -> impl Iterator<Item = Target> {
        self.0
            .into_iter()
            .flat_map(|element| element.into_targets_iter())
    }
}

impl<T: TargetSet, const N: usize> ArrayTarget<T, N> {
    pub fn constant_read(&self, at: usize) -> T {
        self.0[at].clone()
    }

    pub fn constant_read_array<const R: usize>(&self, at: usize) -> ArrayTarget<T, R> {
        ArrayTarget(
            (0..R)
                .map(|offset| self.constant_read(at + offset))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )
    }

    pub fn random_read(&self, at: Target, builder: &mut CircuitBuilder<F, D>) -> T {
        let max_idx = builder.constant(F::from_canonical_usize(N - 1));
        let max_idx_sub_at = builder.sub(max_idx, at);
        builder.range_check(max_idx_sub_at, 32);

        let self_targets = self
            .0
            .clone()
            .into_iter()
            .map(|ts| ts.into_targets_iter().collect::<Vec<_>>())
            .collect::<Vec<_>>();

        let inner_target_len = self_targets[0].len();
        assert!(self_targets.iter().all(|t| t.len() == inner_target_len));

        let mut result_targets = Vec::with_capacity(inner_target_len);
        let zero = builder.zero();
        let selector_targets_padded_len = self_targets.len().next_power_of_two();
        for i in 0..inner_target_len {
            let selector_targets = self_targets
                .iter()
                .map(|t| t[i])
                .chain(iter::repeat(zero))
                .take(selector_targets_padded_len)
                .collect();

            let result = builder.random_access(at, selector_targets);
            result_targets.push(result);
        }

        T::parse_exact(&mut result_targets.into_iter())
    }

    pub fn random_read_array<const R: usize>(
        &self,
        at: Target,
        builder: &mut CircuitBuilder<F, D>,
    ) -> ArrayTarget<T, R> {
        ArrayTarget(
            (0..R)
                .map(|offset| {
                    let offset = builder.constant(F::from_canonical_usize(offset));
                    let read_at = builder.add(at, offset);
                    self.random_read(read_at, builder)
                })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )
    }
}

impl<T, const N: usize> ParsableTargetSet for ArrayTarget<T, N>
where
    T: ParsableTargetSet,
    T::PublicInputsData: Debug,
{
    type PublicInputsData = [T::PublicInputsData; N];

    fn parse_public_inputs(public_inputs: &mut impl Iterator<Item = F>) -> Self::PublicInputsData {
        (0..N)
            .map(|_| T::parse_public_inputs(public_inputs))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}
