pragma solidity ^0.8.24;

struct WithdrawMessage {
    address receiver;
    address token;
    uint128 amount;
}

interface IERC20Treasury {
    error NotAuthorized();
    error BadArguments();
    error BadEthAddress();
    error BadVaraAddress();

    event Deposit(
        address indexed from,
        bytes32 indexed to,
        address indexed token,
        uint256 amount
    );
    event Withdraw(address indexed to, address indexed token, uint256 amount);
}

library Packer {
    function pack(
        WithdrawMessage calldata message
    ) external pure returns (bytes memory) {
        return
            abi.encodePacked(message.receiver, message.token, message.amount);
    }
}
