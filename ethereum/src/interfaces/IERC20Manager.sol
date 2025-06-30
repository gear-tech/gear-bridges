// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageQueueProcessor} from "./IMessageQueueProcessor.sol";

/**
 * @dev Interface for the ERC20Manager contract.
 */
interface IERC20Manager is IMessageQueueProcessor {
    /**
     * @dev Error thrown when the sender is not the message queue.
     */
    error InvalidSender();

    /**
     * @dev Error thrown when the source is not the governance.
     */
    error InvalidSource();

    /**
     * @dev Error thrown when the payload is invalid.
     */
    error InvalidPayload();

    /**
     * @dev Error thrown when the supply type is invalid (`SupplyType.Unknown`).
     */
    error InvalidSupplyType();

    /**
     * @dev Event emitted when bridging request is made.
     */
    event BridgingRequested(address indexed from, bytes32 indexed to, address indexed token, uint256 amount);

    /**
     * @dev Event emitted when bridging request is accepted.
     */
    event BridgingAccepted(bytes32 indexed from, address indexed to, address indexed token, uint256 amount);

    /**
     * @dev Enum representing supply type of token.
     *
     *      - `Unknown` - token is not registered.
     *
     *      - `Ethereum` - token is located on Ethereum.
     *         Token supply is located on Ethereum.
     *
     *         This means that we're working with some pre-existing `ERC20` token on Ethereum and with
     *         wrapped `VFT` token on Gear.
     *
     *         When this type of token supply is activated corresponding tokens will be minted/burned
     *         on the gear side and locked/unlocked on the Ethereum side.
     *
     *         For example this type of token supply can be used to work with
     *         `USDT ERC20 token`/`wrappedUSDT VFT token` pair.
     *
     *      - `Gear` - token is located on Gear.
     *         Token supply is located on Gear.
     *
     *         This means that we're working with some pre-existing `VFT` token on Gear and with
     *         wrapped `ERC20` token on Ethereum.
     *
     *         When this type of token supply is activated corresponding tokens will be locked/unlocked
     *         on the gear side and minted/burned on the Gear side.
     *
     *         For example this type of token supply can be used to work with
     *         `VARA VFT token`/`wrappedVARA ERC20 token` pair.
     */
    enum SupplyType {
        Unknown,
        Ethereum,
        Gear
    }

    /**
     * @dev Struct representing information about token (address and supply type).
     */
    struct TokenWithSupplyType {
        address token;
        SupplyType supplyType;
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

    /**
     * @dev Returns supply type of token.
     * @param token Token address.
     * @return supplyType Supply type of token. Returns `SupplyType.Unknown` if token is not registered.
     */
    function getTokenSupplyType(address token) external view returns (SupplyType);
}

/**
 * @dev Type representing payload of the message that `ERC20Manager` will accept
 *      from `gear-programs/vft-manager` program.
 *      Also see `gear-programs/vft-manager/app/src/services/request_bridging/bridge_builtin_operations.rs`.
 */
struct WithdrawMessage {
    bytes32 sender;
    address receiver;
    address token;
    uint256 amount;
}

/**
 * @dev Library for packing `ERC20Manager` messages into a binary format.
 */
library ERC20ManagerPacker {
    /**
     * @dev Packs `WithdrawMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(WithdrawMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(message.sender, message.receiver, message.token, message.amount);
    }
}
