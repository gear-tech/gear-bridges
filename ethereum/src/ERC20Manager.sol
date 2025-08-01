// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {EnumerableSet} from "@openzeppelin/contracts/utils/structs/EnumerableSet.sol";
import {ERC20GearSupply} from "./erc20/managed/ERC20GearSupply.sol";
import {CustomEnumerableMap} from "./libraries/CustomEnumerableMap.sol";
import {LibString} from "./libraries/LibString.sol";
import {BridgingPayment} from "./BridgingPayment.sol";
import {IBridgingPayment} from "./interfaces/IBridgingPayment.sol";
import {IERC20Burnable} from "./interfaces/IERC20Burnable.sol";
import {IERC20Manager} from "./interfaces/IERC20Manager.sol";
import {IERC20Mintable} from "./interfaces/IERC20Mintable.sol";
import {IGovernance} from "./interfaces/IGovernance.sol";
import {IMessageHandler} from "./interfaces/IMessageHandler.sol";
import {IPausable} from "./interfaces/IPausable.sol";

contract ERC20Manager is
    Initializable,
    AccessControlUpgradeable,
    PausableUpgradeable,
    UUPSUpgradeable,
    IPausable,
    IMessageHandler,
    IERC20Manager
{
    using SafeERC20 for IERC20;

    using EnumerableSet for EnumerableSet.Bytes32Set;
    using EnumerableSet for EnumerableSet.AddressSet;

    using CustomEnumerableMap for CustomEnumerableMap.AddressToTokenTypeMap;

    /**
     * @dev `bytes32 sender` size.
     */
    uint256 internal constant SENDER_SIZE = 32;
    /**
     * @dev `address receiver` size.
     */
    uint256 internal constant RECEIVER_SIZE = 20;
    /**
     * @dev `address token` size.
     */
    uint256 internal constant TOKEN_SIZE = 20;
    /**
     * @dev `uint256 amount` size.
     */
    uint256 internal constant AMOUNT_SIZE = 32;

    /**
     * @dev Size of transfer message.
     */
    uint256 internal constant TRANSFER_MESSAGE_SIZE = SENDER_SIZE + RECEIVER_SIZE + TOKEN_SIZE + AMOUNT_SIZE;

    /**
     * @dev `address receiver` bit shift.
     */
    uint256 internal constant RECEIVER_BIT_SHIFT = 96;
    /**
     * @dev `address token` bit shift.
     */
    uint256 internal constant TOKEN_BIT_SHIFT = 96;

    /**
     * @dev `SENDER_SIZE` offset.
     */
    uint256 internal constant OFFSET1 = 32;
    /**
     * @dev `SENDER_SIZE + RECEIVER_SIZE` offset.
     */
    uint256 internal constant OFFSET2 = 52;
    /**
     * @dev `SENDER_SIZE + RECEIVER_SIZE + TOKEN_SIZE` offset.
     */
    uint256 internal constant OFFSET3 = 72;

    //////////////////////////////////////////////////////////////////////////////

    /**
     * @dev `uint8 discriminant` size.
     */
    uint256 internal constant DISCRIMINANT_SIZE = 1;

    /**
     * @dev `uint8 discriminant` bit shift.
     */
    uint256 internal constant DISCRIMINANT_BIT_SHIFT = 248;

    /**
     * @dev `DISCRIMINANT_SIZE` offset.
     */
    uint256 internal constant OFFSET4 = 1;

    //////////////////////////////////////////////////////////////////////////////

    /**
     * @dev Add VFT manager message discriminant.
     */
    uint256 internal constant ADD_VFT_MANAGER = 0x00;

    /**
     * @dev `bytes32 vftManager` size.
     */
    uint256 internal constant VFT_MANAGER_SIZE = 32;

    /**
     * @dev Size of add VFT manager message.
     */
    uint256 internal constant ADD_VFT_MANAGER_MESSAGE_SIZE = DISCRIMINANT_SIZE + VFT_MANAGER_SIZE;

    //////////////////////////////////////////////////////////////////////////////

    /**
     * @dev `bytes32 tokenName` size.
     */
    uint256 internal constant TOKEN_NAME_SIZE = 32;
    /**
     * @dev `bytes32 tokenSymbol` size.
     */
    uint256 internal constant TOKEN_SYMBOL_SIZE = 32;
    /**
     * @dev `uint8 tokenDecimals` size.
     */
    uint256 internal constant TOKEN_DECIMALS_SIZE = 1;

    /**
     * @dev `uint8 tokenDecimals` bit shift.
     */
    uint256 internal constant TOKEN_DECIMALS_BIT_SHIFT = 248;

    /**
     * @dev `DISCRIMINANT_SIZE + TOKEN_NAME_SIZE` offset.
     */
    uint256 internal constant OFFSET5 = 33;
    /**
     * @dev `DISCRIMINANT_SIZE + TOKEN_NAME_SIZE + TOKEN_SYMBOL_SIZE` offset.
     */
    uint256 internal constant OFFSET6 = 65;

    /**
     * @dev Size of register token message (for `TokenType.Gear`).
     */
    uint256 internal constant REGISTER_GEAR_TOKEN_MESSAGE_SIZE =
        DISCRIMINANT_SIZE + TOKEN_NAME_SIZE + TOKEN_SYMBOL_SIZE + TOKEN_DECIMALS_SIZE;

    //////////////////////////////////////////////////////////////////////////////

    /**
     * @dev `address token` size.
     */
    uint256 internal constant ETHEREUM_TOKEN_SIZE = 20;

    /**
     * @dev Size of register token message (for `TokenType.Ethereum`).
     */
    uint256 internal constant REGISTER_ETHEREUM_TOKEN_MESSAGE_SIZE = DISCRIMINANT_SIZE + ETHEREUM_TOKEN_SIZE;

    /**
     * @dev `address token` bit shift.
     */
    uint256 internal constant ETHEREUM_TOKEN_BIT_SHIFT = 96;

    bytes32 public constant PAUSER_ROLE = bytes32(uint256(0x01));

    IGovernance private _governanceAdmin;
    IGovernance private _governancePauser;
    address private _messageQueue;
    EnumerableSet.Bytes32Set private _vftManagers;
    CustomEnumerableMap.AddressToTokenTypeMap private _tokens;
    EnumerableSet.AddressSet private _bridgingPayments;

    /**
     * @custom:oz-upgrades-unsafe-allow constructor
     */
    constructor() {
        _disableInitializers();
    }

    /**
     * @dev Initializes the ERC20Manager contract with the message queue and VFT manager addresses.
     *      GovernanceAdmin contract is used to upgrade, pause/unpause the ERC20Manager contract.
     *      GovernancePauser contract is used to pause/unpause the ERC20Manager contract.
     * @param governanceAdmin_ The address of the GovernanceAdmin contract that will process messages.
     * @param governancePauser_ The address of the GovernanceAdmin contract that will process pauser messages.
     * @param messageQueue_ The address of the message queue contract.
     * @param vftManager The address of the VFT manager contract (on Vara Network).
     * @param tokens_ The tokens that will be registered.
     */
    function initialize(
        IGovernance governanceAdmin_,
        IGovernance governancePauser_,
        address messageQueue_,
        bytes32 vftManager,
        TokenInfo[] memory tokens_
    ) public initializer {
        __AccessControl_init();
        __Pausable_init();
        __UUPSUpgradeable_init();

        _grantRole(DEFAULT_ADMIN_ROLE, address(governanceAdmin_));

        _grantRole(PAUSER_ROLE, address(governanceAdmin_));
        _grantRole(PAUSER_ROLE, address(governancePauser_));

        _governanceAdmin = governanceAdmin_;
        _governancePauser = governancePauser_;
        _messageQueue = messageQueue_;
        _vftManagers.add(vftManager);

        for (uint256 i = 0; i < tokens_.length; i++) {
            TokenInfo memory tokenInfo = tokens_[i];

            if (tokenInfo.tokenType == TokenType.Unknown) {
                revert InvalidTokenType();
            } else {
                _tokens.set(tokenInfo.token, tokenInfo.tokenType);
            }
        }
    }

    /**
     * @dev Returns governance admin address.
     * @return governanceAdmin Governance admin address.
     */
    function governanceAdmin() external view returns (address) {
        return address(_governanceAdmin);
    }

    /**
     * @dev Returns governance pauser address.
     * @return governancePauser Governance pauser address.
     */
    function governancePauser() external view returns (address) {
        return address(_governancePauser);
    }

    /**
     * @dev Returns message queue address.
     * @return messageQueue Message queue address.
     */
    function messageQueue() external view returns (address) {
        return address(_messageQueue);
    }

    /**
     * @dev Returns total number of VFT managers.
     * @return totalVftManagers Total number of VFT managers.
     */
    function totalVftManagers() external view returns (uint256) {
        return _vftManagers.length();
    }

    /**
     * @dev Returns list of VFT managers.
     * @return vftManagers List of VFT managers.
     */
    function vftManagers() external view returns (bytes32[] memory) {
        return _vftManagers.values();
    }

    /**
     * @dev Returns list of VFT managers.
     * @param offset Offset of the first VFT manager to return.
     * @param limit Maximum number of VFT managers to return.
     * @return vftManagers List of VFT managers.
     */
    function vftManagers(uint256 offset, uint256 limit) external view returns (bytes32[] memory) {
        return paginate(_vftManagers, offset, limit);
    }

    /**
     * @dev Returns whether the VFT manager is registered.
     * @param vftManager VFT manager address.
     * @return isVftManager `true` if the VFT manager is registered, `false` otherwise.
     */
    function isVftManager(bytes32 vftManager) external view returns (bool) {
        return _vftManagers.contains(vftManager);
    }

    /**
     * @dev Returns total number of tokens.
     * @return totalTokens Total number of tokens.
     */
    function totalTokens() external view returns (uint256) {
        return _tokens.length();
    }

    /**
     * @dev Returns list of tokens.
     * @return tokens List of tokens.
     */
    function tokens() external view returns (address[] memory) {
        return _tokens.keys();
    }

    /**
     * @dev Returns list of tokens.
     * @param offset Offset of the first token to return.
     * @param limit Maximum number of tokens to return.
     * @return tokens List of tokens.
     */
    function tokens(uint256 offset, uint256 limit) external view returns (address[] memory) {
        bytes32[] memory store = paginate(_tokens._inner._keys, offset, limit);
        address[] memory result;

        assembly ("memory-safe") {
            result := store
        }

        return result;
    }

    /**
     * @dev Returns token type.
     * @param token Token address.
     * @return tokenType Token type. Returns `TokenType.Unknown` if token is not registered.
     */
    function getTokenType(address token) external view returns (TokenType) {
        (, TokenType tokenType) = _tokens.tryGet(token);
        return tokenType;
    }

    /**
     * @dev Returns total number of bridging payments.
     * @return totalBridgingPayments Total number of bridging payments.
     */
    function totalBridgingPayments() external view returns (uint256) {
        return _bridgingPayments.length();
    }

    /**
     * @dev Returns list of bridging payments.
     * @return bridgingPayments List of bridging payments.
     */
    function bridgingPayments() external view returns (address[] memory) {
        return _bridgingPayments.values();
    }

    /**
     * @dev Returns list of bridging payments.
     * @param offset Offset of the first bridging payment to return.
     * @param limit Maximum number of bridging payments to return.
     * @return bridgingPayments List of bridging payments.
     */
    function bridgingPayments(uint256 offset, uint256 limit) external view returns (address[] memory) {
        EnumerableSet.Bytes32Set storage bytes32Set;
        assembly ("memory-safe") {
            bytes32Set.slot := _bridgingPayments.slot
        }

        bytes32[] memory store = paginate(bytes32Set, offset, limit);
        address[] memory result;

        assembly ("memory-safe") {
            result := store
        }

        return result;
    }

    /**
     * @dev Returns whether the bridging payment is registered.
     * @param bridgingPayment Bridging payment address.
     * @return isBridgingPayment `true` if the bridging payment is registered, `false` otherwise.
     */
    function isBridgingPayment(address bridgingPayment) external view returns (bool) {
        return _bridgingPayments.contains(bridgingPayment);
    }

    /**
     * @dev Returns list of items from the set.
     * @param bytes32Set Set of items.
     * @param offset Offset of the first item to return.
     * @param limit Maximum number of items to return.
     * @return items List of items.
     */
    function paginate(EnumerableSet.Bytes32Set storage bytes32Set, uint256 offset, uint256 limit)
        private
        view
        returns (bytes32[] memory)
    {
        uint256 length = bytes32Set.length();

        if (offset >= length) {
            return new bytes32[](0);
        }

        uint256 end = offset + limit;
        if (end > length) {
            end = length;
        }

        uint256 size = end - offset;
        bytes32[] memory result = new bytes32[](size);

        for (uint256 i = 0; i < size; i++) {
            result[i] = bytes32Set.at(offset + i);
        }

        return result;
    }

    /**
     * @dev Pauses the contract.
     */
    function pause() public onlyRole(PAUSER_ROLE) {
        _pause();
    }

    /**
     * @dev Unpauses the contract.
     */
    function unpause() public onlyRole(PAUSER_ROLE) {
        _unpause();
    }

    /**
     * @dev Function that should revert when `msg.sender` is not authorized to upgrade the contract.
     *      Called by {upgradeToAndCall}.
     */
    function _authorizeUpgrade(address newImplementation) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}

    /**
     * @dev Requests bridging of tokens.
     *      Emits `BridgingRequested` event.
     * @param token Token address.
     * @param amount Amount of tokens to bridge.
     * @param to Destination address.
     * @dev Reverts if token is not registered with `InvalidTokenType` error.
     */
    function requestBridging(address token, uint256 amount, bytes32 to) public whenNotPaused {
        if (amount == 0) {
            revert InvalidAmount();
        }

        (, TokenType tokenType) = _tokens.tryGet(token);

        if (tokenType == TokenType.Unknown) {
            revert InvalidTokenType();
        } else if (tokenType == TokenType.Ethereum) {
            IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
        } else if (tokenType == TokenType.Gear) {
            IERC20Burnable(token).burnFrom(msg.sender, amount);
        }

        emit BridgingRequested(msg.sender, to, token, amount);
    }

    /**
     * @dev Requests bridging of tokens and pays fee to one of the `bridgingPayment` contracts.
     * @param token Token address.
     * @param amount Amount of tokens to bridge.
     * @param to Destination address.
     * @param bridgingPayment Bridging payment address.
     */
    function requestBridgingPayingFee(address token, uint256 amount, bytes32 to, address bridgingPayment)
        public
        payable
        whenNotPaused
    {
        if (!_bridgingPayments.contains(bridgingPayment)) {
            revert InvalidBridgingPayment();
        }

        IBridgingPayment(bridgingPayment).payFee{value: msg.value}();
        requestBridging(token, amount, to);
    }

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
    ) public payable whenNotPaused {
        if (!_bridgingPayments.contains(bridgingPayment)) {
            revert InvalidBridgingPayment();
        }

        IBridgingPayment(bridgingPayment).payFee{value: msg.value}();
        try IERC20Permit(token).permit(msg.sender, address(this), amount, deadline, v, r, s) {} catch {}
        requestBridging(token, amount, to);
    }

    /**
     * @dev Creates a new `BridgingPayment` contract (ERC20Manager is factory).
     *      Emits `BridgingPaymentCreated` event.
     * @param fee Fee amount in wei.
     * @return bridgingPaymentAddress Address of the created `bridgingPayment` contract.
     */
    function createBridgingPayment(uint256 fee) external whenNotPaused returns (address) {
        BridgingPayment bridgingPayment = new BridgingPayment(address(this), fee, msg.sender);

        address bridgingPaymentAddress = address(bridgingPayment);
        _bridgingPayments.add(bridgingPaymentAddress);

        emit BridgingPaymentCreated(bridgingPaymentAddress);

        return bridgingPaymentAddress;
    }

    /**
     * @dev Handles message originated from Vara Network.
     * @param source Source of the message (`ActorId` from Vara Network).
     * @param payload Payload of the message (message from Vara Network).
     */
    function handleMessage(bytes32 source, bytes calldata payload) external {
        if (msg.sender != _messageQueue) {
            revert InvalidSender();
        }

        if (_vftManagers.contains(source)) {
            if (!_tryParseAndApplyTransferMessage(payload)) {
                revert InvalidPayload();
            }
        } else if (source == _governanceAdmin.governance()) {
            if (!_tryParseAndApplyGovernanceMessage(payload)) {
                revert InvalidPayload();
            }
        } else {
            revert InvalidSource();
        }
    }

    /**
     * @dev Tries to parse and apply transfer message originated from Vara Network.
     *
     *      Payload format:
     *      ```solidity
     *      address sender;
     *      address receiver;
     *      address token;
     *      uint256 amount;
     *      ```
     *
     * @param payload Payload of the message (message from Vara Network).
     * @return success `true` if the message is parsed and applied, `false` otherwise.
     */
    function _tryParseAndApplyTransferMessage(bytes calldata payload) private returns (bool) {
        if (!(payload.length == TRANSFER_MESSAGE_SIZE)) {
            return false;
        }

        bytes32 sender;
        address receiver;
        address token;
        uint256 amount;

        // we use offset `OFFSET1 = SENDER_SIZE` to skip `bytes32 sender`
        assembly ("memory-safe") {
            sender := calldataload(payload.offset)
            // `RECEIVER_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            receiver := shr(RECEIVER_BIT_SHIFT, calldataload(add(payload.offset, OFFSET1)))
            // `TOKEN_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            token := shr(TOKEN_BIT_SHIFT, calldataload(add(payload.offset, OFFSET2)))
            amount := calldataload(add(payload.offset, OFFSET3))
        }

        (, TokenType tokenType) = _tokens.tryGet(token);

        if (tokenType == TokenType.Unknown) {
            revert InvalidTokenType();
        } else if (tokenType == TokenType.Ethereum) {
            IERC20(token).safeTransfer(receiver, amount);
        } else if (tokenType == TokenType.Gear) {
            IERC20Mintable(token).mint(receiver, amount);
        }

        emit Bridged(sender, receiver, token, amount);

        return true;
    }

    /**
     * @dev Tries to parse and apply governance message originated from Vara Network.
     *
     *      Payload format:
     *      ```solidity
     *      uint8 discriminant;
     *      ```
     *
     *      `discriminant` can be:
     *      - `ADD_VFT_MANAGER = 0x00` - add new VFT manager to list of registered VFT managers
     *          ```solidity
     *          bytes32 vftManager; // 32 bytes
     *          ```
     *
     *      - `TokenType.Ethereum = 0x01` - register Ethereum token
     *          ```solidity
     *          address token; // 20 bytes
     *          ```
     *
     *      - `TokenType.Gear = 0x02` - register Gear token
     *          ```solidity
     *          bytes32 tokenName; // 1 byte length + 31 bytes datas
     *          bytes32 tokenSymbol; // 1 byte length + 31 bytes data
     *          uint8 tokenDecimals; // 1 byte
     *          ```
     *
     * @param payload Payload of the message (message from Vara Network).
     * @return success `true` if the message is parsed and applied, `false` otherwise.
     */
    function _tryParseAndApplyGovernanceMessage(bytes calldata payload) private returns (bool) {
        if (!(payload.length > 0)) {
            return false;
        }

        uint256 discriminant;
        assembly ("memory-safe") {
            // `DISCRIMINANT_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            discriminant := shr(DISCRIMINANT_BIT_SHIFT, calldataload(payload.offset))
        }

        if (!(discriminant >= ADD_VFT_MANAGER && discriminant <= uint256(TokenType.Gear))) {
            return false;
        }

        if (discriminant == ADD_VFT_MANAGER) {
            if (!(payload.length == ADD_VFT_MANAGER_MESSAGE_SIZE)) {
                return false;
            }

            // we use offset `OFFSET4 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
            bytes32 vftManager;
            assembly ("memory-safe") {
                vftManager := calldataload(add(payload.offset, OFFSET4))
            }

            _vftManagers.add(vftManager);

            emit VftManagerAdded(vftManager);

            return true;
        }

        if (discriminant == uint256(TokenType.Ethereum)) {
            if (!(payload.length == REGISTER_ETHEREUM_TOKEN_MESSAGE_SIZE)) {
                return false;
            }

            // we use offset `OFFSET4 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
            address token;
            assembly ("memory-safe") {
                // `ETHEREUM_TOKEN_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
                token := shr(ETHEREUM_TOKEN_BIT_SHIFT, calldataload(add(payload.offset, OFFSET4)))
            }

            _tokens.set(token, TokenType.Ethereum);

            emit EthereumTokenRegistered(token);

            return true;
        }

        // `discriminant == uint256(TokenType.Gear)` is guaranteed by previous checks
        if (!(payload.length == REGISTER_GEAR_TOKEN_MESSAGE_SIZE)) {
            return false;
        }

        bytes32 tokenName;
        bytes32 tokenSymbol;
        uint8 tokenDecimals;

        // we use offset `OFFSET4 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
        // we use offset `OFFSET5 = DISCRIMINANT_SIZE + TOKEN_NAME_SIZE` to skip `uint8 discriminant` and `bytes32 tokenName`
        // we use offset `OFFSET6 = DISCRIMINANT_SIZE + TOKEN_NAME_SIZE + TOKEN_SYMBOL_SIZE` to skip `uint8 discriminant`, `bytes32 tokenName` and `bytes32 tokenSymbol`
        assembly ("memory-safe") {
            tokenName := calldataload(add(payload.offset, OFFSET4))
            tokenSymbol := calldataload(add(payload.offset, OFFSET5))
            tokenDecimals := calldataload(add(payload.offset, OFFSET6))
            // `TOKEN_DECIMALS_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            tokenDecimals := shr(TOKEN_DECIMALS_BIT_SHIFT, calldataload(add(payload.offset, OFFSET6)))
        }

        uint8 tokenNameLength = uint8(tokenName[0]);
        if (!(tokenNameLength >= 1 && tokenNameLength <= 31)) {
            return false;
        }

        uint8 tokenSymbolLength = uint8(tokenSymbol[0]);
        if (!(tokenSymbolLength >= 1 && tokenSymbolLength <= 31)) {
            return false;
        }

        string memory tokenNameStr = LibString.unpackOne(tokenName);
        string memory tokenSymbolStr = LibString.unpackOne(tokenSymbol);

        ERC20GearSupply gearSupply = new ERC20GearSupply(address(this), tokenNameStr, tokenSymbolStr, tokenDecimals);
        address tokenAddress = address(gearSupply);

        _tokens.set(tokenAddress, TokenType.Gear);

        emit GearTokenRegistered(tokenAddress, tokenNameStr, tokenSymbolStr, tokenDecimals);

        return true;
    }
}
