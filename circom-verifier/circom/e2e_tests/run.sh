#!/bin/bash
set -e

export NODE_OPTIONS="--max-old-space-size=16384"
source ~/.bashrc 

CUR_DIR=$(cd $(dirname $0);pwd)

POWER=26
BIG_POWER=27
SRS=${CUR_DIR}/../../../eigen-zkvm/keys/setup_2^${POWER}.ptau

CIRCUIT_NAME=plonky2

WORK_DIR=${CUR_DIR}/../test/data

# SNARK_CIRCOM=$WORK_DIR/$CIRCUIT_NAME.circom
SNARK_INPUT=$WORK_DIR/proof.json

RUNDIR="${CUR_DIR}/../starkjs"


if [ "$1" = "true" ]; then 
    echo "compile circom and generate wasm and r1cs"
    circom $CUR_DIR/../circuits/$CIRCUIT_NAME.circom --wasm --r1cs -p bn128 --O2=full -o $WORK_DIR
    # cp $WORK_DIR/$CIRCUIT_NAME"_js"/$CIRCUIT_NAME.wasm /tmp/aggregation/circuits.wasm
fi 



# if [ ! -f $SRS ]; then
#     echo "downloading powersOfTau28_hez_final_${POWER}.ptau"
#     curl https://hermez.s3-eu-west-1.amazonaws.com/powersOfTau28_hez_final_${POWER}.ptau -o $SRS
# fi

echo ">>> groth16 scheme <<< "
echo "1. groth16 setup"
snarkjs g16s $WORK_DIR/$CIRCUIT_NAME.r1cs $SRS  $WORK_DIR/g16.zkey

echo "2. groth16 fullprove"
snarkjs g16f $SNARK_INPUT $WORK_DIR/$CIRCUIT_NAME"_js"/$CIRCUIT_NAME.wasm  $WORK_DIR/g16.zkey $WORK_DIR/g16_proof.json $WORK_DIR/g16_public.json

echo "3. generate verification_key"
snarkjs zkev  $WORK_DIR/g16.zkey  $WORK_DIR/verification_key.json

echo "4. verify groth16 proof"
snarkjs g16v $WORK_DIR/verification_key.json $WORK_DIR/public.json $WORK_DIR/proof.json

cp $WORK_DIR/g16_public.json /tmp/aggregation/final_public.json 
cp $WORK_DIR/g16_proof.json /tmp/aggregation/final_proof.json

echo "5. generate verifier contract"
snarkjs zkesv  $WORK_DIR/g16.zkey  ${CUR_DIR}/hardhat/contracts/final_verifier.sol

echo "6. calculate verify gas cost"
cd hardhat && npm install && npx hardhat test test/final.test.ts