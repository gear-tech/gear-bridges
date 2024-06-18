use alloy::sol;

sol! {
    #[derive(Debug,PartialEq, Eq)]
    struct ContentMessage  {
        bytes32 nonce;
        bytes32 sender;
        address receiver;
        bytes data;
    }

    #[sol(rpc)]
    #[derive(Debug,PartialEq, Eq)]
    interface IMessageQueue {
        event MessageProcessed(uint256 indexed blockNumber, bytes32 indexed messageHash);

        function calculateMerkleRoot(bytes32[] calldata proof, bytes32 hash, uint256 width, uint256 leaf_index) external view returns (bytes32);

        function processMessage(uint256 block_number, uint256 total_leaves, uint256 leaf_index, ContentMessage calldata message, bytes32[] calldata proof) external;

        function isProcessed(ContentMessage calldata message) external view returns (bool);
    }

    #[sol(rpc)]
    interface IRelayer {
        event MerkleRoot(uint256 indexed blockNumber, bytes32 indexed merkleRoot);

        function submitMerkleRoot(uint256 block_number, bytes32 merkle_root, bytes calldata proof) external;

        function getMerkleRoot(uint256 block_number) external view returns (bytes32);

        function getBlockNumber(bytes32 merkle_root) external view returns (uint256);
    }




}

#[cfg(test)]
sol!(
    #[sol(rpc)]
    ProxyContract,
    "../out/ProxyContract.sol/ProxyContract.json"
);

#[cfg(test)]
sol!(
    #[sol(rpc)]
    ERC20Mock,
    "../out/ERC20Mock.sol/ERC20Mock.json"
);

#[cfg(test)]
sol!(
    #[sol(rpc)]
    Relayer,
    "../out/Relayer.sol/Relayer.json"
);

#[cfg(test)]
sol!(
    #[sol(rpc)]
    MessageQueue,
    "../out/MessageQueue.sol/MessageQueue.json"
);

#[cfg(test)]
sol!(
    #[sol(rpc)]
    Verifier,
    "../out/Verifier.sol/Verifier.json"
);

#[cfg(test)]
sol!(
    #[sol(rpc)]
    VerifierMock,
    "../out/VerifierMock.sol/Verifier.json"
);

#[cfg(test)]
sol!(
    #[sol(rpc)]
    ERC20Treasury,
    "../out/ERC20Treasury.sol/ERC20Treasury.json"
);

impl ContentMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret: Vec<u8> = Vec::with_capacity(32 + 32 + 20 + self.data.len());
        ret.extend(self.nonce.to_vec());
        ret.extend(self.sender.to_vec());
        ret.extend(self.receiver.to_vec());
        ret.extend(self.data.to_vec());
        ret
    }
}
