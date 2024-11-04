pragma solidity ^0.8.24;

interface IERC20Manager {
    error NotAuthorized();
    error BadArguments();
    error BadVftManagerAddress();
    error UnsupportedTokenSupply();

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
