// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IBridgingPayment} from "./IBridgingPayment.sol";
import {IMessageHandler} from "./IMessageHandler.sol";
import {LibString} from "src/libraries/LibString.sol";

/**
 * @dev Interface for the ERC20Manager contract.
 */
interface IERC20Manager is IMessageHandler {
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
     * @dev Error thrown when the bridging payment is invalid (not created by `ERC20Manager`).
     */
    error InvalidBridgingPayment();

    /**
     * @dev Event emitted when bridging request is made.
     */
    event BridgingRequested(address indexed from, bytes32 indexed to, address indexed token, uint256 amount);

    /**
     * @dev Event emitted when bridging request is accepted.
     */
    event BridgingAccepted(bytes32 indexed from, address indexed to, address indexed token, uint256 amount);

    /**
     * @dev Event emitted when bridging payment is created.
     */
    event BridgingPaymentCreated(address indexed bridgingPayment);

    /**
     * @dev Enum representing supply type of token.
     *
     *      - `Unknown` - token is not registered.
     *
     *      - `Ethereum` - token supply is located on Ethereum.
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
     *      - `Gear` - token supply is located on Gear.
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

    function createBridgingPayment(uint256 fee) external returns (address);

    /**
     * @dev Returns supply type of token.
     * @param token Token address.
     * @return supplyType Supply type of token. Returns `SupplyType.Unknown` if token is not registered.
     */
    function getTokenSupplyType(address token) external view returns (SupplyType);

    /**
     * @dev Returns whether the bridging payment is known.
     * @param bridgingPayment Bridging payment address.
     * @return isKnown `true` if the bridging payment is known, `false` otherwise.
     */
    function isKnownBridgingPayment(address bridgingPayment) external view returns (bool);
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
 * @dev Type representing payload of the message that registers Ethereum token.
 */
struct RegisterEthereumTokenMessage {
    string tokenName;
    string tokenSymbol;
    uint8 tokenDecimals;
}

/**
 * @dev Type representing payload of the message that registers Gear token.
 */
struct RegisterGearTokenMessage {
    address token;
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

    /**
     * @dev Packs `RegisterEthereumTokenMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(RegisterEthereumTokenMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(
            uint8(IERC20Manager.SupplyType.Ethereum),
            LibString.packOne(message.tokenName),
            LibString.packOne(message.tokenSymbol),
            message.tokenDecimals
        );
    }

    /**
     * @dev Packs `RegisterGearTokenMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(RegisterGearTokenMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(IERC20Manager.SupplyType.Gear), message.token);
    }
}
