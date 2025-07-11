// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IVerifier} from "./IVerifier.sol";

/**
 * @dev Interface for the VerifierMock contract.
 */
interface IVerifierMock is IVerifier {
    /**
     * @dev Sets the value.
     * @param value value to return from `safeVerifyProof` function.
     */
    function setValue(bool value) external;
}
