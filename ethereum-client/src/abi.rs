use alloy_sol_types::{sol, SolCall, SolInterface};
use std::io::Read;

sol! {

    #[derive(Debug,PartialEq, Eq)]
    struct ContentMessage  {
        bytes32 sender;
        address receiver;
        uint256 nonce;
        bytes data;
    }

    #[sol(rpc)]
    #[derive(Debug,PartialEq, Eq)]
    interface IMessageQueue {

        event MessageProcessed(uint256 indexed blockNumber, bytes32 indexed messageHash);

        function calculateMerkleRoot(bytes32[] calldata proof, bytes32 hash, uint256 width, uint256 leaf_index) external view returns (bytes32);

        function processMessage(uint256 block_number, uint256 total_leaves, uint256 leaf_index, ContentMessage calldata message, bytes32[] calldata proof) external;

    }


    #[sol(rpc)]
    interface IRelayer {
        event MerkleRoot(uint256 indexed blockNumber, bytes32 indexed merkleRoot);

        function submitMerkleRoot(uint256 block_number, bytes32 merkle_root, bytes calldata proof) external;

        function getMerkleRoot(uint256 block_number) external view returns (bytes32);

        function getBlockNumber(bytes32 merkle_root) external view returns (uint256);
    }
}

impl ContentMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret: Vec<u8> = Vec::with_capacity(96 + self.data.len());
        ret.extend(self.sender.to_vec());
        ret.extend(self.receiver.to_vec());
        ret.extend(self.nonce.to_be_bytes::<32>());
        ret.extend(self.data.to_vec());
        ret
    }
}
