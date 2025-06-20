// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IVerifier} from "./interfaces/IVerifier.sol";
import {PlonkVerifier} from "./libraries/PlonkVerifier.sol";

/**
 * @dev Verifier smart contract is responsible for verifying zk-SNARK Plonk proofs.
 *      This is done with help of PlonkVerifier smart contract.
 */
contract Verifier is IVerifier, PlonkVerifier {
    /**
     * @dev See {IVerifier-verifyProof}.
     */
    function verifyProof(bytes calldata proof, uint256[] calldata publicInputs) external view returns (bool) {
        return Verify(proof, publicInputs);
    }
}
