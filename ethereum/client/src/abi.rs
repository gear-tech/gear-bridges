use alloy::sol;

sol!(
    #[sol(rpc)]
    IMessageQueue,
    "../../api/ethereum/MessageQueue.json"
);

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

impl IMessageQueue::VaraMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret: Vec<u8> = Vec::with_capacity(32 + 32 + 20 + self.data.len());
        ret.extend(self.nonce.to_vec());
        ret.extend(self.sender.to_vec());
        ret.extend(self.receiver.to_vec());
        ret.extend(self.data.to_vec());
        ret
    }
}
