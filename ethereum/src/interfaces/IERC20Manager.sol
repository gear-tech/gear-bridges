// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

interface IERC20Manager {
    error NotAuthorized();
    error BadArguments();
    error BadVftManagerAddress();
    error UnsupportedTokenSupply();

    event BridgingRequested(address indexed from, bytes32 indexed to, address indexed token, uint256 amount);

    event BridgingAccepted(address indexed to, address indexed token, uint256 amount, bytes32 tokens_sender);

    enum SupplyType {
        Unknown,
        Ethereum,
        Gear
    }
}

struct WithdrawMessage {
    address receiver;
    address token;
    uint256 amount;
    bytes32 tokens_sender;
}

library Packer {
    function pack(WithdrawMessage calldata message) external pure returns (bytes memory) {
        return abi.encodePacked(message.receiver, message.token, message.amount, message.tokens_sender);
    }
}
