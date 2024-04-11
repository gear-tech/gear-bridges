pragma solidity ^0.8.24;

    struct VaraMessage {
        uint256 block_number;
        ContentMessage content;
        bytes proof;
    }


    struct ContentMessage {
        address eth_address;
        bytes32 vara_address;
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


    function process_message(uint256 block_number, ContentMessage calldata message, bytes32[] calldata proof) external;
}

library Hasher {
    function hash(ContentMessage calldata message) external pure returns (bytes32) {
        bytes memory data = abi.encodePacked(message.eth_address, message.vara_address, message.nonce, message.data);
        return keccak256(data);
    }
}