// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IRelayer} from "./interfaces/IRelayer.sol";
import {IVerifier} from "./interfaces/IVerifier.sol";

/**
 * @dev Relayer smart contract is responsible for storing Merkle roots for blocks
 *      that were observed on Vara Network. Before storing Merkle roots, Relayer
 *      verifies received Merkle roots with help of Verifier smart contract.
 */
contract Relayer is IRelayer {
    mapping(uint256 blockNumber => bytes32 merkleRoot) private _blockNumbers;
    mapping(bytes32 merkleRoot => uint256 blockNumber) private _merkleRoots;
    bool private _emergencyStop;

    IVerifier immutable VERIFIER;

    /**
     * @dev Initializes the Relayer contract with the Verifier address.
     * @param verifier The address of the Verifier contract that will be used to verify Merkle roots.
     */
    constructor(IVerifier verifier) {
        VERIFIER = verifier;
    }

    /**
     * @dev Receives, verifies and stores Merkle roots from Vara Network.
     *
     *      Upon successfully storing data about block number and corresponding Merkle root,
     *      Relayer smart contract will emit a MerkleRoot event.
     *
     *      It is important to note that anyone can submit a Merkle root because only
     *      validated Merkle roots will be stored in the Relayer smart contract.
     *
     * @param blockNumber Block number on Vara Network
     * @param merkleRoot Merkle root of transactions included in block with corresponding block number
     * @param proof Serialised Plonk proof (using gnark's `MarshalSolidity`).
     * @dev Reverts if emergency stop status is set.
     * @dev Reverts if `proof` or `publicInputs` are malformed (depends on implementation of `IVerifier`).
     */
    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external {
        if (_emergencyStop) {
            revert EmergencyStop();
        }

        uint256[] memory publicInputs = new uint256[](2);
        publicInputs[0] = uint256(merkleRoot) >> 64;
        publicInputs[1] = ((uint256(merkleRoot) & uint256(type(uint64).max)) << 128)
            | ((blockNumber & uint256(type(uint32).max)) << 96);

        if (!VERIFIER.verifyProof(proof, publicInputs)) {
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
     * @dev Returns merkle root for specified block number.
     *      Returns `bytes32(0)` if merkle root was not provided for specified block number.
     * @param blockNumber Target block number.
     * @return merkleRoot Merkle root for specified block number.
     */
    function getMerkleRoot(uint256 blockNumber) external view returns (bytes32) {
        return _blockNumbers[blockNumber];
    }

    /**
     * @dev Returns block number for provided merkle root.
     *      Returns `uint256(0)` if block number was not provided for specified merkle root.
     * @param merkleRoot Target merkle root.
     * @return blockNumber Block number for provided merkle root.
     */
    function getBlockNumber(bytes32 merkleRoot) external view returns (uint256) {
        return _merkleRoots[merkleRoot];
    }

    /**
     * @dev Returns emergency stop status.
     * @return emergencyStop emergency stop status.
     */
    function emergencyStop() external view returns (bool) {
        return _emergencyStop;
    }
}
