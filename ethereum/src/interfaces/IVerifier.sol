// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

/**
 * @dev Interface for the Verifier contract.
 */
interface IVerifier {
    /**
     * @dev Authenticates a Vara queue root with a BEEFY-backed MMR proof.
     * @param sourceBlock Original source block number, bounded to uint32.
     * @param merkleRoot Queue root at the source block.
     * @param proof Canonical v2 queue proof envelope.
     * @return success `true` if proof is valid, `false` otherwise, including malformed proofs.
     */
    function safeVerifyProof(uint256 sourceBlock, bytes32 merkleRoot, bytes calldata proof)
        external
        view
        returns (bool success);
}
