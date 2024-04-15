pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IProver} from "./interfaces/IProver.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";

import {Constants} from "./libraries/Constants.sol";
import {VaraMessage, ContentMessage, IMessageQueue, Hasher} from "./interfaces/IMessageQueue.sol";
import {MerkleProof} from "openzeppelin-contracts/contracts/utils/cryptography/MerkleProof.sol";


contract MessageQueue is IMessageQueue, AccessControl {
    using Address for address;
    IProver private _prover;
    IRelayer private _relayer;
    using Hasher for ContentMessage;

    mapping(bytes32 => bool) private _processed_messages;

    constructor() {
    }


    function initialize(address prover, address relayer) public {
        if (getRoleAdmin(Constants.ADMIN_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.ADMIN_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.ADMIN_ROLE, msg.sender);
        _prover = IProver(prover);
        emit ProoverAddressUpdated(prover);

        _relayer = IRelayer(relayer);
        emit RelayerAddressUpdated(relayer);
    }

    function setProover(address prover) public onlyRole(Constants.ADMIN_ROLE) {
        _prover = IProver(prover);
        emit ProoverAddressUpdated(prover);
    }

    function setRelayer(address relayer) public onlyRole(Constants.ADMIN_ROLE) {
        _relayer = IRelayer(relayer);
        emit RelayerAddressUpdated(relayer);
    }


    function calc_merkle_root(
        bytes32[] calldata proof,
        bytes32 leaf,
        uint256 width,
        uint256 index
    ) public pure returns (bytes32) {
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

    function calculate_root(bytes32[] calldata proof, bytes32 leaf_hash, uint256 width, uint256 leaf_index) public view returns (bytes32) {
        return calc_merkle_root(proof, leaf_hash, width, leaf_index);
    }

    function process_message(uint256 block_number, ContentMessage calldata message, bytes32[] calldata proof, uint256 width, uint256 leaf_index) public {
        bytes32 msg_hash = message.hash();

        if (_processed_messages[msg_hash]) revert MessageAlreadyProcessed(msg_hash);

        bytes32 merkle_root = _relayer.get_merkle_root(block_number);

        if (merkle_root == bytes32(0)) revert MerkleRootNotSet(block_number);

        if (calc_merkle_root(proof, msg_hash, width, leaf_index) != merkle_root) revert BadProof();

        _processed_messages[msg_hash] = true;

        message.eth_address.functionCall(message.data);

        emit MessageProcessed(block_number, msg_hash);
    }

    function is_processed(VaraMessage calldata message) external view returns (bool) {
        bytes32 msg_hash = message.content.hash();
        return _processed_messages[msg_hash];
    }


}