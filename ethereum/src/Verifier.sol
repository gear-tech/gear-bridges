pragma solidity ^0.8.24;

import {IVerifier} from "./interfaces/IVerifier.sol";
import {PlonkVerifier} from "./libraries/PlonkVerifier.sol";

contract Verifier is IVerifier, PlonkVerifier {
    /** @dev Verify a proof. Calls `verifyProof` in `PlonkVerifier` and reverts if the proof or the
     * public inputs are malformed.
     *
     * @param proof Serialised plonk proof (using gnark's `MarshalSolidity`).
     * @param public_inputs Reduced public inputs.
     * @return success If proof is valid.
     */
    function verifyProof(
        bytes calldata proof,
        uint256[] calldata public_inputs
    ) external view returns (bool) {
        return Verify(proof, public_inputs);
    }
}
