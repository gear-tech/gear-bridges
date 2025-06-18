// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

struct VaraMessage {
    bytes32 nonce;
    bytes32 sender;
    address receiver;
    bytes data;
}

interface IMessageQueue {
    error RelayerEmergencyStop();
    error MessageAlreadyProcessed(bytes32 messageNonce);
    error MessageNotProcessed();
    error MerkleRootNotSet(uint256 blockNumber);
    error BadProof();

    event MessageProcessed(
        uint256 indexed blockNumber,
        bytes32 indexed messageHash,
        bytes32 indexed messageNonce
    );

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
        bytes32 sender,
        bytes calldata payload
    ) external returns (bool);
}

library Hasher {
    function hash(VaraMessage calldata message) public pure returns (bytes32) {
        bytes memory data = abi.encodePacked(
            message.nonce,
            message.sender,
            message.receiver,
            message.data
        );
        return keccak256(abi.encodePacked(keccak256(data)));
    }
}
