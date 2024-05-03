pragma solidity ^0.8.24;

import {IProver} from "./interfaces/IProver.sol";
import {PlonkVerifier} from "./libraries/PlonkVerifier.sol";


contract Prover is IProver, PlonkVerifier {
    /// Verify a Block Merkle Hash. Calls verifyProof in PlonkVerifier
    /// Reverts if the proof or the public inputs are malformed.
    /// @param proof serialised plonk proof (using gnark's MarshalSolidity)
    /// @param public_inputs (must be reduced)
    /// @return success true if the proof passes false otherwise
    function verifyProof(bytes calldata proof, uint256[] calldata public_inputs) external view returns (bool) {
        return Verify(proof, public_inputs);
    }

}