pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {IProver} from "./interfaces/IProver.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";
import {Constants} from "./libraries/Constants.sol";


contract Relayer is IRelayer, AccessControl {
    IProver private _prover;
    mapping(uint256 => bytes32) private _block_numbers;
    mapping(bytes32 => uint256) private _merkle_roots;

    uint256 private constant MASK_32BITS = (2 ** 32) - 1;
    uint256 private constant MASK_64BITS = (2 ** 64) - 1;
    uint256 private constant MASK_192BITS = (2 ** 192) - 1;


    function get_block_id_from_inputs(uint256[] calldata public_inputs) private pure returns (uint256) {
        uint256 ret = uint256(public_inputs[1] >> 96) & MASK_32BITS;
        return ret;
    }


    function get_merkle_root_from_inputs(uint256[] calldata public_inputs) private pure returns (bytes32) {
        uint256 ret = ((public_inputs[0] & MASK_192BITS) << 64) | ((public_inputs[1] >> 128) & MASK_64BITS);
        return bytes32(ret);
    }

    function initialize(address prover) external {
        if (getRoleAdmin(Constants.ADMIN_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.ADMIN_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.ADMIN_ROLE, msg.sender);
        _prover = IProver(prover);
    }


    function submit_merkle_root(uint256[] calldata public_inputs, bytes calldata proof) external {
        if (!_prover.verifyProof(proof, public_inputs)) {
            revert InvalidProof();
        }

        bytes32 merkle_root = get_merkle_root_from_inputs(public_inputs);

        uint256 block_number = get_block_id_from_inputs(public_inputs);

        _block_numbers[block_number] = merkle_root;
        _merkle_roots[merkle_root] = block_number;

        emit MerkleRoot(block_number, bytes32(merkle_root));

    }

    function get_merkle_root(uint256 block_number) external view returns (bytes32) {
        return _block_numbers[block_number];
    }

    function get_block_number(bytes32 merkle_root) external view returns (uint256) {
        return _merkle_roots[merkle_root];
    }


}