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
        event MessageProcessed(
            uint256 indexed blockNumber,
            bytes32 indexed messageHash,
            bytes32 indexed messageNonce
        );

        function calculateMerkleRoot(bytes32[] calldata proof, bytes32 hash, uint256 width, uint256 leaf_index) external view returns (bytes32);

        function processMessage(uint256 block_number, uint256 total_leaves, uint256 leaf_index, ContentMessage calldata message, bytes32[] calldata proof) external;

        function isProcessed(ContentMessage calldata message) external view returns (bool);
    }
}

sol!(
    #[sol(rpc)]
    IRelayer,
    "../../api/ethereum/IRelayer.json"
);

sol!(
    #[sol(rpc)]
    IERC20Manager,
    "../out/IERC20Manager.sol/IERC20Manager.json"
);

sol!(
    #[sol(rpc)]
    BridgingPayment,
    "../out/BridgingPayment.sol/BridgingPayment.json"
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
