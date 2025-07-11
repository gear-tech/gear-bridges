// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IVerifier} from "src/interfaces/IVerifier.sol";
import {IVerifierMock} from "src/interfaces/IVerifierMock.sol";

/**
 * @dev Mock Verifier smart contract is responsible for verifying zk-SNARK Plonk proofs.
 *      It is used for testing purposes.
 */
contract VerifierMock is IVerifier, IVerifierMock {
    bool private _value;

    /**
     * @dev Initializes the VerifierMock.
     * @param value value to return from `safeVerifyProof` function.
     */
    constructor(bool value) {
        _value = value;
    }

    /**
     * @dev Sets the value.
     * @param value value to return from `safeVerifyProof` function.
     */
    function setValue(bool value) external {
        _value = value;
    }

    /**
     * @dev See {IVerifier-safeVerifyProof}.
     */
    function safeVerifyProof(bytes calldata, uint256[] calldata) external view returns (bool) {
        return _value;
    }
}
