use alloc::{vec, vec::Vec};

use plonky2::{
    field::extension::Extendable, hash::hash_types::RichField, iop::target::Target,
    plonk::circuit_builder::CircuitBuilder,
};

use crate::{gadgets::arithmetic_u32::U32Target, gates::range_check_u32::U32RangeCheckGate};

pub fn range_check_u32_circuit<F: RichField + Extendable<D>, const D: usize>(
    builder: &mut CircuitBuilder<F, D>,
    vals: Vec<U32Target>,
) {
    let num_input_limbs = vals.len();
    let gate = U32RangeCheckGate::<F, D>::new(num_input_limbs);
    let row = builder.add_gate(gate, vec![]);

    for i in 0..num_input_limbs {
        builder.connect(Target::wire(row, gate.wire_ith_input_limb(i)), vals[i].0);
    }
}
