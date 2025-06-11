// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IRelayer} from "./interfaces/IRelayer.sol";
import {IVerifier} from "./interfaces/IVerifier.sol";

contract Relayer is IRelayer {
    mapping(uint256 blockNumber => bytes32 merkleRoot) private _blockNumbers;
    mapping(bytes32 merkleRoot => uint256 blockNumber) private _merkleRoots;
    bool _emergencyStop;

    IVerifier immutable VERIFIER_ADDRESS;

    constructor(IVerifier verifier) {
        VERIFIER_ADDRESS = verifier;
    }

    /**
     * @dev Verifies and stores a `merkleRoot` for specified `blockNumber`. Calls `verifyProof`
     *      in `PlonkVerifier` and reverts if the proof or the public inputs are malformed.
     *
     * @param blockNumber Block number where merkle root was relayed.
     * @param merkleRoot Merkle root containing messages queued to relay on VARA.
     * @param proof serialised plonk proof (using gnark's MarshalSolidity).
     */
    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external {
        if (_emergencyStop) {
            // Emergency stop is active, stop processing.
            revert EmergencyStop();
        }

        uint256[] memory publicInputs = new uint256[](2);
        publicInputs[0] = uint256(merkleRoot) >> 64;
        publicInputs[1] = ((uint256(merkleRoot) & uint256(type(uint64).max)) << 128)
            | ((blockNumber & uint256(type(uint32).max)) << 96);

        if (!VERIFIER_ADDRESS.verifyProof(proof, publicInputs)) {
            revert InvalidProof();
        }

        // Check if the provided Merkle root is a duplicate.
        // If it is a duplicate, set the emergency stop.
        bytes32 originalMerkleRoot = _blockNumbers[blockNumber];
        if (originalMerkleRoot != 0 && originalMerkleRoot != merkleRoot) {
            _emergencyStop = true;
            return;
        }

        _blockNumbers[blockNumber] = merkleRoot;
        _merkleRoots[merkleRoot] = blockNumber;

        emit MerkleRoot(blockNumber, merkleRoot);
    }

    /**
     * @dev Returns emergency stop status.
     */
    function emergencyStop() external view override returns (bool) {
        return _emergencyStop;
    }

    /**
     * @dev Returns merkle root for specified block number. Returns bytes32(0) if merkle root was
     *      not provided for specified blockNumber.
     *
     * @param blockNumber Target block number.
     * @return merkleRoot, bytes32(0) if no merkle root was found.
     */
    function getMerkleRoot(uint256 blockNumber) external view returns (bytes32) {
        if (_emergencyStop) {
            // Emergency stop is active, stop processing.
            revert EmergencyStop();
        }
        return _blockNumbers[blockNumber];
    }

    /**
     * @dev Returns block number for provided merkleRoot. Returns uint256(0) if merkle root was not
     *      provided for specified blockNumber.
     *
     * @param merkleRoot merkle root
     * @return blockNumber, uint256(0) if no block number was found.
     */
    function getBlockNumber(bytes32 merkleRoot) external view returns (uint256) {
        return _merkleRoots[merkleRoot];
    }
}
