// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Type representing message being bridged from Gear-based chain (Vara Network) to Ethereum.
 *      - https://github.com/gear-tech/gear/blob/v1.8.1/pallets/gear-eth-bridge/src/internal.rs#L58
 *
 * TODO: rename fields.
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
    /**
     * @dev Relayer emergency stop status is set.
     */
    error RelayerEmergencyStop();

    /**
     * @dev Message nonce is already processed.
     */
    error MessageAlreadyProcessed(bytes32 messageNonce);

    /**
     * @dev Message is not processed (failed to call IMessageQueueReceiver interface).
     */
    error MessageNotProcessed();

    /**
     * @dev Merkle root is not set for the block number in Relayer smart contract.
     */
    error MerkleRootNotSet(uint256 blockNumber);

    /**
     * @dev Merkle proof is invalid.
     */
    error InvalidMerkleProof();

    /**
     * @dev Emitted when message is processed.
     */
    event MessageProcessed(
        uint256 indexed blockNumber, bytes32 indexed messageHash, bytes32 indexed messageNonce, address messageReceiver
    );

    /**
     * @dev Verifies and processes message originated from Vara Network.
     *
     *      In this process, MessageQueue smart contract will calculate Merkle root
     *      for message and validate that it corresponds to Merkle root which is already stored
     *      in Relayer smart contract for same block number. If proof is correct, nonce of received
     *      message will be stored in smart contract and message will be forwarded to adequate message
     *      receiver, either ERC20Manager or ProxyUpdater smart contract.
     *
     *      Upon successful processing of the message MessageProcessed event is emited.
     *
     *      It is important to note that anyone can submit a message because all messages
     *      will be validated against previously stored Merkle roots in the Relayer smart contract.
     *
     * @param blockNumber Block number of block containing target merkle tree.
     * @param totalLeaves Number of leaves in target merkle tree.
     * @param leafIndex Index of leaf containing target message.
     * @param message Target message.
     * @param proof Merkle proof of inclusion of leaf #`leafIndex` into target merkle tree that
     *              was included into `blockNumber`.
     *
     * @dev Reverts if:
     *      - Relayer emergency stop status is set.
     *      - Message nonce is already processed.
     *      - Merkle root is not set for the block number in Relayer smart contract.
     *      - Merkle proof is invalid.
     *      - Message processing fails (failed to call IMessageQueueReceiver interface).
     */
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

/**
 * @dev Library for hashing VaraMessage.
 */
library Hasher {
    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     *
     * TODO: avoid double hashing.
     */
    function hashCalldata(VaraMessage calldata message) internal pure returns (bytes32) {
        bytes32 hash1 = keccak256(abi.encodePacked(message.nonce, message.sender, message.receiver, message.data));

        bytes32 hash2;
        assembly ("memory-safe") {
            mstore(0x00, hash1)
            hash2 := keccak256(0x00, 0x20)
        }

        return hash2;
    }

    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     *
     * TODO: avoid double hashing.
     */
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
