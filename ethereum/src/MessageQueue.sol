// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IRelayer} from "./interfaces/IRelayer.sol";
import {VaraMessage, IMessageQueue, IMessageQueueReceiver} from "./interfaces/IMessageQueue.sol";
import {BinaryMerkleTree} from "./libraries/BinaryMerkleTree.sol";

contract MessageQueue is IMessageQueue {
    IRelayer immutable RELAYER;

    constructor(IRelayer relayer) {
        RELAYER = relayer;
    }

    function hash_vara_msg(
        VaraMessage calldata message
    ) internal pure returns (bytes32) {
        bytes memory data = abi.encodePacked(
            message.nonce,
            message.sender,
            message.receiver,
            message.data
        );
        return keccak256(abi.encodePacked(keccak256(data)));
    }

    mapping(bytes32 => bool) private _processed_messages;

    /**
     * @dev Unpack message from merkle tree and relay it to the receiver.
     *
     * @param block_number - Block number of block containing target merkle tree.
     * @param total_leaves - Number of leaves in target merkle tree.
     * @param leaf_index - Index of leaf containing target message. See `binary_merkle_tree` for
     *  reference.
     * @param message - Target message.
     * @param proof - Merkle proof of inclusion of leaf #`leaf_index` into target merkle tree that
     *  was included into `block_number`.
     */
    function processMessage(
        uint256 block_number,
        uint256 total_leaves,
        uint256 leaf_index,
        VaraMessage calldata message,
        bytes32[] calldata proof
    ) public {
        if (RELAYER.emergencyStop()) {
            revert RelayerEmergencyStop();
        }

        if (_processed_messages[message.nonce]) {
            revert MessageAlreadyProcessed(message.nonce);
        }

        bytes32 msg_hash = hash_vara_msg(message);

        bytes32 merkle_root = RELAYER.getMerkleRoot(block_number);

        if (merkle_root == bytes32(0)) {
            revert MerkleRootNotSet(block_number);
        }

        if (!BinaryMerkleTree.verifyProofCalldata(merkle_root, proof, total_leaves, leaf_index, msg_hash)) {
            revert BadProof();
        }

        _processed_messages[message.nonce] = true;

        if (
            !IMessageQueueReceiver(message.receiver).processVaraMessage(
                message.sender,
                message.data
            )
        ) {
            revert MessageNotProcessed();
        } else {
            emit MessageProcessed(block_number, msg_hash, message.nonce);
        }
    }

    /**
     * @dev Checks if `VaraMessage` already was processed.
     *
     * @param message - Message it checks agaiunst.
     */
    function isProcessed(VaraMessage calldata message) external view returns (bool) {
        return _processed_messages[message.nonce];
    }
}
