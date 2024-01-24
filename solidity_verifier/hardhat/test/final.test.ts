const { expect } = require("chai");
const { ethers } = require("hardhat");

const proof_for_validator_change = require("../../aggregation/vs_change/final_proof.json");
const publics_for_validator_change = require("../../aggregation/vs_change/final_public.json");

const proof_for_message_sent = require("../../aggregation/message_sent/final_proof.json");
const publics_for_message_sent = require("../../aggregation/message_sent/final_public.json");

describe("Validator set change verifier test", function () {
  it("Groth16 Verify set change", async function () {
    const verifierFactory = await ethers.getContractFactory("ValidatorSetChangeVerifier");
    const  circuitDigestAndMerkleRoots = publics_for_validator_change.slice(0, 8);
    const validatorSet = publics_for_validator_change.slice(8, 13);
           
    const verifier = await verifierFactory.deploy(
      circuitDigestAndMerkleRoots, validatorSet, 232
    );
    
    await verifier.deployed();
    
    const solProof = [
      [proof_for_validator_change.pi_a[0], proof_for_validator_change.pi_a[1]],
      [
        [proof_for_validator_change.pi_b[0][1], proof_for_validator_change.pi_b[0][0]],
        [proof_for_validator_change.pi_b[1][1], proof_for_validator_change.pi_b[1][0]],
      ],
      [proof_for_validator_change.pi_c[0], proof_for_validator_change.pi_c[1]],
    ];

    const nextValidatorSet = publics_for_validator_change.slice(13,18);

    await expect(verifier.verifyValidatorSetChangeProof(
      solProof[0],
      solProof[1],
      solProof[2],
      nextValidatorSet,
      233,
    )).to.emit(verifier, "SuccessfulVerification")
      .withArgs(nextValidatorSet);
    
  });
});

describe("Message sent verifier test", function () {
  it("Groth16 Verify message sent", async function () {
    const verifierFactory = await ethers.getContractFactory("MessageSentVerifier");
    const  circuitDigestAndMerkleRoots = publics_for_message_sent.slice(0, 8);
    const validatorSet = publics_for_message_sent.slice(8, 13);
           
    const verifier = await verifierFactory.deploy(
      circuitDigestAndMerkleRoots, validatorSet, 230
    );
    
    await verifier.deployed();
    
    const solProof = [
      [proof_for_message_sent.pi_a[0], proof_for_message_sent.pi_a[1]],
      [
        [proof_for_message_sent.pi_b[0][1], proof_for_message_sent.pi_b[0][0]],
        [proof_for_message_sent.pi_b[1][1], proof_for_message_sent.pi_b[1][0]],
      ],
      [proof_for_message_sent.pi_c[0], proof_for_message_sent.pi_c[1]],
    ];

    const nextValidatorSet = publics_for_message_sent.slice(14);

    await expect(verifier.verifyMsgSentProof(
      solProof[0],
      solProof[1],
      solProof[2],
      nextValidatorSet,
      231,
    )).to.emit(verifier, "SuccessfulVerification")
      .withArgs(nextValidatorSet);
    
  });
});
