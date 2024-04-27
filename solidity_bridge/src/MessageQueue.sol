pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IProver} from "./interfaces/IProver.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";

import {Constants} from "./libraries/Constants.sol";
import {VaraMessage, VaraMessage, IMessageQueue, IMessageQueueReceiver, Hasher} from "./interfaces/IMessageQueue.sol";
import {MerkleProof} from "openzeppelin-contracts/contracts/utils/cryptography/MerkleProof.sol";


contract MessageQueue is IMessageQueue, AccessControl {
    using Address for address;
    using Hasher for VaraMessage;

    IRelayer private _relayer;
    mapping(bytes32 => bool) private _processed_messages;

    constructor() {
    }


    function initialize(address relayer) public {
        if (address(_relayer) != address(0)) revert AlreadyInitialized();
        _relayer = IRelayer(relayer);
        emit RelayerAddressUpdated(relayer);
    }


    function _calculateMerkleRoot(
        bytes32[] calldata proof,
        bytes32 leaf,
        uint256 width,
        uint256 index
    ) internal pure returns (bytes32) {
        bytes32 hash = leaf;

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


    function processMessage(uint256 block_number, uint256 total_leaves, uint256 leaf_index, VaraMessage calldata message, bytes32[] calldata proof) public {
        bytes32 msg_hash = message.hash();

        if (_processed_messages[msg_hash]) revert MessageAlreadyProcessed(msg_hash);

        bytes32 merkle_root = _relayer.getMerkleRoot(block_number);

        if (merkle_root == bytes32(0)) revert MerkleRootNotSet(block_number);

        if (_calculateMerkleRoot(proof, msg_hash, total_leaves, leaf_index) != merkle_root) revert BadProof();

        _processed_messages[msg_hash] = true;

        if (!IMessageQueueReceiver(message.eth_address).processVaraMessage(message)) {
            revert MessageNotProcessed();
        } else {
            emit MessageProcessed(block_number, msg_hash);
        }

    }

    function calculateMerkleRoot(bytes32[] calldata proof, bytes32 leaf_hash, uint256 width, uint256 leaf_index) public pure returns (bytes32) {
        return _calculateMerkleRoot(proof, leaf_hash, width, leaf_index);
    }

    function isProcessed(VaraMessage calldata message) external view returns (bool) {
        return _processed_messages[message.hash()];
    }


}