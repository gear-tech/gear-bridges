pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {IProover} from "./interfaces/IProover.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";
import {Constants} from "./libraries/Constants.sol";



contract Relayer is IRelayer, AccessControl {
    IProover private _proover;
    mapping(uint256=>bytes32) private _block_numbers;
    mapping(bytes32=>uint256) private _merkle_roots;



    function initialize(address proover) external {
        if(getRoleAdmin(Constants.ADMIN_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.ADMIN_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.ADMIN_ROLE, msg.sender );
        _proover = IProover(proover);
    }

    function add_merkle_root(uint256 blockNumber, bytes32 merkle_root, bytes calldata proof ) external {
        bytes memory message = abi.encodePacked(blockNumber, merkle_root);
        if(!_proover.verifyProof(message, proof)) {
            revert InvalidProof();
        }
        _block_numbers[blockNumber] = merkle_root;
        _merkle_roots[merkle_root] = blockNumber;
    }
    
    function get_merkle_root(uint256 blockNumber) external view returns(bytes32) {
        return _block_numbers[blockNumber];
    }

    function get_block_number(bytes32 merkleRoot) external view returns(uint256) {
        return _merkle_roots[merkleRoot];
    }



}