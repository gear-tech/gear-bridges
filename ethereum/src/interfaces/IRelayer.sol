// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

interface IRelayer {
    error AlreadyInitialized();
    error InvalidProof();
    error BadInput();
    error EmergencyStop();

    event MerkleRoot(uint256 indexed blockNumber, bytes32 indexed merkleRoot);

    function submitMerkleRoot(
        uint256 block_number,
        bytes32 merkle_root,
        bytes calldata proof
    ) external;

    function getMerkleRoot(
        uint256 block_number
    ) external view returns (bytes32);

    function getBlockNumber(
        bytes32 merkle_root
    ) external view returns (uint256);

    function buildPublicInputs(
        uint256 block_number,
        bytes32 merkle_root
    ) external pure returns (uint256[] memory public_inputs);

    function emergencyStop() external view returns (bool);
}
