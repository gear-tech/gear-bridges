// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the Verifier contract.
 */
interface IVerifier {
    /**
     * @dev Verifies zk-SNARK Plonk proof, which lets us know that
     *      `blockNumber` and `merkleRoot` are on Vara Network.
     * @param proof Serialised Plonk proof (using gnark's `MarshalSolidity`).
     * @param publicInputs Reduced public inputs in the following format:
     *        ```solidity
     *        uint256 blockNumber = 0xcccccccc; // actually `uint32` (because Vara Network uses 32-bit block numbers)
     *        bytes32 merkleRoot = 0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbb;
     *
     *        uint256[] memory publicInputs = new uint256[](2);
     *        publicInputs[0] = uint256(merkleRoot) >> 64;
     *        publicInputs[1] = ((uint256(merkleRoot) & uint256(type(uint64).max)) << 128)
     *            | ((blockNumber & uint256(type(uint32).max)) << 96);
     *
     *        assert(publicInputs[0] == 0x0000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa);
     *        assert(publicInputs[1] == 0x0000000000000000bbbbbbbbbbbbbbbbcccccccc000000000000000000000000);
     *        ```
     * @return success `true` if proof is valid, `false` otherwise.
     */
    function verifyProof(bytes calldata proof, uint256[] calldata publicInputs) external view returns (bool success);
}
