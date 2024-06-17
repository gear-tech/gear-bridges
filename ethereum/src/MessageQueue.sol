pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IRelayer} from "./interfaces/IRelayer.sol";

import {VaraMessage, VaraMessage, IMessageQueue, IMessageQueueReceiver, Hasher} from "./interfaces/IMessageQueue.sol";
import {MerkleProof} from "openzeppelin-contracts/contracts/utils/cryptography/MerkleProof.sol";
import {RELAYER_ADDRESS} from "./libraries/Environment.sol";

contract MessageQueue is IMessageQueue {
    using Address for address;
    using Hasher for VaraMessage;

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
        if (_processed_messages[message.nonce])
            revert MessageAlreadyProcessed(message.nonce);

        bytes32 msg_hash = message.hash();
        
        bytes32 merkle_root = IRelayer(RELAYER_ADDRESS).getMerkleRoot(
            block_number
        );

        if (merkle_root == bytes32(0)) revert MerkleRootNotSet(block_number);

        if (
            _calculateMerkleRoot(
                proof,
                msg_hash,
                total_leaves,
                leaf_index
            ) != merkle_root
        ) revert BadProof();

        _processed_messages[message.nonce] = true;

        if (
            !IMessageQueueReceiver(message.receiver).processVaraMessage(message)
        ) {
            revert MessageNotProcessed();
        } else {
            emit MessageProcessed(block_number, msg_hash, message.nonce);
        }
    }

    /**
     * @dev Calculated merkle tree root for a provided merkle proof.
     *
     * @param proof - Merkle proof.
     * @param leaf_hash - Hash of data stored in target leaf.
     * @param total_leaves - Number of leaves in merkle tree.
     * @param leaf_index - Index of target leaf.
     */
    function calculateMerkleRoot(
        bytes32[] calldata proof,
        bytes32 leaf_hash,
        uint256 total_leaves,
        uint256 leaf_index
    ) public pure returns (bytes32) {
        return _calculateMerkleRoot(proof, leaf_hash, total_leaves, leaf_index);
    }

    /**
     * @dev Checks if `VaraMessage` already was processed.
     *
     * @param message - Message it checks agaiunst.
     */

    function isProcessed(
        VaraMessage calldata message
    ) external view returns (bool) {
        return _processed_messages[message.nonce];
    }

    function _calculateMerkleRoot(
        bytes32[] calldata proof,
        bytes32 leaf,
        uint256 width,
        uint256 index
    ) internal pure returns (bytes32) {
        bytes32 hash = leaf;

        // TODO: Add check that index < width

        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 proofElement = proof[i];

            if ((index % 2 == 1) || (index + 1 == width)) {
                assembly {
                    mstore(0x00, proofElement)
                    mstore(0x20, hash)
                    hash := keccak256(0x00, 0x40)
                }
            } else {
                assembly {
                    mstore(0x00, hash)
                    mstore(0x20, proofElement)
                    hash := keccak256(0x00, 0x40)
                }
            }

            index = index / 2;
            width = ((width - 1) / 2) + 1;
        }

        return hash;
    }
}
