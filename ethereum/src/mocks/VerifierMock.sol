// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IVerifier} from "../interfaces/IVerifier.sol";

/**
 * @dev Mock Verifier smart contract is responsible for verifying zk-SNARK Plonk proofs.
 *      It is used for testing purposes.
 */
contract VerifierMock is IVerifier {
    /**
     * @dev See {IVerifier-verifyProof}.
     */
    function verifyProof(bytes calldata, uint256[] calldata) external pure returns (bool) {
        return true;
    }
}
