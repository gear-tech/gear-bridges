// This file was generated by verifier.rs

pragma circom 2.0.9;

// Order of Goldilocks field
function Order() { return 18446744069414584321; }
function W() { return 7; }
function DTH_ROOT() { return 18446744069414584320; }

function NUM_WIRES_CAP() { return 16; }
function NUM_PLONK_ZS_PARTIAL_PRODUCTS_CAP() { return 16; }
function NUM_QUOTIENT_POLYS_CAP() { return 16; }

function NUM_OPENINGS_CONSTANTS() { return 5; }
function NUM_OPENINGS_PLONK_SIGMAS() { return 80; }
function NUM_OPENINGS_WIRES() { return 135; }
function NUM_OPENINGS_PLONK_ZS() { return 2; }
function NUM_OPENINGS_PLONK_ZS_NEXT() { return 2; }
function NUM_OPENINGS_PARTIAL_PRODUCTS() { return 18; }
function NUM_OPENINGS_QUOTIENT_POLYS() { return 16; }

function NUM_FRI_COMMIT_ROUND() { return 2; }
function FRI_COMMIT_MERKLE_CAP_HEIGHT() { return 16; }
function NUM_FRI_QUERY_ROUND() { return 28; }
function NUM_FRI_QUERY_INIT_CONSTANTS_SIGMAS_V() { return 85; }
function NUM_FRI_QUERY_INIT_CONSTANTS_SIGMAS_P() { return 12; }
function NUM_FRI_QUERY_INIT_WIRES_V() { return 135; }
function NUM_FRI_QUERY_INIT_WIRES_P() { return 12; }
function NUM_FRI_QUERY_INIT_ZS_PARTIAL_V() { return 20; }
function NUM_FRI_QUERY_INIT_ZS_PARTIAL_P() { return 12; }
function NUM_FRI_QUERY_INIT_QUOTIENT_V() { return 16; }
function NUM_FRI_QUERY_INIT_QUOTIENT_P() { return 12; }
function NUM_FRI_QUERY_STEP0_V() { return 16; }
function NUM_FRI_QUERY_STEP0_P() { return 8; }
function NUM_FRI_QUERY_STEP1_V() { return 16; }
function NUM_FRI_QUERY_STEP1_P() { return 4; }
function NUM_FRI_FINAL_POLY_EXT_V() { return 32; }

function NUM_SIGMA_CAPS() { return 16; }
function GET_SIGMA_CAP(i) {
  var sc[16][4];
  sc[0][0] = 16656805480755392017;
  sc[0][1] = 1288205818690948819;
  sc[0][2] = 660096263630807509;
  sc[0][3] = 3647952404725990310;
  sc[1][0] = 9342331985281856774;
  sc[1][1] = 12107378345294024224;
  sc[1][2] = 8393156612150138974;
  sc[1][3] = 8118907517207740353;
  sc[2][0] = 5569130242190782648;
  sc[2][1] = 12611828235734151804;
  sc[2][2] = 3245517580357017335;
  sc[2][3] = 149797747706732126;
  sc[3][0] = 11490101624699712897;
  sc[3][1] = 11512637407340994160;
  sc[3][2] = 12226637772424721424;
  sc[3][3] = 13260759803321689984;
  sc[4][0] = 3187910594403599813;
  sc[4][1] = 17548188422601131190;
  sc[4][2] = 2119746012929596515;
  sc[4][3] = 5010326730190555414;
  sc[5][0] = 2730702727019063179;
  sc[5][1] = 13616537187018240440;
  sc[5][2] = 9892128193173387165;
  sc[5][3] = 11313537670062686190;
  sc[6][0] = 1871455426070712631;
  sc[6][1] = 18418758760249353570;
  sc[6][2] = 11533489739181286789;
  sc[6][3] = 11367919089537507975;
  sc[7][0] = 15917875714866747675;
  sc[7][1] = 17346539610276087026;
  sc[7][2] = 13994400516260809756;
  sc[7][3] = 9141430554037209404;
  sc[8][0] = 5934690663621555679;
  sc[8][1] = 7470705009607350851;
  sc[8][2] = 14244695914490111833;
  sc[8][3] = 2656856331264984504;
  sc[9][0] = 7798409569340589348;
  sc[9][1] = 12159497113022709192;
  sc[9][2] = 17814347255701873304;
  sc[9][3] = 17352036357226277276;
  sc[10][0] = 7210672799342420604;
  sc[10][1] = 4813091103787318684;
  sc[10][2] = 9467238497900304851;
  sc[10][3] = 17986191893328231527;
  sc[11][0] = 6103772618783755386;
  sc[11][1] = 8668958539867588829;
  sc[11][2] = 1862840129118636715;
  sc[11][3] = 9656193417956410794;
  sc[12][0] = 6357318494124916793;
  sc[12][1] = 16777011416054576861;
  sc[12][2] = 4516632505843711882;
  sc[12][3] = 11137287958225515329;
  sc[13][0] = 10509250887763083571;
  sc[13][1] = 6044221512139546846;
  sc[13][2] = 5176333427215476127;
  sc[13][3] = 6087679355198498536;
  sc[14][0] = 12826683664267929542;
  sc[14][1] = 16188378541188366932;
  sc[14][2] = 11093473289663589626;
  sc[14][3] = 176120682398401437;
  sc[15][0] = 4070466442687566;
  sc[15][1] = 7366840467566282995;
  sc[15][2] = 6350160243995152977;
  sc[15][3] = 9988336175311202040;
  return sc[i];
}

function NUM_REDUCTION_ARITY_BITS() { return 2; }
function REDUCTION_ARITY_BITS() {
  var bits[2];
  bits[0] = 4;
  bits[1] = 4;
  return bits;
}

function G_BY_ARITY_BITS(arity_bits) {
  var g_arity_bits[4];
  g_arity_bits[0] = 18446744069414584320;
  g_arity_bits[1] = 281474976710656;
  g_arity_bits[2] = 18446744069397807105;
  g_arity_bits[3] = 17293822564807737345;
  return g_arity_bits[arity_bits - 1];
}

function G_FROM_DEGREE_BITS() {
  var g[2];
  g[0] = 1532612707718625687;
  g[1] = 0;
  return g;
}

function MULTIPLICATIVE_GROUP_GENERATOR() { return 7; }
function PRIMITIVE_ROOT_OF_UNITY_LDE() { return 6115771955107415310; }
function LOG_SIZE_OF_LDE_DOMAIN() { return 16; }
function NUM_CHALLENGES() { return 2; }
function MIN_FRI_POW_RESPONSE() { return 16; }
function CIRCUIT_DIGEST() {
  var cd[4];
  cd[0] = 12658120547267168142;
  cd[1] = 13511449626833326595;
  cd[2] = 18198016358255438531;
  cd[3] = 337698940268172735;
  return cd;
}
function SPONGE_RATE() { return 8; }
function SPONGE_CAPACITY() { return 4; }
function SPONGE_WIDTH() { return 12; }
function DEGREE_BITS() { return 13; }
function FRI_RATE_BITS() { return 3; }
function NUM_GATE_CONSTRAINTS() { return 123; }
function NUM_PARTIAL_PRODUCTS_TERMS() { return (NUM_OPENINGS_PLONK_SIGMAS() + QUOTIENT_DEGREE_FACTOR() - 1) \ QUOTIENT_DEGREE_FACTOR(); }
function QUOTIENT_DEGREE_FACTOR() { return 8; }
function K_IS(i) {
  var k_is[80];
  k_is[0] = 1;
  k_is[1] = 7;
  k_is[2] = 49;
  k_is[3] = 343;
  k_is[4] = 2401;
  k_is[5] = 16807;
  k_is[6] = 117649;
  k_is[7] = 823543;
  k_is[8] = 5764801;
  k_is[9] = 40353607;
  k_is[10] = 282475249;
  k_is[11] = 1977326743;
  k_is[12] = 13841287201;
  k_is[13] = 96889010407;
  k_is[14] = 678223072849;
  k_is[15] = 4747561509943;
  k_is[16] = 33232930569601;
  k_is[17] = 232630513987207;
  k_is[18] = 1628413597910449;
  k_is[19] = 11398895185373143;
  k_is[20] = 79792266297612001;
  k_is[21] = 558545864083284007;
  k_is[22] = 3909821048582988049;
  k_is[23] = 8922003270666332022;
  k_is[24] = 7113790686420571191;
  k_is[25] = 12903046666114829695;
  k_is[26] = 16534350385145470581;
  k_is[27] = 5059988279530788141;
  k_is[28] = 16973173887300932666;
  k_is[29] = 8131752794619022736;
  k_is[30] = 1582037354089406189;
  k_is[31] = 11074261478625843323;
  k_is[32] = 3732854072722565977;
  k_is[33] = 7683234439643377518;
  k_is[34] = 16889152938674473984;
  k_is[35] = 7543606154233811962;
  k_is[36] = 15911754940807515092;
  k_is[37] = 701820169165099718;
  k_is[38] = 4912741184155698026;
  k_is[39] = 15942444219675301861;
  k_is[40] = 916645121239607101;
  k_is[41] = 6416515848677249707;
  k_is[42] = 8022122801911579307;
  k_is[43] = 814627405137302186;
  k_is[44] = 5702391835961115302;
  k_is[45] = 3023254712898638472;
  k_is[46] = 2716038920875884983;
  k_is[47] = 565528376716610560;
  k_is[48] = 3958698637016273920;
  k_is[49] = 9264146389699333119;
  k_is[50] = 9508792519651578870;
  k_is[51] = 11221315429317299127;
  k_is[52] = 4762231727562756605;
  k_is[53] = 14888878023524711914;
  k_is[54] = 11988425817600061793;
  k_is[55] = 10132004445542095267;
  k_is[56] = 15583798910550913906;
  k_is[57] = 16852872026783475737;
  k_is[58] = 7289639770996824233;
  k_is[59] = 14133990258148600989;
  k_is[60] = 6704211459967285318;
  k_is[61] = 10035992080941828584;
  k_is[62] = 14911712358349047125;
  k_is[63] = 12148266161370408270;
  k_is[64] = 11250886851934520606;
  k_is[65] = 4969231685883306958;
  k_is[66] = 16337877731768564385;
  k_is[67] = 3684679705892444769;
  k_is[68] = 7346013871832529062;
  k_is[69] = 14528608963998534792;
  k_is[70] = 9466542400916821939;
  k_is[71] = 10925564598174000610;
  k_is[72] = 2691975909559666986;
  k_is[73] = 397087297503084581;
  k_is[74] = 2779611082521592067;
  k_is[75] = 1010533508236560148;
  k_is[76] = 7073734557655921036;
  k_is[77] = 12622653764762278610;
  k_is[78] = 14571600075677612986;
  k_is[79] = 9767480182670369297;
  return k_is[i];
}
function NUM_PUBLIC_INPUTS() { return 20; }
