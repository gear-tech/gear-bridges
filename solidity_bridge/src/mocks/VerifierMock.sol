pragma solidity ^0.8.24;

import {IVerifier} from "../interfaces/IVerifier.sol";

contract Verifier is IVerifier {
    function verifyProof(
        bytes calldata proof,
        uint256[] calldata public_inputs
    ) external pure returns (bool) {
        return true;
    }
}
