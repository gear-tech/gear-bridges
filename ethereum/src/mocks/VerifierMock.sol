pragma solidity ^0.8.24;

import {IVerifier} from "../interfaces/IVerifier.sol";

contract Verifier is IVerifier {
    function verifyProof(
        bytes calldata,
        uint256[] calldata
    ) external pure returns (bool) {
        return true;
    }
}
