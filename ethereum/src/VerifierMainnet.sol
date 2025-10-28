// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IPlonkVerifier} from "./interfaces/IPlonkVerifier.sol";
import {IVerifier} from "./interfaces/IVerifier.sol";
import {PlonkVerifierMainnet} from "./libraries/PlonkVerifierMainnet.sol";

/**
 * @dev VerifierMainnet smart contract is responsible for verifying zk-SNARK Plonk proofs.
 *      This is done with help of PlonkVerifierMainnet smart contract.
 */
contract VerifierMainnet is IVerifier, IPlonkVerifier, PlonkVerifierMainnet {
    /**
     * @dev See {IVerifier-safeVerifyProof}.
     */
    function safeVerifyProof(bytes calldata proof, uint256[] calldata publicInputs) external view returns (bool) {
        try IPlonkVerifier(address(this)).verifyProof(proof, publicInputs) returns (bool success) {
            return success;
        } catch {
            return false;
        }
    }
}
