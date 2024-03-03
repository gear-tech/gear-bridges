pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IProover} from "./interfaces/IProover.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";

import {Constants} from "./libraries/Constants.sol";
import {VaraMessage, ContentMessage, IMessageQueue, Hasher} from "./interfaces/IMessageQueue.sol";



contract MessageQueue is IMessageQueue,AccessControl {
    using Address for address;
    IProover private _proover;
    IRelayer private _relayer;
    using Hasher for VaraMessage;
    using Hasher for ContentMessage;

    mapping(uint256=>mapping(bytes32=>bool)) private _processed_messages;

    constructor() {
    }


    function initialize(address proover, address relayer) public {
        if(getRoleAdmin(Constants.ADMIN_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.ADMIN_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.ADMIN_ROLE, msg.sender );
        _proover = IProover(proover);
        emit ProoverAddressUpdated(proover);

        _relayer = IRelayer(relayer);
        emit RelayerAddressUpdated(relayer);
    }

    function setProover(address proover) public onlyRole(Constants.ADMIN_ROLE) {    
        _proover = IProover(proover);
        emit ProoverAddressUpdated(proover);
    }

    function setRelayer(address relayer) public onlyRole(Constants.ADMIN_ROLE) {    
        _relayer = IRelayer(relayer);
        emit RelayerAddressUpdated(relayer);
    }


    function process_message(VaraMessage calldata message) public {
        bytes32 msg_hash = message.content.hash();
        if( _processed_messages[message.block_number][msg_hash] ) revert MessageAlreadyProcessed(msg_hash);

        bytes32 merkle_root = _relayer.get_merkle_root(message.block_number);
        if(merkle_root == bytes32(0)) revert MerkleRootNotSet(message.block_number);

        bytes memory message_bytes = abi.encodePacked(merkle_root, msg_hash);
        if(!_proover.verifyProof( message_bytes, message.proof) ) revert BadProof();

        _processed_messages[message.block_number][ msg_hash ] = true;

        message.content.eth_address.functionCall(message.content.data);

        emit MessageProcessed(message.block_number, msg_hash);
    }


}