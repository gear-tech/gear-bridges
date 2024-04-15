pragma solidity ^0.8.24;

interface IRelayer {
    error AlreadyInitialized();
    error InvalidProof();
    error BadInput();

    event MerkleRoot(uint256 indexed blockNumber, bytes32 indexed merkleRoot);

    function get_merkle_root(uint256 blockNumber) external view returns (bytes32);

    function get_block_number(bytes32 merkleRoot) external view returns (uint256);

    function build_public_inputs(bytes32 merkle_root, uint256 block_id) external pure returns (uint256[] memory public_inputs);

    function submit_merkle_root(uint256[] calldata public_inputs, bytes calldata proof) external;


}