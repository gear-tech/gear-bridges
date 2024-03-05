pragma solidity ^0.8.24;

interface IRelayer {
    error AlreadyInitialized();
    error InvalidProof();
 

    function get_merkle_root(uint256 blockNumber) external view returns(bytes32);
    function get_block_number(bytes32 merkleRoot) external view returns(uint256);


}