import { ethers } from "hardhat";
const publics = require("../../aggregation/final_public.json");
const proof = require("../../aggregation/final_proof.json");

async function main() {
  const verifierFactory = await ethers.getContractFactory("ValidatorSetChangeVerifier");
  const  circuitDigestAndMerkleRoots = publics.slice(0, 68);
  const validatorSet = publics.slice(68, 73);
  const verifier = await verifierFactory.deploy(
    circuitDigestAndMerkleRoots, validatorSet
  );
  await verifier.deployed();

  console.log("Verifier deployed at ", verifier.address);

  // const nextValidatorSet = publics.slice(73);
  // console.log(nextValidatorSet);
  
  // await verifier.verifyValidatorSetChangeProof(
  //   [proof.pi_a[0], proof.pi_a[1]],
  //   [
  //     [proof.pi_b[0][1], proof.pi_b[0][0]],
  //     [proof.pi_b[1][1], proof.pi_b[1][0]],
  //   ],
  //   [proof.pi_c[0], proof.pi_c[1]],
  //   nextValidatorSet,
  // )
    
  //   console.log(await verifier.getVerified());
  //   console.log(await verifier.getValidatorSet());
}

// We recommend this pattern to be able to use async/await everywhere
// and properly handle errors.
main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});