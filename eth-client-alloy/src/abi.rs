use alloy_sol_types::{sol, SolCall, SolInterface};
//use alloy_transport::Transport;

sol!{
    struct VaraMessage {
        uint256 block_number;
        ContentMessage content;
        bytes proof;
    }

    struct ContentMessage  {
        address eth_address;
        bytes32 vara_address;
        uint256 nonce;
        bytes data;
    }

    #[sol(rpc)]
    interface IMessageQueue {
        function process_message(VaraMessage calldata message ) external;
    }


    #[sol(rpc)]
    interface IRelayer {

        function get_merkle_root(uint256 blockNumber) external view returns(bytes32);
        function get_block_number(bytes32 merkleRoot) external view returns(uint256);
        function submit_merkle_root(uint256[] calldata public_inputs, bytes calldata proof ) external;
    }
}






