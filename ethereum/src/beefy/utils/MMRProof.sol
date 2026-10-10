// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2023 Snowfork <hello@snowfork.com>
pragma solidity ^0.8.37;

library MMRProof {
    error ProofSizeExceeded();

    uint256 internal constant MAXIMUM_PROOF_SIZE = 256;

    /**
     * @dev Verify inclusion of a leaf in an MMR
     * @param root MMR root hash
     * @param leafHash leaf hash
     * @param proof an array of hashes
     * @param proofOrder a bitfield describing the order of each item (left vs right)
     */
    function verifyLeafProof(bytes32 root, bytes32 leafHash, bytes32[] calldata proof, uint256 proofOrder)
        internal
        pure
        returns (bool)
    {
        if (proof.length > MAXIMUM_PROOF_SIZE) {
            revert ProofSizeExceeded();
        }
        if (proof.length < MAXIMUM_PROOF_SIZE && (proofOrder >> proof.length) != 0) {
            return false;
        }

        bytes32 acc = leafHash;
        for (uint256 i = 0; i < proof.length; i++) {
            acc = hashPairs(acc, proof[i], (proofOrder >> i) & 1);
        }
        return root == acc;
    }

    function hashPairs(bytes32 x, bytes32 y, uint256 order) internal pure returns (bytes32 value) {
        // forge-lint: disable-next-item(inline-assembly)
        assembly ("memory-safe") {
            /* reviewed: ... */
            switch order
            case 0 {
                mstore(0x00, x)
                mstore(0x20, y)
            }
            default {
                mstore(0x00, y)
                mstore(0x20, x)
            }
            value := keccak256(0x0, 0x40)
        }
    }
}
