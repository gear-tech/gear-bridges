pragma solidity ^0.8.24;

import {IProver} from "../interfaces/IProver.sol";


contract Prover is IProver {
    function verifyProof(bytes calldata proof, uint256[] calldata public_inputs) external pure returns(bool) {
        return true;
    }

}