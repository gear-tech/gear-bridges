const { expect } = require("chai");
const { ethers } = require("hardhat");

const proof = require("../../aggregation/final_proof.json");
const publics = require("../../aggregation/final_public.json");

describe("Plonk verifier test", function () {
  it("Groth16 Verify", async function () {
    const verifierFactory = await ethers.getContractFactory("Groth16Verifier");
    const verifier = await verifierFactory.deploy();

    await verifier.deployed();

    
    const solProof = [
      [proof.pi_a[0], proof.pi_a[1]],
      [
        [proof.pi_b[0][1], proof.pi_b[0][0]],
        [proof.pi_b[1][1], proof.pi_b[1][0]],
      ],
      [proof.pi_c[0], proof.pi_c[1]],
    ];

    expect(await verifier.verifyProof(
      solProof[0],
      solProof[1],
      solProof[2],
      publics,
    )).to.equal(true);
  });
});

describe("Validator set change verifier test", function () {
  it("Groth16 Verify set change", async function () {
    const verifierFactory = await ethers.getContractFactory("ValidatorSetChangeVerifier");
    const  circuitDigestAndMerkleRoots = publics.slice(0, 68);
    const validatorSet = publics.slice(68, 73);
    const verifier = await verifierFactory.deploy(
      circuitDigestAndMerkleRoots, validatorSet
    );

    await verifier.deployed();

    
    const solProof = [
      [proof.pi_a[0], proof.pi_a[1]],
      [
        [proof.pi_b[0][1], proof.pi_b[0][0]],
        [proof.pi_b[1][1], proof.pi_b[1][0]],
      ],
      [proof.pi_c[0], proof.pi_c[1]],
    ];

    const nextValidatorSet = publics.slice(73);
    console.log(nextValidatorSet);
    
    expect(await verifier.verifyValidatorSetChangeProof(
      solProof[0],
      solProof[1],
      solProof[2],
      nextValidatorSet,
    )).to.emit(verifier, "SuccessfulVerification")
      .withArgs(nextValidatorSet);  
      
      expect(await verifier.getVerified()).to.equal(true);
  });
});
