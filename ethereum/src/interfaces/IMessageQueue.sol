pragma solidity ^0.8.24;

struct VaraMessage {
    uint256 nonce;
    bytes32 sender;
    address receiver;
    bytes data;
}

interface IMessageQueue {
    error AlreadyInitialized();
    error MessageAlreadyProcessed(uint256 messageNonce);
    error MessageNotProcessed();
    error MerkleRootNotSet(uint256 blockNumber);
    error BadProof();

    event RelayerAddressUpdated(address relayer);
    event ProoverAddressUpdated(address proover);

    event MessageProcessed(
        uint256 indexed blockNumber,
        bytes32 indexed messageHash,
        uint256 indexed messageNonce
    );

    function calculateMerkleRoot(
        bytes32[] calldata proof,
        bytes32 hash,
        uint256 width,
        uint256 leaf_index
    ) external view returns (bytes32);

    function processMessage(
        uint256 block_number,
        uint256 total_leaves,
        uint256 leaf_index,
        VaraMessage calldata message,
        bytes32[] calldata proof
    ) external;
}

interface IMessageQueueReceiver {
    function processVaraMessage(
        VaraMessage calldata vara_msg
    ) external returns (bool);
}

library Hasher {
    function hash(
        VaraMessage calldata message
    ) external pure returns (bytes32) {
        bytes memory data = abi.encodePacked(
            be_to_le(message.nonce),
            message.sender,
            message.receiver,
            message.data
        );
        return keccak256(data);
    }

    function be_to_le(uint256 input) internal pure returns (uint256 v) {
        v = input;
        v =
            ((v &
                0xFF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00) >>
                8) |
            ((v &
                0x00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF00FF) <<
                8);
        v =
            ((v &
                0xFFFF0000FFFF0000FFFF0000FFFF0000FFFF0000FFFF0000FFFF0000FFFF0000) >>
                16) |
            ((v &
                0x0000FFFF0000FFFF0000FFFF0000FFFF0000FFFF0000FFFF0000FFFF0000FFFF) <<
                16);
        v =
            ((v &
                0xFFFFFFFF00000000FFFFFFFF00000000FFFFFFFF00000000FFFFFFFF00000000) >>
                32) |
            ((v &
                0x00000000FFFFFFFF00000000FFFFFFFF00000000FFFFFFFF00000000FFFFFFFF) <<
                32);
        v =
            ((v &
                0xFFFFFFFFFFFFFFFF0000000000000000FFFFFFFFFFFFFFFF0000000000000000) >>
                64) |
            ((v &
                0x0000000000000000FFFFFFFFFFFFFFFF0000000000000000FFFFFFFFFFFFFFFF) <<
                64);

        v = (v >> 128) | (v << 128);
    }
}
