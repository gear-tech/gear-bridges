pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {IVerifier} from "./interfaces/IVerifier.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";

contract Relayer is IRelayer {
    IVerifier private _verifier;
    mapping(uint256 => bytes32) private _block_numbers;
    mapping(bytes32 => uint256) private _merkle_roots;
    bool _emergencyStop = false;

    uint256 private constant MASK_32BITS = (2 ** 32) - 1;
    uint256 private constant MASK_64BITS = (2 ** 64) - 1;
    uint256 private constant MASK_192BITS = (2 ** 192) - 1;

    address immutable VERIFIER_ADDRESS;

    constructor(address verifier) {
        VERIFIER_ADDRESS = verifier;
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
        if (_emergencyStop) {
            // Emergency stop is active, stop processing.
            revert EmergencyStop();
        }

        uint256[] memory public_inputs = _buildPublicInputs(
            block_number,
            merkle_root
        );
        if (!IVerifier(VERIFIER_ADDRESS).verifyProof(proof, public_inputs)) {
            revert InvalidProof();
        }

        // Check if the provided Merkle root is a duplicate.
        // If it is a duplicate, set the emergency stop.
        bytes32 orig_merkle_root = _block_numbers[block_number];
        _emergencyStop = (orig_merkle_root != 0 &&
            orig_merkle_root != merkle_root);
        if (_emergencyStop) {
            return;
        }

        _block_numbers[block_number] = merkle_root;
        _merkle_roots[merkle_root] = block_number;

        emit MerkleRoot(block_number, bytes32(merkle_root));
    }

    /**
     * @dev Returns emergency stop status.
     */
    function emergencyStop() external view override returns (bool) {
        return _emergencyStop;
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
        if (_emergencyStop) {
            // Emergency stop is active, stop processing.
            revert EmergencyStop();
        }
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
