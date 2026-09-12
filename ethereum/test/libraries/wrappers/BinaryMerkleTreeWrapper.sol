// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {BinaryMerkleTree} from "src/libraries/BinaryMerkleTree.sol";

contract BinaryMerkleTreeWrapper {
    function verifyProofCalldata(
        bytes32 root,
        bytes32[] calldata proof,
        uint256 numberOfLeaves,
        uint256 leafIndex,
        bytes32 leafHash
    ) external pure returns (bool) {
        return BinaryMerkleTree.verifyProofCalldata(root, proof, numberOfLeaves, leafIndex, leafHash);
    }

    function verifyProof(
        bytes32 root,
        bytes32[] memory proof,
        uint256 numberOfLeaves,
        uint256 leafIndex,
        bytes32 leafHash
    ) external pure returns (bool) {
        return BinaryMerkleTree.verifyProof(root, proof, numberOfLeaves, leafIndex, leafHash);
    }
}
