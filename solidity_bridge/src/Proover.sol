pragma solidity ^0.8.24;

import {IProover} from "./interfaces/IProover.sol";


contract Proover is IProover {
    function verifyProof(bytes calldata message, bytes calldata proof) public pure returns(bool) {
        return true;
    }

}