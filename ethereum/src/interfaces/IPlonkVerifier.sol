// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the PlonkVerifier contract.
 */
interface IPlonkVerifier {
    /**
     * @dev Verify a Plonk proof.
     * @param proof Serialised plonk proof (using gnark's MarshalSolidity).
     * @param public_inputs (must be reduced).
     * @return success `true` if the proof passes, `false` otherwise.
     * @dev Reverts if the proof or the public inputs are malformed.
     */
    function Verify(bytes calldata proof, uint256[] calldata public_inputs) external view returns (bool success);
}
