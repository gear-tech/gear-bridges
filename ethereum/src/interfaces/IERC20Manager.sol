// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageHandler} from "./IMessageHandler.sol";
import {IPausable} from "./IPausable.sol";
import {LibString} from "src/libraries/LibString.sol";

/**
 * @dev Interface for the ERC20Manager contract.
 */
interface IERC20Manager is IPausable, IMessageHandler {
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
     * @dev Error thrown when the amount is invalid.
     */
    error InvalidAmount();

    /**
     * @dev Error thrown when the token type is invalid (`TokenType.Unknown`).
     */
    error InvalidTokenType();

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
    event Bridged(bytes32 indexed from, address indexed to, address indexed token, uint256 amount);

    /**
     * @dev Event emitted when bridging payment is created.
     */
    event BridgingPaymentCreated(address bridgingPayment);

    /**
     * @dev Event emitted when VFT manager is added to list of VFT managers.
     */
    event VftManagerAdded(bytes32 vftManager);

    /**
     * @dev Event emitted when Ethereum token is registered.
     */
    event EthereumTokenRegistered(address token);

    /**
     * @dev Event emitted when Gear token is registered.
     */
    event GearTokenRegistered(address token, string tokenName, string tokenSymbol, uint8 tokenDecimals);

    /**
     * @dev Enum representing token type.
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
    enum TokenType {
        Unknown,
        Ethereum,
        Gear
    }

    /**
     * @dev Struct representing information about token (address and token type).
     */
    struct TokenInfo {
        address token;
        TokenType tokenType;
    }

    /**
     * @dev Returns governance admin address.
     * @return governanceAdmin Governance admin address.
     */
    function governanceAdmin() external view returns (address);

    /**
     * @dev Returns governance pauser address.
     * @return governancePauser Governance pauser address.
     */
    function governancePauser() external view returns (address);

    /**
     * @dev Returns message queue address.
     * @return messageQueue Message queue address.
     */
    function messageQueue() external view returns (address);

    /**
     * @dev Returns total number of VFT managers.
     * @return totalVftManagers Total number of VFT managers.
     */
    function totalVftManagers() external view returns (uint256);

    /**
     * @dev Returns list of VFT managers.
     * @return vftManagers List of VFT managers.
     */
    function vftManagers() external view returns (bytes32[] memory);

    /**
     * @dev Returns list of VFT managers.
     * @param offset Offset of the first VFT manager to return.
     * @param limit Maximum number of VFT managers to return.
     * @return vftManagers List of VFT managers.
     */
    function vftManagers(uint256 offset, uint256 limit) external view returns (bytes32[] memory);

    /**
     * @dev Returns whether the VFT manager is registered.
     * @param vftManager VFT manager address.
     * @return isVftManager `true` if the VFT manager is registered, `false` otherwise.
     */
    function isVftManager(bytes32 vftManager) external view returns (bool);

    /**
     * @dev Returns total number of tokens.
     * @return totalTokens Total number of tokens.
     */
    function totalTokens() external view returns (uint256);

    /**
     * @dev Returns list of tokens.
     * @return tokens List of tokens.
     */
    function tokens() external view returns (address[] memory);

    /**
     * @dev Returns list of tokens.
     * @param offset Offset of the first token to return.
     * @param limit Maximum number of tokens to return.
     * @return tokens List of tokens.
     */
    function tokens(uint256 offset, uint256 limit) external view returns (address[] memory);

    /**
     * @dev Returns token type.
     * @param token Token address.
     * @return tokenType Token type. Returns `TokenType.Unknown` if token is not registered.
     */
    function getTokenType(address token) external view returns (TokenType);

    /**
     * @dev Returns total number of bridging payments.
     * @return totalBridgingPayments Total number of bridging payments.
     */
    function totalBridgingPayments() external view returns (uint256);

    /**
     * @dev Returns list of bridging payments.
     * @return bridgingPayments List of bridging payments.
     */
    function bridgingPayments() external view returns (address[] memory);

    /**
     * @dev Returns list of bridging payments.
     * @param offset Offset of the first bridging payment to return.
     * @param limit Maximum number of bridging payments to return.
     * @return bridgingPayments List of bridging payments.
     */
    function bridgingPayments(uint256 offset, uint256 limit) external view returns (address[] memory);

    /**
     * @dev Returns whether the bridging payment is registered.
     * @param bridgingPayment Bridging payment address.
     * @return isBridgingPayment `true` if the bridging payment is registered, `false` otherwise.
     */
    function isBridgingPayment(address bridgingPayment) external view returns (bool);

    /**
     * @dev Requests bridging of tokens.
     *      Emits `BridgingRequested` event.
     * @param token Token address.
     * @param amount Amount of tokens to bridge.
     * @param to Destination address.
     * @dev Reverts if token is not registered with `InvalidTokenType` error.
     */
    function requestBridging(address token, uint256 amount, bytes32 to) external;

    /**
     * @dev Requests bridging of tokens and pays fee to one of the `bridgingPayment` contracts.
     * @param token Token address.
     * @param amount Amount of tokens to bridge.
     * @param to Destination address.
     * @param bridgingPayment Bridging payment address.
     */
    function requestBridgingPayingFee(address token, uint256 amount, bytes32 to, address bridgingPayment)
        external
        payable;

    /**
     * @dev Requests bridging of tokens and pays fee to one of the `bridgingPayment` contracts.
     *      This function uses `permit` to approve spending of tokens to optimize gas costs.
     *      (If token supports `permit` function).
     * @param token Token address.
     * @param amount Amount of tokens to bridge.
     * @param to Destination address.
     * @param deadline Deadline for the transaction to be executed.
     * @param v ECDSA signature parameter.
     * @param r ECDSA signature parameter.
     * @param s ECDSA signature parameter.
     * @param bridgingPayment Bridging payment address.
     */
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
     * @dev Creates a new `BridgingPayment` contract (ERC20Manager is factory).
     *      Emits `BridgingPaymentCreated` event.
     * @param fee Fee amount in wei.
     * @return bridgingPaymentAddress Address of the created `bridgingPayment` contract.
     */
    function createBridgingPayment(uint256 fee) external returns (address);
}

/**
 * @dev Type representing payload of the message that `ERC20Manager` will accept
 *      from `gear-programs/vft-manager` program.
 *      Also see `gear-programs/vft-manager/app/src/services/request_bridging/bridge_builtin_operations.rs`.
 */
struct TransferMessage {
    bytes32 sender;
    address receiver;
    address token;
    uint256 amount;
}

/**
 * @dev Type representing payload of the message that adds VFT manager to list of registered VFT managers.
 */
struct AddVftManagerMessage {
    bytes32 vftManager;
}

/**
 * @dev Type representing payload of the message that registers Ethereum token.
 */
struct RegisterEthereumTokenMessage {
    address token;
}

/**
 * @dev Type representing payload of the message that registers Gear token.
 */
struct RegisterGearTokenMessage {
    string tokenName;
    string tokenSymbol;
    uint8 tokenDecimals;
}

/**
 * @dev Library for packing `ERC20Manager` messages into a binary format.
 */
library ERC20ManagerPacker {
    /**
     * @dev Packs `TransferMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(TransferMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(message.sender, message.receiver, message.token, message.amount);
    }

    /**
     * @dev Packs `AddVftManagerMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(AddVftManagerMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(0x00), message.vftManager);
    }

    /**
     * @dev Packs `RegisterEthereumTokenMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(RegisterEthereumTokenMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(IERC20Manager.TokenType.Ethereum), message.token);
    }

    /**
     * @dev Packs `RegisterGearTokenMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(RegisterGearTokenMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(
            uint8(IERC20Manager.TokenType.Gear),
            LibString.packOne(message.tokenName),
            LibString.packOne(message.tokenSymbol),
            message.tokenDecimals
        );
    }
}
