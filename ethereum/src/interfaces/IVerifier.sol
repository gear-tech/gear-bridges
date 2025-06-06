// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

interface IVerifier {
    function verifyProof(
        bytes calldata proof,
        uint256[] calldata public_inputs
    ) external view returns (bool);
}
