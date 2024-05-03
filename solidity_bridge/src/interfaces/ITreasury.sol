pragma solidity ^0.8.24;

struct WithdrawMessage {
    address receiver;
    address token;
    uint128 amount;
}

interface ITreasury {
    error AlreadyInitialized();
    error BadArguments();
    error BadEthAddress();
    error BadVaraAddress();

    event Deposit(address indexed token, address indexed from, uint256 amount);
    event Withdraw(address indexed token, address indexed to, uint256 amount);
}

library Packer {
    function pack(WithdrawMessage calldata message) external pure returns (bytes memory) {
        return abi.encodePacked(message.receiver, message.token, message.amount);
    }
}