// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Type representing message being bridged from Gear-based chain (Vara Network) to Ethereum.
 *      - https://github.com/gear-tech/gear/blob/v1.8.1/pallets/gear-eth-bridge/src/internal.rs#L58
 */
struct VaraMessage {
    uint256 nonce;
    bytes32 source;
    address destination;
    bytes payload;
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
    error MessageAlreadyProcessed(uint256 messageNonce);

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
        uint256 indexed blockNumber,
        bytes32 indexed messageHash,
        uint256 indexed messageNonce,
        address messageDestination
    );

    /**
     * @dev Verifies and processes message originated from Vara Network.
     *
     *      In this process, MessageQueue smart contract will calculate Merkle root
     *      for message and validate that it corresponds to Merkle root which is already stored
     *      in Relayer smart contract for same block number. If proof is correct, nonce of received
     *      message will be stored in smart contract and message will be forwarded to adequate message
     *      processor, either ERC20Manager or Governance smart contract.
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
     *      - MessageQueue is paused and message source is not any governance address.
     *      - Relayer emergency stop status is set.
     *      - Message nonce is already processed.
     *      - Merkle root is not set for the block number in Relayer smart contract.
     *      - Merkle proof is invalid.
     *      - Message processing fails.
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
    function isProcessed(uint256 messageNonce) external view returns (bool);
}

/**
 * @dev Library for hashing VaraMessage.
 */
library Hasher {
    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     */
    function hashCalldata(VaraMessage calldata message) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
    }

    /**
     * @dev Hashes VaraMessage.
     * @param message Message to hash.
     * @return hash Hash of the message.
     */
    function hash(VaraMessage memory message) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
    }
}
