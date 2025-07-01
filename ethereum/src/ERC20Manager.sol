// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ERC20GearSupply} from "./erc20/managed/ERC20GearSupply.sol";
import {LibString} from "./libraries/LibString.sol";
import {BridgingPayment} from "./BridgingPayment.sol";
import {IBridgingPayment} from "./interfaces/IBridgingPayment.sol";
import {IERC20Burnable} from "./interfaces/IERC20Burnable.sol";
import {IERC20Manager} from "./interfaces/IERC20Manager.sol";
import {IERC20Mintable} from "./interfaces/IERC20Mintable.sol";
import {IGovernance} from "./interfaces/IGovernance.sol";
import {IMessageHandler} from "./interfaces/IMessageHandler.sol";

contract ERC20Manager is
    Initializable,
    AccessControlUpgradeable,
    PausableUpgradeable,
    UUPSUpgradeable,
    IMessageHandler,
    IERC20Manager
{
    using SafeERC20 for IERC20;

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
    uint256 internal constant TOKEN1_SIZE = 20;
    /**
     * @dev `uint256 amount` size.
     */
    uint256 internal constant AMOUNT_SIZE = 32;

    /**
     * @dev Size of withdraw message.
     */
    uint256 internal constant WITHDRAW_MESSAGE_SIZE = SENDER_SIZE + RECEIVER_SIZE + TOKEN1_SIZE + AMOUNT_SIZE;

    /**
     * @dev `address receiver` bit shift.
     */
    uint256 internal constant RECEIVER_BIT_SHIFT = 96;
    /**
     * @dev `address token` bit shift.
     */
    uint256 internal constant TOKEN1_BIT_SHIFT = 96;

    /**
     * @dev `SENDER_SIZE` offset.
     */
    uint256 internal constant OFFSET1 = 32;
    /**
     * @dev `SENDER_SIZE + RECEIVER_SIZE` offset.
     */
    uint256 internal constant OFFSET2 = 52;
    /**
     * @dev `SENDER_SIZE + RECEIVER_SIZE + TOKEN1_SIZE` offset.
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
     * @dev Size of register token message (for `SupplyType.Ethereum`).
     */
    uint256 internal constant REGISTER_ETHEREUM_TOKEN_MESSAGE_SIZE =
        DISCRIMINANT_SIZE + TOKEN_NAME_SIZE + TOKEN_SYMBOL_SIZE + TOKEN_DECIMALS_SIZE;

    //////////////////////////////////////////////////////////////////////////////

    /**
     * @dev `address token` size.
     */
    uint256 internal constant TOKEN2_SIZE = 20;

    /**
     * @dev Size of register token message (for `SupplyType.Gear`).
     */
    uint256 internal constant REGISTER_GEAR_TOKEN_MESSAGE_SIZE = DISCRIMINANT_SIZE + TOKEN2_SIZE;

    /**
     * @dev `address token` bit shift.
     */
    uint256 internal constant TOKEN2_BIT_SHIFT = 96;

    bytes32 public constant PAUSER_ROLE = bytes32(uint256(0x01));

    IGovernance private _governanceAdmin;
    IGovernance private _governancePauser;
    address private _messageQueue;
    bytes32 private _vftManager;
    mapping(address token => SupplyType supplyType) private _tokenSupplyType;
    mapping(address bridgingPayment => bool isKnown) private _knownBridgingPayments;

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
     * @param governanceAdmin The address of the GovernanceAdmin contract that will process messages.
     * @param governancePauser The address of the GovernanceAdmin contract that will process pauser messages.
     * @param messageQueue The address of the message queue contract.
     * @param vftManager The address of the VFT manager contract (on Vara Network).
     * @param tokens The tokens that will be registered.
     */
    function initialize(
        IGovernance governanceAdmin,
        IGovernance governancePauser,
        address messageQueue,
        bytes32 vftManager,
        TokenWithSupplyType[] memory tokens
    ) public initializer {
        __AccessControl_init();
        __Pausable_init();
        __UUPSUpgradeable_init();

        _grantRole(DEFAULT_ADMIN_ROLE, address(governanceAdmin));

        _grantRole(PAUSER_ROLE, address(governanceAdmin));
        _grantRole(PAUSER_ROLE, address(governancePauser));

        _governanceAdmin = governanceAdmin;
        _governancePauser = governancePauser;
        _messageQueue = messageQueue;
        _vftManager = vftManager;

        for (uint256 i = 0; i < tokens.length; i++) {
            TokenWithSupplyType memory tokenWithSupplyType = tokens[i];

            if (tokenWithSupplyType.supplyType == SupplyType.Unknown) {
                revert InvalidSupplyType();
            } else {
                _tokenSupplyType[tokenWithSupplyType.token] = tokenWithSupplyType.supplyType;
            }
        }
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

    function requestBridging(address token, uint256 amount, bytes32 to) public whenNotPaused {
        SupplyType supplyType = _tokenSupplyType[token];

        if (supplyType == SupplyType.Unknown) {
            revert InvalidSupplyType();
        } else if (supplyType == SupplyType.Ethereum) {
            IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
        } else if (supplyType == SupplyType.Gear) {
            IERC20Burnable(token).burnFrom(msg.sender, amount);
        }

        emit BridgingRequested(msg.sender, to, token, amount);
    }

    function requestBridgingPayingFee(address token, uint256 amount, bytes32 to, address bridgingPayment)
        public
        payable
        whenNotPaused
    {
        if (!_knownBridgingPayments[bridgingPayment]) {
            revert InvalidBridgingPayment();
        }

        IBridgingPayment(bridgingPayment).payFee{value: msg.value}();
        requestBridging(token, amount, to);
    }

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
        if (!_knownBridgingPayments[bridgingPayment]) {
            revert InvalidBridgingPayment();
        }

        IBridgingPayment(bridgingPayment).payFee{value: msg.value}();
        try IERC20Permit(token).permit(msg.sender, address(this), amount, deadline, v, r, s) {} catch {}
        requestBridging(token, amount, to);
    }

    function createBridgingPayment(uint256 fee) external whenNotPaused returns (address) {
        BridgingPayment bridgingPayment = new BridgingPayment(address(this), fee, msg.sender);

        address bridgingPaymentAddress = address(bridgingPayment);
        _knownBridgingPayments[bridgingPaymentAddress] = true;

        emit BridgingPaymentCreated(bridgingPaymentAddress);

        return bridgingPaymentAddress;
    }

    function handleMessage(bytes32 source, bytes calldata payload) external {
        if (msg.sender != _messageQueue) {
            revert InvalidSender();
        }

        if (source == _vftManager) {
            if (!_tryParseAndApplyWithdrawMessage(payload)) {
                revert InvalidPayload();
            }
        } else if (source == _governanceAdmin.governance()) {
            if (!_tryParseAndApplyRegisterTokenMessage(payload)) {
                revert InvalidPayload();
            }
        } else {
            revert InvalidSource();
        }
    }

    /**
     * @dev Tries to parse and apply withdraw message originated from Vara Network.
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
    function _tryParseAndApplyWithdrawMessage(bytes calldata payload) private returns (bool) {
        if (!(payload.length == WITHDRAW_MESSAGE_SIZE)) {
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
            // `TOKEN1_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            token := shr(TOKEN1_BIT_SHIFT, calldataload(add(payload.offset, OFFSET2)))
            amount := calldataload(add(payload.offset, OFFSET3))
        }

        SupplyType supplyType = _tokenSupplyType[token];

        if (supplyType == SupplyType.Unknown) {
            revert InvalidSupplyType();
        } else if (supplyType == SupplyType.Ethereum) {
            IERC20(token).safeTransfer(receiver, amount);
        } else if (supplyType == SupplyType.Gear) {
            IERC20Mintable(token).mint(receiver, amount);
        }

        emit BridgingAccepted(sender, receiver, token, amount);

        return true;
    }

    /**
     * @dev Tries to parse and apply register token message originated from Vara Network.
     *
     *      Payload format:
     *      ```solidity
     *      uint8 discriminant;
     *      ```
     *
     *      `discriminant` can be:
     *      - `SupplyType.Ethereum = 0x01` - register Ethereum token
     *          ```solidity
     *          bytes32 tokenName; // 1 byte length + 31 bytes datas
     *          bytes32 tokenSymbol; // 1 byte length + 31 bytes data
     *          uint8 tokenDecimals; // 1 byte
     *          ```
     *
     *      - `SupplyType.Gear = 0x02` - register Gear token
     *          ```solidity
     *          address token; // 20 bytes
     *          ```
     *
     * @param payload Payload of the message (message from Vara Network).
     * @return success `true` if the message is parsed and applied, `false` otherwise.
     */
    function _tryParseAndApplyRegisterTokenMessage(bytes calldata payload) private returns (bool) {
        if (!(payload.length > 0)) {
            return false;
        }

        uint256 discriminant;
        assembly ("memory-safe") {
            // `DISCRIMINANT_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            discriminant := shr(DISCRIMINANT_BIT_SHIFT, calldataload(payload.offset))
        }

        if (!(discriminant >= uint256(SupplyType.Ethereum) && discriminant <= uint256(SupplyType.Gear))) {
            return false;
        }

        if (discriminant == uint256(SupplyType.Ethereum)) {
            if (!(payload.length == REGISTER_ETHEREUM_TOKEN_MESSAGE_SIZE)) {
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

            ERC20GearSupply token = new ERC20GearSupply(
                address(this), LibString.unpackOne(tokenName), LibString.unpackOne(tokenSymbol), tokenDecimals
            );
            _tokenSupplyType[address(token)] = SupplyType.Ethereum;
        }

        if (discriminant == uint256(SupplyType.Gear)) {
            if (!(payload.length == REGISTER_GEAR_TOKEN_MESSAGE_SIZE)) {
                return false;
            }

            // we use offset `OFFSET4 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
            address token;
            assembly ("memory-safe") {
                // `TOKEN2_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
                token := shr(TOKEN2_BIT_SHIFT, calldataload(add(payload.offset, OFFSET4)))
            }

            _tokenSupplyType[token] = SupplyType.Gear;
        }

        return true;
    }

    /**
     * @dev Returns supply type of token.
     * @param token Token address.
     * @return supplyType Supply type of token. Returns `SupplyType.Unknown` if token is not registered.
     */
    function getTokenSupplyType(address token) external view returns (SupplyType) {
        return _tokenSupplyType[token];
    }

    /**
     * @dev Returns whether the bridging payment is known.
     * @param bridgingPayment Bridging payment address.
     * @return isKnown `true` if the bridging payment is known, `false` otherwise.
     */
    function isKnownBridgingPayment(address bridgingPayment) external view returns (bool) {
        return _knownBridgingPayments[bridgingPayment];
    }
}
