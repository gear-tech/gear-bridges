// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

struct VaraMessage {
    bytes32 nonce;
    bytes32 sender;
    address receiver;
    bytes data;
}

interface IMessageQueue {
    error RelayerEmergencyStop();
    error MessageAlreadyProcessed(bytes32 messageNonce);
    error MessageNotProcessed();
    error MerkleRootNotSet(uint256 blockNumber);
    error BadProof();

    event MessageProcessed(
        uint256 indexed blockNumber,
        bytes32 indexed messageHash,
        bytes32 indexed messageNonce
    );

    function processMessage(
        uint256 block_number,
        uint256 total_leaves,
        uint256 leaf_index,
        VaraMessage calldata message,
        bytes32[] calldata proof
    ) external;
}

interface IMessageQueueReceiver {
    function processVaraMessage(
        bytes32 sender,
        bytes calldata payload
    ) external returns (bool);
}

library Hasher {
    function hashCalldata(VaraMessage calldata message) internal pure returns (bytes32) {
        bytes32 hash1 = keccak256(abi.encodePacked(
            message.nonce,
            message.sender,
            message.receiver,
            message.data
        ));

        bytes32 hash2;
        assembly ("memory-safe") {
            mstore(0x00, hash1)
            hash2 := keccak256(0x00, 0x20)
        }

        return hash2;
    }

    function hash(VaraMessage memory message) internal pure returns (bytes32) {
        bytes32 hash1 = keccak256(abi.encodePacked(
            message.nonce,
            message.sender,
            message.receiver,
            message.data
        ));

        bytes32 hash2;
        assembly ("memory-safe") {
            mstore(0x00, hash1)
            hash2 := keccak256(0x00, 0x20)
        }

        return hash2;
    }
}
