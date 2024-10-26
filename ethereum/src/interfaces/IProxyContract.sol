pragma solidity ^0.8.24;

interface IProxyContract {
    function upgradeToAndCall(
        address newImplementation,
        bytes calldata data
    ) external;
}
