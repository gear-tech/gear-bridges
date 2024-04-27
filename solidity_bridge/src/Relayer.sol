pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {IProver} from "./interfaces/IProver.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";
import {Constants} from "./libraries/Constants.sol";


contract Relayer is IRelayer {
    IProver private _prover;
    mapping(uint256 => bytes32) private _block_numbers;
    mapping(bytes32 => uint256) private _merkle_roots;

    uint256 private constant MASK_32BITS = (2 ** 32) - 1;
    uint256 private constant MASK_64BITS = (2 ** 64) - 1;
    uint256 private constant MASK_192BITS = (2 ** 192) - 1;


    function initialize(address prover) external {
        if (address(_prover) != address(0)) revert AlreadyInitialized();
        _prover = IProver(prover);
    }


    function submitMerkleRoot(uint256[] calldata public_inputs, bytes calldata proof) external {
        if (!_prover.verifyProof(proof, public_inputs)) {
            revert InvalidProof();
        }

        bytes32 merkle_root = getMerkleRootFromInputs(public_inputs);

        uint256 block_number = getBlockIdFromInputs(public_inputs);

        _block_numbers[block_number] = merkle_root;
        _merkle_roots[merkle_root] = block_number;

        emit MerkleRoot(block_number, bytes32(merkle_root));

    }

    function getMerkleRoot(uint256 block_number) external view returns (bytes32) {
        return _block_numbers[block_number];
    }

    function getBlockNumber(bytes32 merkle_root) external view returns (uint256) {
        return _merkle_roots[merkle_root];
    }

    function getBlockIdFromInputs(uint256[] calldata public_inputs) public pure returns (uint256) {
        uint256 ret = uint256(public_inputs[1] >> 96) & MASK_32BITS;
        return ret;
    }


    function getMerkleRootFromInputs(uint256[] calldata public_inputs) public pure returns (bytes32) {
        uint256 ret = ((public_inputs[0] & MASK_192BITS) << 64) | ((public_inputs[1] >> 128) & MASK_64BITS);
        return bytes32(ret);
    }

    function buildPublicInputs(bytes32 merkle_root, uint256 block_id) public pure returns (uint256[] memory public_inputs) {
        uint256[] memory ret = new uint256[](2);
        ret[0] = (uint256(merkle_root) >> 64) & MASK_192BITS;
        ret[1] = ((uint256(block_id) & MASK_32BITS) << 96) | ((uint256(merkle_root) & MASK_64BITS) << 128);

        return ret;
    }


}