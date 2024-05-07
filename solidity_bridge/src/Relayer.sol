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

    /**
     * @dev Initialize relayer instance with verifier address
     *
     * @param prover - address of `Verifier` contract
     */
    function initialize(address prover) external {
        if (address(_prover) != address(0)) revert AlreadyInitialized();
        _prover = IProver(prover);
    }

    /**  @dev Verifies and stores a `merkle_root` for specified `block_number`. Calls `verifyProof`
     * in `PlonkVerifier` and reverts if the proof or the public inputs are malformed.
     *
     * @param block_number Block number where merkle root was relayed.
     * @param merkle_root Merkle root containing messages queued to relay on VARA.
     * @param proof serialised plonk proof (using gnark's MarshalSolidity).
     */
    function submitMerkleRoot(
        uint256 block_number,
        bytes32 merkle_root,
        bytes calldata proof
    ) external {
        uint256[] memory public_inputs = _buildPublicInputs(
            block_number,
            merkle_root
        );
        if (!_prover.verifyProof(proof, public_inputs)) {
            revert InvalidProof();
        }

        _block_numbers[block_number] = merkle_root;
        _merkle_roots[merkle_root] = block_number;

        emit MerkleRoot(block_number, bytes32(merkle_root));
    }

    /**
     * @dev Returns merkle root for specified block number. Returns bytes32(0) if merkle root was
     * not provided for specified block_number.
     *
     * @param block_number Target block number.
     * @return merkle_root, bytes32(0) if no merkle root was found.
     */
    function getMerkleRoot(
        uint256 block_number
    ) external view returns (bytes32) {
        return _block_numbers[block_number];
    }

    /**
     * @dev Returns block number for provided merkle_root. Returns uint256(0) if merkle root was not
     * provided for specified block_number
     *
     * @param merkle_root merkle root
     * @return block_number, uint256(0) if no block number was found.
     */
    function getBlockNumber(
        bytes32 merkle_root
    ) external view returns (uint256) {
        return _merkle_roots[merkle_root];
    }

    /**
     * @dev Calculates block number from provided public inputs
     *
     * @param public_inputs - proof's public inputs
     * @return block_number returns block number
     */
    function getBlockNumberFromPublicInputs(
        uint256[] calldata public_inputs
    ) public pure returns (uint256 block_number) {
        uint256 ret = uint256(public_inputs[1] >> 96) & MASK_32BITS;
        return ret;
    }

    /**
     * @dev Calculates merkle root from provided public inputs
     *
     * @param public_inputs - proof's public inputs
     * @return merkle_root - merkle root of target block
     */
    function getMerkleRootFromPublicInputs(
        uint256[] calldata public_inputs
    ) public pure returns (bytes32 merkle_root) {
        uint256 ret = ((public_inputs[0] & MASK_192BITS) << 64) |
            ((public_inputs[1] >> 128) & MASK_64BITS);
        return bytes32(ret);
    }

    /**
     * @dev Constructs public inputs for verifier from provided `block_number` and `merkle_root`.
     *
     * @param block_number Target block number.
     * @param merkle_root Target merkle root.
     * @return public_inputs Constructed public inputs.
     */
    function buildPublicInputs(
        uint256 block_number,
        bytes32 merkle_root
    ) public pure returns (uint256[] memory public_inputs) {
        return _buildPublicInputs(block_number, merkle_root);
    }

    /**
     * @dev Constructs public inputs for verifier from provided `block_number` and `merkle_root`.
     *
     * @param block_number Target block number.
     * @param merkle_root Target merkle root.
     * @return public_inputs Constructed public inputs.
     */
    function _buildPublicInputs(
        uint256 block_number,
        bytes32 merkle_root
    ) public pure returns (uint256[] memory public_inputs) {
        uint256[] memory ret = new uint256[](2);
        ret[0] = (uint256(merkle_root) >> 64) & MASK_192BITS;
        ret[1] =
            ((uint256(block_number) & MASK_32BITS) << 96) |
            ((uint256(merkle_root) & MASK_64BITS) << 128);

        return ret;
    }
}
