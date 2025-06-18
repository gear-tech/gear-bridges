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

    constructor(IRelayer relayer) {
        RELAYER = relayer;
    }

    mapping(bytes32 => bool) private _processedMessages;

    /**
     * @dev Unpack message from merkle tree and relay it to the receiver.
     *
     * @param blockNumber - Block number of block containing target merkle tree.
     * @param totalLeaves - Number of leaves in target merkle tree.
     * @param leafIndex - Index of leaf containing target message. See `binary_merkle_tree` for
     *  reference.
     * @param message - Target message.
     * @param proof - Merkle proof of inclusion of leaf #`leafIndex` into target merkle tree that
     *  was included into `blockNumber`.
     */
    function processMessage(
        uint256 blockNumber,
        uint256 totalLeaves,
        uint256 leafIndex,
        VaraMessage calldata message,
        bytes32[] calldata proof
    ) public {
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

        if (!IMessageQueueReceiver(message.receiver).processVaraMessage(message.sender, message.data)) {
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
