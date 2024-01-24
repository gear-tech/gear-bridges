import { ethers } from "hardhat";
const publics_for_validator_change = require("../../aggregation/vs_change/final_public.json");
const publics_for_message_sent = require("../../aggregation/message_sent/final_public.json");

async function main() {
  const setChangeVerifierFactory = await ethers.getContractFactory("ValidatorSetChangeVerifier");
  let circuitDigestAndMerkleRoots = publics_for_validator_change.slice(0, 8);
  let validatorSet = publics_for_validator_change.slice(8, 13);
  const setChangeVerifier = await setChangeVerifierFactory.deploy(
    circuitDigestAndMerkleRoots, validatorSet, 232
  );
  await setChangeVerifier.deployed();

  console.log("SetChangeVerifier deployed at ", setChangeVerifier.address);

  const msgSentVerifierFactory = await ethers.getContractFactory("MessageSentVerifier");
  circuitDigestAndMerkleRoots = publics_for_message_sent.slice(0, 8);
  validatorSet = publics_for_message_sent.slice(8, 13);
  const msgSentVerifier = await msgSentVerifierFactory.deploy(
    circuitDigestAndMerkleRoots, validatorSet, 230
  );
  await setChangeVerifier.deployed();

  console.log("MsgSentVerifier deployed at ", msgSentVerifier.address);

}

// We recommend this pattern to be able to use async/await everywhere
// and properly handle errors.
main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});