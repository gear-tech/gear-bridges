pragma solidity ^0.8.24;

struct BridgingRequest {
    address receiver;
    address token;
    uint128 amount;
}

interface IERC20Gateway {
    error NotAuthorized();
    error BadArguments();
    error BadEthAddress();
    error BadVaraAddress();

    event BridgingRequested(
        address indexed from,
        bytes32 indexed to,
        address indexed token,
        uint256 amount
    );
    event BridgingAccepted(
        address indexed to,
        address indexed token,
        uint256 amount
    );
}

library Packer {
    function pack(
        BridgingRequest calldata message
    ) external pure returns (bytes memory) {
        return
            abi.encodePacked(message.receiver, message.token, message.amount);
    }
}
