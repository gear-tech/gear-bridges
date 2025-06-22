// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the Relayer contract.
 */
interface IRelayer {
    /**
     * @dev Emergency stop status is active.
     */
    error EmergencyStop();

    /**
     * @dev The plonk proof is invalid.
     */
    error InvalidPlonkProof();

    /**
     * @dev Emitted when emergency stop status is set.
     */
    event EmergencyStopSet();

    /**
     * @dev Emitted when block number and merkle root are stored.
     */
    event MerkleRoot(uint256 indexed blockNumber, bytes32 indexed merkleRoot);

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
    function submitMerkleRoot(uint256 blockNumber, bytes32 merkleRoot, bytes calldata proof) external;

    /**
     * @dev Returns merkle root for specified block number.
     *      Returns `bytes32(0)` if merkle root was not provided for specified block number.
     * @param blockNumber Target block number.
     * @return merkleRoot Merkle root for specified block number.
     */
    function getMerkleRoot(uint256 blockNumber) external view returns (bytes32);

    /**
     * @dev Returns block number for provided merkle root.
     *      Returns `uint256(0)` if block number was not provided for specified merkle root.
     * @param merkleRoot Target merkle root.
     * @return blockNumber Block number for provided merkle root.
     */
    function getBlockNumber(bytes32 merkleRoot) external view returns (uint256);

    /**
     * @dev Returns emergency stop status.
     * @return emergencyStop emergency stop status.
     */
    function emergencyStop() external view returns (bool);
}
