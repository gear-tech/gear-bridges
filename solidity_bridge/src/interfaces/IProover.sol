pragma solidity ^0.8.24;


interface IProover {
    function verifyProof(bytes calldata message, bytes calldata proof) external returns(bool);
}