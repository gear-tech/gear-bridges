// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageQueueReceiver} from "./IMessageQueueReceiver.sol";

/**
 * @dev Interface for the ERC20Manager contract.
 */
interface IERC20Manager is IMessageQueueReceiver {
    error NotAuthorized();
    error BadArguments();
    error BadSender();

    event BridgingRequested(address indexed from, bytes32 indexed to, address indexed token, uint256 amount);

    event BridgingAccepted(address indexed to, address indexed token, uint256 amount, bytes32 gearTokensSender);

    enum SupplyType {
        Unknown,
        Ethereum,
        Gear
    }

    function requestBridging(address token, uint256 amount, bytes32 to) external;

    function requestBridgingPayingFee(address token, uint256 amount, bytes32 to, address bridgingPayment)
        external
        payable;

    function requestBridgingPayingFeeWithPermit(
        address token,
        uint256 amount,
        bytes32 to,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s,
        address bridgingPayment
    ) external payable;

    function getTokenSupplyType(address token) external view returns (SupplyType);
}

/**
 * @dev Type representing payload of the message that `ERC20Manager` will accept.
 *      Also see `gear-programs/vft-manager/app/src/services/request_bridging/bridge_builtin_operations.rs`.
 */
struct WithdrawMessage {
    address receiver;
    address token;
    uint256 amount;
    bytes32 gearTokensSender;
}

/**
 * @dev Library for packing `WithdrawMessage` into a binary format.
 */
library Packer {
    /**
     * @dev Packs `WithdrawMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(WithdrawMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(message.receiver, message.token, message.amount, message.gearTokensSender);
    }
}
