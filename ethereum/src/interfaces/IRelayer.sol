// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the Relayer contract.
 */
interface IRelayer {
    error InvalidProof();
    error EmergencyStop();

    event MerkleRoot(uint256 indexed blockNumber, bytes32 indexed merkleRoot);

    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external;

    function getMerkleRoot(uint256 blockNumber) external view returns (bytes32);

    function getBlockNumber(bytes32 merkleRoot) external view returns (uint256);

    function emergencyStop() external view returns (bool);
}
