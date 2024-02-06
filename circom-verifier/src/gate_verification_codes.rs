use std::any::Any;

use plonky2::{
    gates::{
        arithmetic_base::ArithmeticGate, arithmetic_extension::ArithmeticExtensionGate,
        base_sum::BaseSumGate, constant::ConstantGate, exponentiation::ExponentiationGate,
        gate::AnyGate, multiplication_extension::MulExtensionGate, poseidon::PoseidonGate,
        poseidon_mds::PoseidonMdsGate, public_input::PublicInputGate,
        random_access::RandomAccessGate, reducing::ReducingGate,
        reducing_extension::ReducingExtensionGate,
    },
    hash::{
        hash_types::RichField,
        poseidon::{self, Poseidon},
    },
};
use plonky2_field::extension::Extendable;

trait GateVerificationCode {
    fn export_verification_code(&self) -> String;
}

pub fn export_gate_verification_code<F, const D: usize>(gate: &dyn AnyGate<F, D>) -> String
where
    F: RichField + Extendable<D>,
{
    let gate = gate.as_any();

    macro_rules! process_gate {
        ($gate_ty:ty) => {
            match gate.downcast_ref::<$gate_ty>() {
                Some(gate) => return gate.export_verification_code(),
                None => {}
            }
        };
    }

    process_gate!(ArithmeticGate);
    process_gate!(ArithmeticExtensionGate<D>);

    process_gate!(BaseSumGate<1>);
    process_gate!(BaseSumGate<2>);
    process_gate!(BaseSumGate<3>);
    process_gate!(BaseSumGate<4>);
    process_gate!(BaseSumGate<5>);
    process_gate!(BaseSumGate<6>);
    process_gate!(BaseSumGate<7>);
    process_gate!(BaseSumGate<8>);

    process_gate!(ConstantGate);
    process_gate!(ExponentiationGate<F, D>);
    process_gate!(MulExtensionGate<D>);
    process_gate!(PoseidonGate<F, D>);
    process_gate!(PoseidonMdsGate<F, D>);
    process_gate!(PublicInputGate);
    process_gate!(RandomAccessGate<F, D>);
    process_gate!(ReducingGate<D>);
    process_gate!(ReducingExtensionGate<D>);

    unimpemented!()
}

impl GateVerificationCode for ArithmeticGate {
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template Arithmetic$NUM_OPS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                for (var i = 0; i < $NUM_OPS; i++) {{
                    out[i] <== ConstraintPush()(constraints[i], filter, GlExtSub()(wires[4 * i + 3], GlExtAdd()(GlExtMul()(GlExtMul()(wires[4 * i], wires[4 * i + 1]), constants[$NUM_SELECTORS + 0]), GlExtMul()(wires[4 * i + 2], constants[$NUM_SELECTORS + 1]))));
                }}
                for (var i = $NUM_OPS; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
        ).to_string();
        template_str = template_str.replace("$NUM_OPS", &*self.num_ops.to_string());
        template_str
    }
}

impl<const D: usize> GateVerificationCode for ArithmeticExtensionGate<D> {
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template ArithmeticExtension$NUM_OPS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                signal m[$NUM_OPS][2][2];
                for (var i = 0; i < $NUM_OPS; i++) {{
                    m[i] <== WiresAlgebraMul(4 * $D * i, 4 * $D * i + $D)(wires);
                    for (var j = 0; j < $D; j++) {{
                    out[i * $D + j] <== ConstraintPush()(constraints[i * $D + j], filter, GlExtSub()(wires[4 * $D * i + 3 * $D + j], GlExtAdd()(GlExtMul()(m[i][j], constants[$NUM_SELECTORS]), GlExtMul()(wires[4 * $D * i + 2 * $D + j], constants[$NUM_SELECTORS + 1]))));
                    }}
                }}
                for (var i = $NUM_OPS * $D; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
        ).to_string();
        template_str = template_str.replace("$NUM_OPS", &*self.num_ops.to_string());
        template_str = template_str.replace("$D", &*D.to_string());
        template_str
    }
}

impl<const B: usize> GateVerificationCode for BaseSumGate<B> {
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template BaseSum$NUM_LIMBS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                component reduce = Reduce($NUM_LIMBS);
                reduce.alpha <== GlExt($B, 0)();
                reduce.old_eval <== GlExt(0, 0)();
                for (var i = 1; i < $NUM_LIMBS + 1; i++) {{
                    reduce.in[i - 1] <== wires[i];
                }}
                out[0] <== ConstraintPush()(constraints[0], filter, GlExtSub()(reduce.out, wires[0]));
                component product[$NUM_LIMBS][$B - 1];
                for (var i = 0; i < $NUM_LIMBS; i++) {{
                    for (var j = 0; j < $B - 1; j++) {{
                    product[i][j] = GlExtMul();
                    if (j == 0) product[i][j].a <== wires[i + 1];
                    else product[i][j].a <== product[i][j - 1].out;
                    product[i][j].b <== GlExtSub()(wires[i + 1], GlExt(j + 1, 0)());
                    }}
                    out[i + 1] <== ConstraintPush()(constraints[i + 1], filter, product[i][$B - 2].out);
                }}
                for (var i = $NUM_LIMBS + 1; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
        )
        .to_string();
        template_str = template_str.replace("$NUM_LIMBS", &*self.num_limbs.to_string());
        template_str = template_str.replace("$B", &*B.to_string());

        template_str
    }
}

impl GateVerificationCode for ConstantGate {
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template Constant$NUM_CONSTANTS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                for (var i = 0; i < $NUM_CONSTANTS; i++) {{
                    out[i] <== ConstraintPush()(constraints[i], filter, GlExtSub()(constants[$NUM_SELECTORS + i], wires[i]));
                }}
                for (var i = $NUM_CONSTANTS; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
        ).to_string();
        template_str = template_str.replace("$NUM_CONSTANTS", &*self.num_consts.to_string());
        template_str
    }
}

impl<F, const D: usize> GateVerificationCode for ExponentiationGate<F, D>
where
    F: RichField + Extendable<D>,
{
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template Exponentiation$NUM_POWER_BITS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                out[0] <== ConstraintPush()(constraints[0], filter,
                            GlExtSub()(GlExtMul()(GlExt(1, 0)(),
                                                    GlExtAdd()(GlExtMul()(wires[$NUM_POWER_BITS], wires[0]),
                                                            GlExtSub()(GlExt(1, 0)(), wires[$NUM_POWER_BITS])
                                                            )
                                                    ),
                                        wires[$NUM_POWER_BITS + 2]));
                for (var i = 1; i < $NUM_POWER_BITS; i++) {{
                    // prev_intermediate_value * (cur_bit * wires[0] + (1 - cur_bit)) - wires[$NUM_POWER_BITS + 2 + i]
                    out[i] <== ConstraintPush()(constraints[i], filter,
                                GlExtSub()(GlExtMul()(GlExtSquare()(wires[$NUM_POWER_BITS + 1 + i]),
                                                    GlExtAdd()(GlExtMul()(wires[$NUM_POWER_BITS - i], wires[0]),
                                                                GlExtSub()(GlExt(1, 0)(), wires[$NUM_POWER_BITS - i])
                                                                )
                                                    ),
                                        wires[$NUM_POWER_BITS + 2 + i]));
                }}
                out[$NUM_POWER_BITS] <== ConstraintPush()(constraints[$NUM_POWER_BITS], filter, GlExtSub()(wires[$NUM_POWER_BITS + 1], wires[2 * $NUM_POWER_BITS + 1]));
                for (var i = $NUM_POWER_BITS + 1; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
            ).to_string();
        template_str = template_str.replace("$NUM_POWER_BITS", &*self.num_power_bits.to_string());
        template_str
    }
}

impl<const D: usize> GateVerificationCode for MulExtensionGate<D> {
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template MultiplicationExtension$NUM_OPS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                signal m[$NUM_OPS][2][2];
                for (var i = 0; i < $NUM_OPS; i++) {{
                    m[i] <== WiresAlgebraMul(3 * $D * i, 3 * $D * i + $D)(wires);
                    for (var j = 0; j < $D; j++) {{
                    out[i * $D + j] <== ConstraintPush()(constraints[i * $D + j], filter, GlExtSub()(wires[3 * $D * i + 2 * $D + j], GlExtMul()(m[i][j], constants[$NUM_SELECTORS])));
                    }}
                }}
                for (var i = $NUM_OPS * $D; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
        ).to_string();
        template_str = template_str.replace("$NUM_OPS", &*self.num_ops.to_string());
        template_str = template_str.replace("$D", &*D.to_string());
        template_str
    }
}

impl<F, const D: usize> GateVerificationCode for PoseidonGate<F, D>
where
    F: RichField + Extendable<D>,
{
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template Poseidon12() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                var index = 0;
                out[index] <== ConstraintPush()(constraints[index], filter, GlExtMul()(wires[$WIRE_SWAP], GlExtSub()(wires[$WIRE_SWAP], GlExt(1, 0)())));
                index++;
                for (var i = 0; i < 4; i++) {{
                    out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(GlExtMul()(wires[$WIRE_SWAP], GlExtSub()(wires[i + 4], wires[i])), wires[$START_DELTA + i]));
                    index++;
                }}
                // SPONGE_RATE = 8
                // SPONGE_CAPACITY = 4
                // SPONGE_WIDTH = 12
                signal state[12][$HALF_N_FULL_ROUNDS * 8 + 2 + $N_PARTIAL_ROUNDS * 2][2];
                var state_round = 0;
                for (var i = 0; i < 4; i++) {{
                    state[i][state_round] <== GlExtAdd()(wires[i], wires[$START_DELTA + i]);
                    state[i + 4][state_round] <== GlExtSub()(wires[i + 4], wires[$START_DELTA + i]);
                }}
                for (var i = 8; i < 12; i++) {{
                    state[i][state_round] <== wires[i];
                }}
                state_round++;
                var round_ctr = 0;
                // First set of full rounds.
                signal mds_row_shf_field[$HALF_N_FULL_ROUNDS][12][13][2];
                for (var r = 0; r < $HALF_N_FULL_ROUNDS; r ++) {{
                    for (var i = 0; i < 12; i++) {{
                    state[i][state_round] <== GlExtAdd()(state[i][state_round - 1], GlExt(GL_CONST(i + 12 * round_ctr), 0)());
                    }}
                    state_round++;
                    if (r != 0 ) {{
                    for (var i = 0; i < 12; i++) {{
                        state[i][state_round] <== wires[$START_DELTA + 4 + 12 * (r - 1) + i];
                        out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(state[i][state_round - 1], state[i][state_round]));
                        index++;
                    }}
                    state_round++;
                    }}
                    for (var i = 0; i < 12; i++) {{
                    state[i][state_round] <== GlExtExpN(3)(state[i][state_round - 1], 7);
                    }}
                    state_round++;
                    for (var i = 0; i < 12; i++) {{ // for r
                    mds_row_shf_field[r][i][0][0] <== 0;
                    mds_row_shf_field[r][i][0][1] <== 0;
                    for (var j = 0; j < 12; j++) {{ // for i,
                        mds_row_shf_field[r][i][j + 1] <== GlExtAdd()(mds_row_shf_field[r][i][j], GlExtMul()(state[(i + j) < 12 ? (i + j) : (i + j - 12)][state_round - 1], GlExt(MDS_MATRIX_CIRC(j), 0)()));
                    }}
                    state[i][state_round] <== GlExtAdd()(mds_row_shf_field[r][i][12], GlExtMul()(state[i][state_round - 1], GlExt(MDS_MATRIX_DIAG(i), 0)()));
                    }}
                    state_round++;
                    round_ctr++;
                }}
                // Partial rounds.
                for (var i = 0; i < 12; i++) {{
                    state[i][state_round] <== GlExtAdd()(state[i][state_round - 1], GlExt(FAST_PARTIAL_FIRST_ROUND_CONSTANT(i), 0)());
                }}
                state_round++;
                component partial_res[11][11];
                state[0][state_round] <== state[0][state_round - 1];
                for (var r = 0; r < 11; r++) {{
                    for (var c = 0; c < 11; c++) {{
                    partial_res[r][c] = GlExtAdd();
                    if (r == 0) {{
                        partial_res[r][c].a <== GlExt(0, 0)();
                    }} else {{
                        partial_res[r][c].a <== partial_res[r - 1][c].out;
                    }}
                    partial_res[r][c].b <== GlExtMul()(state[r + 1][state_round - 1], GlExt(FAST_PARTIAL_ROUND_INITIAL_MATRIX(r, c), 0)());
                    }}
                }}
                for (var i = 1; i < 12; i++) {{
                    state[i][state_round] <== partial_res[10][i - 1].out;
                }}
                state_round++;
                signal partial_d[12][$N_PARTIAL_ROUNDS][2];
                for (var r = 0; r < $N_PARTIAL_ROUNDS; r++) {{
                    out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(state[0][state_round - 1], wires[$START_PARTIAL + r]));
                    index++;
                    if (r == $N_PARTIAL_ROUNDS - 1) {{
                    state[0][state_round] <== GlExtExpN(3)(wires[$START_PARTIAL + r], 7);
                    }} else {{
                    state[0][state_round] <== GlExtAdd()(GlExt(FAST_PARTIAL_ROUND_CONSTANTS(r), 0)(), GlExtExpN(3)(wires[$START_PARTIAL + r], 7));
                    }}
                    for (var i = 1; i < 12; i++) {{
                    state[i][state_round] <== state[i][state_round - 1];
                    }}
                    partial_d[0][r] <== GlExtMul()(state[0][state_round], GlExt(MDS_MATRIX_CIRC(0) + MDS_MATRIX_DIAG(0), 0)());
                    for (var i = 1; i < 12; i++) {{
                    partial_d[i][r] <== GlExtAdd()(partial_d[i - 1][r], GlExtMul()(state[i][state_round], GlExt(FAST_PARTIAL_ROUND_W_HATS(r, i - 1), 0)()));
                    }}
                    state_round++;
                    state[0][state_round] <== partial_d[11][r];
                    for (var i = 1; i < 12; i++) {{
                    state[i][state_round] <== GlExtAdd()(state[i][state_round - 1], GlExtMul()(state[0][state_round - 1], GlExt(FAST_PARTIAL_ROUND_VS(r, i - 1), 0)()));
                    }}
                    state_round++;
                }}
                round_ctr += $N_PARTIAL_ROUNDS;
                // Second set of full rounds.
                signal mds_row_shf_field2[$HALF_N_FULL_ROUNDS][12][13][2];
                for (var r = 0; r < $HALF_N_FULL_ROUNDS; r ++) {{
                    for (var i = 0; i < 12; i++) {{
                    state[i][state_round] <== GlExtAdd()(state[i][state_round - 1], GlExt(GL_CONST(i + 12 * round_ctr), 0)());
                    }}
                    state_round++;
                    for (var i = 0; i < 12; i++) {{
                    state[i][state_round] <== wires[$START_FULL_1 + 12 * r + i];
                    out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(state[i][state_round - 1], state[i][state_round]));
                    index++;
                    }}
                    state_round++;
                    for (var i = 0; i < 12; i++) {{
                    state[i][state_round] <== GlExtExpN(3)(state[i][state_round - 1], 7);
                    }}
                    state_round++;
                    for (var i = 0; i < 12; i++) {{ // for r
                    mds_row_shf_field2[r][i][0][0] <== 0;
                    mds_row_shf_field2[r][i][0][1] <== 0;
                    for (var j = 0; j < 12; j++) {{ // for i,
                        mds_row_shf_field2[r][i][j + 1] <== GlExtAdd()(mds_row_shf_field2[r][i][j], GlExtMul()(state[(i + j) < 12 ? (i + j) : (i + j - 12)][state_round - 1], GlExt(MDS_MATRIX_CIRC(j), 0)()));
                    }}
                    state[i][state_round] <== GlExtAdd()(mds_row_shf_field2[r][i][12], GlExtMul()(state[i][state_round - 1], GlExt(MDS_MATRIX_DIAG(i), 0)()));
                    }}
                    state_round++;
                    round_ctr++;
                }}
                for (var i = 0; i < 12; i++) {{
                    out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(state[i][state_round - 1], wires[12 + i]));
                    index++;
                }}
                for (var i = index + 1; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}
                function FAST_PARTIAL_ROUND_W_HATS(i, j) {{
                var value[$N_PARTIAL_ROUNDS][11];
                $SET_FAST_PARTIAL_ROUND_W_HATS;
                return value[i][j];
                }}
                function FAST_PARTIAL_ROUND_VS(i, j) {{
                var value[$N_PARTIAL_ROUNDS][11];
                $SET_FAST_PARTIAL_ROUND_VS;
                return value[i][j];
                }}
                function FAST_PARTIAL_ROUND_INITIAL_MATRIX(i, j) {{
                var value[11][11];
                $SET_FAST_PARTIAL_ROUND_INITIAL_MATRIX;
                return value[i][j];
                }}
                function FAST_PARTIAL_ROUND_CONSTANTS(i) {{
                var value[$N_PARTIAL_ROUNDS];
                $SET_FAST_PARTIAL_ROUND_CONSTANTS;
                return value[i];
                }}
                function FAST_PARTIAL_FIRST_ROUND_CONSTANT(i) {{
                var value[12];
                $SET_FAST_PARTIAL_FIRST_ROUND_CONSTANT;
                return value[i];
                }}
                function MDS_MATRIX_CIRC(i) {{
                var mds[12];
                $SET_MDS_MATRIX_CIRC;
                return mds[i];
                }}
                function MDS_MATRIX_DIAG(i) {{
                var mds[12];
                $SET_MDS_MATRIX_DIAG;
                return mds[i];
                }}"
        ).to_string();
        template_str = template_str.replace("$WIRE_SWAP", &*Self::WIRE_SWAP.to_string());
        template_str = template_str.replace("$START_DELTA", &*Self::START_DELTA.to_string());
        template_str = template_str.replace("$START_FULL_1", &*Self::START_FULL_1.to_string());
        template_str = template_str.replace(
            "$HALF_N_FULL_ROUNDS",
            &*poseidon::HALF_N_FULL_ROUNDS.to_string(),
        );
        template_str = template_str.replace(
            "$N_PARTIAL_ROUNDS",
            &*poseidon::N_PARTIAL_ROUNDS.to_string(),
        );
        template_str = template_str.replace("$START_PARTIAL", &*Self::START_PARTIAL.to_string());

        let mut partial_const_str = "".to_owned();
        for i in 0..poseidon::N_PARTIAL_ROUNDS {
            partial_const_str += &*("  value[".to_owned()
                + &*i.to_string()
                + "] = "
                + &*<F as Poseidon>::FAST_PARTIAL_ROUND_CONSTANTS[i].to_string()
                + ";\n");
        }
        template_str = template_str.replace(
            "  $SET_FAST_PARTIAL_ROUND_CONSTANTS;\n",
            &*partial_const_str,
        );

        let mut circ_str = "".to_owned();
        for i in 0..12 {
            circ_str += &*("  mds[".to_owned()
                + &*i.to_string()
                + "] = "
                + &*<F as Poseidon>::MDS_MATRIX_CIRC[i].to_string()
                + ";\n");
        }
        template_str = template_str.replace("  $SET_MDS_MATRIX_CIRC;\n", &*circ_str);

        let mut diag_str = "".to_owned();
        for i in 0..12 {
            diag_str += &*("  mds[".to_owned()
                + &*i.to_string()
                + "] = "
                + &*<F as Poseidon>::MDS_MATRIX_DIAG[i].to_string()
                + ";\n");
        }
        template_str = template_str.replace("  $SET_MDS_MATRIX_DIAG;\n", &*diag_str);

        let mut first_round_const_str = "".to_owned();
        for i in 0..12 {
            first_round_const_str += &*("  value[".to_owned()
                + &*i.to_string()
                + "] = "
                + &*<F as Poseidon>::FAST_PARTIAL_FIRST_ROUND_CONSTANT[i].to_string()
                + ";\n");
        }
        template_str = template_str.replace(
            "  $SET_FAST_PARTIAL_FIRST_ROUND_CONSTANT;\n",
            &*first_round_const_str,
        );

        let mut init_m_str = "".to_owned();
        for i in 0..11 {
            for j in 0..11 {
                init_m_str += &*("  value[".to_owned()
                    + &*i.to_string()
                    + "]["
                    + &*j.to_string()
                    + "] = "
                    + &*<F as Poseidon>::FAST_PARTIAL_ROUND_INITIAL_MATRIX[i][j].to_string()
                    + ";\n");
            }
        }
        template_str =
            template_str.replace("  $SET_FAST_PARTIAL_ROUND_INITIAL_MATRIX;\n", &*init_m_str);

        let mut partial_hats_str = "".to_owned();
        for i in 0..poseidon::N_PARTIAL_ROUNDS {
            for j in 0..11 {
                partial_hats_str += &*("  value[".to_owned()
                    + &*i.to_string()
                    + "]["
                    + &*j.to_string()
                    + "] = "
                    + &*<F as Poseidon>::FAST_PARTIAL_ROUND_W_HATS[i][j].to_string()
                    + ";\n");
            }
        }
        template_str =
            template_str.replace("  $SET_FAST_PARTIAL_ROUND_W_HATS;\n", &*partial_hats_str);

        let mut partial_vs_str = "".to_owned();
        for i in 0..poseidon::N_PARTIAL_ROUNDS {
            for j in 0..11 {
                partial_vs_str += &*("  value[".to_owned()
                    + &*i.to_string()
                    + "]["
                    + &*j.to_string()
                    + "] = "
                    + &*<F as Poseidon>::FAST_PARTIAL_ROUND_VS[i][j].to_string()
                    + ";\n");
            }
        }
        template_str = template_str.replace("  $SET_FAST_PARTIAL_ROUND_VS;\n", &*partial_vs_str);

        template_str
    }
}

impl<F, const D: usize> GateVerificationCode for PoseidonMdsGate<F, D>
where
    F: RichField + Extendable<D>,
{
    fn export_verification_code(&self) -> String {
        assert_eq!(D, 2);
        assert_eq!(poseidon::SPONGE_WIDTH, 12);
        let template_str = format!(
            "template PoseidonMdsGate12() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                signal state[13][12][2][2];
                for (var r = 0; r < 12; r++) {{
                    for (var i = 0; i < 12; i++) {{
                    var j = i + r >= 12 ? i + r - 12 : i + r;
                    if (i == 0) {{
                        state[i][r][0] <== GlExtScalarMul()(wires[j * 2], MDS_MATRIX_CIRC(i));
                        state[i][r][1] <== GlExtScalarMul()(wires[j * 2 + 1], MDS_MATRIX_CIRC(i));
                    }} else {{
                        state[i][r][0] <== GlExtAdd()(state[i - 1][r][0], GlExtScalarMul()(wires[j * 2], MDS_MATRIX_CIRC(i)));
                        state[i][r][1] <== GlExtAdd()(state[i - 1][r][1], GlExtScalarMul()(wires[j * 2 + 1], MDS_MATRIX_CIRC(i)));
                    }}
                    }}
                    state[12][r][0] <== GlExtAdd()(state[11][r][0], GlExtScalarMul()(wires[r * 2], MDS_MATRIX_DIAG(r)));
                    state[12][r][1] <== GlExtAdd()(state[11][r][1], GlExtScalarMul()(wires[r * 2 + 1], MDS_MATRIX_DIAG(r)));
                }}
                for (var r = 0; r < 12; r ++) {{
                    out[r * 2] <== ConstraintPush()(constraints[r * 2], filter, GlExtSub()(wires[(12 + r) * 2], state[12][r][0]));
                    out[r * 2 + 1] <== ConstraintPush()(constraints[r * 2 + 1], filter, GlExtSub()(wires[(12 + r) * 2 + 1], state[12][r][1]));
                }}
                for (var i = 24; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
        ).to_string();
        template_str
    }
}

impl GateVerificationCode for PublicInputGate {
    fn export_verification_code(&self) -> String {
        format!(
            "template PublicInputGateLib() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                signal hashes[4][2];
                for (var i = 0; i < 4; i++) {{
                    hashes[i][0] <== public_input_hash[i];
                    hashes[i][1] <== 0;
                    out[i] <== ConstraintPush()(constraints[i], filter, GlExtSub()(wires[i], hashes[i]));
                }}
                for (var i = 4; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}"
        )
    }
}

impl<F, const D: usize> GateVerificationCode for RandomAccessGate<F, D>
where
    F: RichField + Extendable<D>,
{
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template RandomAccessB$BITSC$NUM_COPIESE$NUM_EXTRA_CONSTANTS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                var index = 0;
                signal acc[$NUM_COPIES][$BITS][2];
                signal list_items[$NUM_COPIES][$BITS + 1][$VEC_SIZE][2];
                for (var copy = 0; copy < $NUM_COPIES; copy++) {{
                    for (var i = 0; i < $BITS; i++) {{
                    out[index] <== ConstraintPush()(constraints[index], filter,
                        GlExtMul()(wires[ra_wire_bit(i, copy)], GlExtSub()(wires[ra_wire_bit(i, copy)], GlExt(1, 0)())));
                    index++;
                    }}
                    for (var i = $BITS; i > 0; i--) {{
                    if(i == $BITS) {{
                        acc[copy][i - 1] <== wires[ra_wire_bit(i - 1, copy)];
                    }} else {{
                        acc[copy][i - 1] <== GlExtAdd()(GlExtAdd()(acc[copy][i], acc[copy][i]), wires[ra_wire_bit(i - 1, copy)]);
                    }}
                    }}
                    out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(acc[copy][0], wires[(2 + $VEC_SIZE) * copy]));
                    index++;
                    for (var i = 0; i < $VEC_SIZE; i++) {{
                    list_items[copy][0][i] <== wires[(2 + $VEC_SIZE) * copy + 2 + i];
                    }}
                    for (var i = 0; i < $BITS; i++) {{
                    for (var j = 0; j < ($VEC_SIZE >> i); j = j + 2) {{
                        list_items[copy][i + 1][j \\ 2] <== GlExtAdd()(list_items[copy][i][j], GlExtMul()(wires[ra_wire_bit(i, copy)], GlExtSub()(list_items[copy][i][j + 1], list_items[copy][i][j])));
                    }}
                    }}
                    out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(list_items[copy][$BITS][0], wires[(2 + $VEC_SIZE) * copy + 1]));
                    index++;
                }}
                for (var i = 0; i < $NUM_EXTRA_CONSTANTS; i++) {{
                    out[index] <== ConstraintPush()(constraints[index], filter, GlExtSub()(constants[$NUM_SELECTORS + i], wires[(2 + $VEC_SIZE) * $NUM_COPIES + i]));
                    index++;
                }}
                for (var i = index; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}
                function ra_wire_bit(i, copy) {{
                return $NUM_ROUTED_WIRES + copy * $BITS + i;
                }}"
        ).to_string();
        template_str = template_str.replace("$BITS", &*self.bits.to_string());
        template_str = template_str.replace("$VEC_SIZE", &*self.vec_size().to_string());
        template_str =
            template_str.replace("$NUM_ROUTED_WIRES", &*self.num_routed_wires().to_string());
        template_str = template_str.replace("$NUM_COPIES", &*self.num_copies.to_string());
        template_str = template_str.replace(
            "$NUM_EXTRA_CONSTANTS",
            &*self.num_extra_constants.to_string(),
        );
        template_str
    }
}

impl<const D: usize> GateVerificationCode for ReducingGate<D> {
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template Reducing$NUM_COEFFS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                var acc_start = 2 * $D;
                signal m[$NUM_COEFFS][2][2];
                for (var i = 0; i < $NUM_COEFFS; i++) {{
                    m[i] <== WiresAlgebraMul(acc_start, $D)(wires);
                    out[i * $D] <== ConstraintPush()(constraints[i * $D], filter, GlExtAdd()(m[i][0], GlExtSub()(wires[3 * $D + i], wires[r_wires_accs_start(i, $NUM_COEFFS)])));
                    for (var j = 1; j < $D; j++) {{
                    out[i * $D + j] <== ConstraintPush()(constraints[i * $D + j], filter, GlExtSub()(m[i][j], wires[r_wires_accs_start(i, $NUM_COEFFS) + j]));
                    }}
                    acc_start = r_wires_accs_start(i, $NUM_COEFFS);
                }}
                for (var i = $NUM_COEFFS * $D; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}
                function r_wires_accs_start(i, num_coeffs) {{
                if (i == num_coeffs - 1) return 0;
                else return (3 + i) * $D + num_coeffs;
                }}"
        ).to_string();

        template_str = template_str.replace("$NUM_COEFFS", &*self.num_coeffs.to_string());
        template_str = template_str.replace("$D", &*D.to_string());

        template_str
    }
}

impl<const D: usize> GateVerificationCode for ReducingExtensionGate<D> {
    fn export_verification_code(&self) -> String {
        let mut template_str = format!(
            "template ReducingExtension$NUM_COEFFS() {{
                signal input constants[NUM_OPENINGS_CONSTANTS()][2];
                signal input wires[NUM_OPENINGS_WIRES()][2];
                signal input public_input_hash[4];
                signal input constraints[NUM_GATE_CONSTRAINTS()][2];
                signal output out[NUM_GATE_CONSTRAINTS()][2];
                signal filter[2];
                $SET_FILTER;
                var acc_start = 2 * $D;
                signal m[$NUM_COEFFS][2][2];
                for (var i = 0; i < $NUM_COEFFS; i++) {{
                    m[i] <== WiresAlgebraMul(acc_start, $D)(wires);
                    for (var j = 0; j < $D; j++) {{
                    out[i * $D + j] <== ConstraintPush()(constraints[i * $D + j], filter, GlExtAdd()(m[i][j], GlExtSub()(wires[(3 + i) * $D + j], wires[re_wires_accs_start(i, $NUM_COEFFS) + j])));
                    }}
                    acc_start = re_wires_accs_start(i, $NUM_COEFFS);
                }}
                for (var i = $NUM_COEFFS * $D; i < NUM_GATE_CONSTRAINTS(); i++) {{
                    out[i] <== constraints[i];
                }}
                }}
                function re_wires_accs_start(i, num_coeffs) {{
                if (i == num_coeffs - 1) return 0;
                else return (3 + i + num_coeffs) * $D;
                }}"
        ).to_string();

        template_str = template_str.replace("$NUM_COEFFS", &*self.num_coeffs.to_string());
        template_str = template_str.replace("$D", &*D.to_string());

        template_str
    }
}
