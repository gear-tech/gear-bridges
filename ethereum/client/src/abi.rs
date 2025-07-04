use alloy::sol;

sol!(
    #[sol(rpc)]
    IMessageQueue,
    "../../api/ethereum/MessageQueue.json"
);

sol!(
    #[sol(rpc)]
    #[allow(clippy::too_many_arguments)]
    IERC20Manager,
    "../../api/ethereum/IERC20Manager.json"
);

sol!(
    #[sol(rpc)]
    BridgingPayment,
    "../../api/ethereum/BridgingPayment.json"
);

impl IMessageQueue::VaraMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        // struct VaraMessage {
        //     uint256 nonce; // 32 bytes
        //     bytes32 source; // 32 bytes
        //     address destination; // 20 bytes
        //     bytes payload; // variable length
        // }
        let mut ret: Vec<u8> = Vec::with_capacity(32 + 32 + 20 + self.payload.len());
        ret.extend(self.nonce.to_be_bytes_vec());
        ret.extend(self.source.to_vec());
        ret.extend(self.destination.to_vec());
        ret.extend(self.payload.to_vec());
        ret
    }
}
