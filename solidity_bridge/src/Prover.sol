pragma solidity ^0.8.24;

import {IProver} from "./interfaces/IProver.sol";
import {PlonkVerifier} from "./libraries/PlonkVerifier.sol";


contract Prover is IProver, PlonkVerifier {
    function verifyProof(bytes calldata proof, uint256[] calldata public_inputs) external view returns(bool) {
        return Verify(proof, public_inputs);
    }

}