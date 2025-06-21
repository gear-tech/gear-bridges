// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IRelayer} from "./interfaces/IRelayer.sol";
import {VaraMessage, IMessageQueue, Hasher} from "./interfaces/IMessageQueue.sol";
import {IMessageQueueReceiver} from "./interfaces/IMessageQueueReceiver.sol";
import {BinaryMerkleTree} from "./libraries/BinaryMerkleTree.sol";

/**
 * @dev MessageQueue smart contract is responsible for verifying and processing
 *      received messages originated from Vara Network.
 */
contract MessageQueue is IMessageQueue {
    using Hasher for VaraMessage;

    IRelayer immutable RELAYER;

    mapping(bytes32 messageNonce => bool isProcessed) private _processedMessages;

    /**
     * @dev Initializes the MessageQueue contract with the Relayer address.
     * @param relayer The address of the Relayer contract that will store merkle roots.
     */
    constructor(IRelayer relayer) {
        RELAYER = relayer;
    }

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
    ) external {
        if (RELAYER.emergencyStop()) {
            revert RelayerEmergencyStop();
        }

        if (_processedMessages[message.nonce]) {
            revert MessageAlreadyProcessed(message.nonce);
        }

        bytes32 merkleRoot = RELAYER.getMerkleRoot(blockNumber);
        if (merkleRoot == bytes32(0)) {
            revert MerkleRootNotSet(blockNumber);
        }

        bytes32 messageHash = message.hashCalldata();
        if (!BinaryMerkleTree.verifyProofCalldata(merkleRoot, proof, totalLeaves, leafIndex, messageHash)) {
            revert InvalidMerkleProof();
        }

        _processedMessages[message.nonce] = true;

        if (message.receiver.code.length == 0) {
            revert MessageNotProcessed();
        }

        (bool success,) = message.receiver.call(
            abi.encodeWithSelector(IMessageQueueReceiver.processVaraMessage.selector, message.sender, message.data)
        );
        if (!success) {
            revert MessageNotProcessed();
        }

        emit MessageProcessed(blockNumber, messageHash, message.nonce, message.receiver);
    }

    /**
     * @dev Checks if message was already processed.
     * @param messageNonce Message nonce to check.
     * @return isProcessed `true` if message was already processed, `false` otherwise.
     */
    function isProcessed(bytes32 messageNonce) external view returns (bool) {
        return _processedMessages[messageNonce];
    }
}
