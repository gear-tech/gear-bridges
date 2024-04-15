pragma solidity ^0.8.24;


    struct VaraMessage {
        bytes32 vara_address;
        address eth_address;
        uint256 nonce;
        bytes data;
    }

interface IMessageQueue {
    error AlreadyInitialized();
    error MessageAlreadyProcessed(bytes32 messageHash);
    error MerkleRootNotSet(uint256 blockNumber);
    error BadProof();

    event RelayerAddressUpdated(address relayer);
    event ProoverAddressUpdated(address proover);
    event MessageProcessed(uint256 indexed blockNumber, bytes32 indexed messageHash);

    function calculate_root(bytes32[] calldata proof, bytes32 hash, uint256 width, uint256 leaf_index) external view returns (bytes32);

    function process_message(uint256 block_number, uint256 total_leaves, uint256 leaf_index, VaraMessage calldata message, bytes32[] calldata proof) external;
}

library Hasher {
    function hash(VaraMessage calldata message) external pure returns (bytes32) {
        bytes memory data = abi.encodePacked(message.vara_address, message.eth_address, message.nonce, message.data);
        return keccak256(data);
    }
}