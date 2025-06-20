// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Message struct, same as in Vara Network:
 *      - https://github.com/gear-tech/gear/blob/v1.8.1/pallets/gear-eth-bridge/src/internal.rs#L58
 *
 * TODO: rename fields
 */
struct VaraMessage {
    bytes32 nonce;
    bytes32 sender;
    address receiver;
    bytes data;
}

/**
 * @dev Interface for the MessageQueue contract.
 */
interface IMessageQueue {
    error RelayerEmergencyStop();
    error MessageAlreadyProcessed(bytes32 messageNonce);
    error MessageNotProcessed();
    error MerkleRootNotSet(uint256 blockNumber);
    error InvalidMerkleProof();

    event MessageProcessed(
        uint256 indexed blockNumber, bytes32 indexed messageHash, bytes32 indexed messageNonce, address messageReceiver
    );

    function processMessage(
        uint256 blockNumber,
        uint256 totalLeaves,
        uint256 leafIndex,
        VaraMessage calldata message,
        bytes32[] calldata proof
    ) external;

    /**
     * @dev Checks if message was already processed.
     * @param messageNonce Message nonce to check.
     * @return isProcessed `true` if message was already processed, `false` otherwise.
     */
    function isProcessed(bytes32 messageNonce) external view returns (bool);
}

library Hasher {
    function hashCalldata(VaraMessage calldata message) internal pure returns (bytes32) {
        bytes32 hash1 = keccak256(abi.encodePacked(message.nonce, message.sender, message.receiver, message.data));

        bytes32 hash2;
        assembly ("memory-safe") {
            mstore(0x00, hash1)
            hash2 := keccak256(0x00, 0x20)
        }

        return hash2;
    }

    function hash(VaraMessage memory message) internal pure returns (bytes32) {
        bytes32 hash1 = keccak256(abi.encodePacked(message.nonce, message.sender, message.receiver, message.data));

        bytes32 hash2;
        assembly ("memory-safe") {
            mstore(0x00, hash1)
            hash2 := keccak256(0x00, 0x20)
        }

        return hash2;
    }
}
