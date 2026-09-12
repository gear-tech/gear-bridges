// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceConstants} from "src/GovernanceConstants.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IMessageHandler} from "src/interfaces/IMessageHandler.sol";
import {IPausable} from "src/interfaces/IPausable.sol";
import {IUUPSUpgradeable} from "src/interfaces/IUUPSUpgradeable.sol";

/**
 * @dev GovernanceAdmin smart contract is responsible for processing messages
 *      originated from Vara Network. It is used to change governance address,
 *      upgrade proxies and pause/unpause them.
 */
contract GovernanceAdmin is IMessageHandler, IGovernance {
    /**
     * @dev `uint8 discriminant` bit shift.
     */
    uint256 internal constant DISCRIMINANT_BIT_SHIFT = 248;
    /**
     * @dev `address proxy` bit shift.
     */
    uint256 internal constant PROXY_ADDRESS_BIT_SHIFT = 96;
    /**
     * @dev `address newImplementation` bit shift.
     */
    uint256 internal constant NEW_IMPLEMENTATION_BIT_SHIFT = 96;

    /**
     * @dev `DISCRIMINANT_SIZE` offset.
     */
    uint256 internal constant OFFSET1 = 1;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE` offset.
     */
    uint256 internal constant OFFSET2 = 21;

    bytes32 public governance;
    WrappedVara public wrappedVara;
    MessageQueue public messageQueue;
    ERC20Manager public erc20Manager;

    /**
     * @dev Initializes the GovernanceAdmin contract.
     * @param _governance The governance address (Vara Network address).
     * @param _wrappedVara The WrappedVara contract.
     * @param _messageQueue The MessageQueue contract.
     * @param _erc20Manager The ERC20Manager contract.
     */
    constructor(bytes32 _governance, WrappedVara _wrappedVara, MessageQueue _messageQueue, ERC20Manager _erc20Manager) {
        governance = _governance;
        wrappedVara = _wrappedVara;
        messageQueue = _messageQueue;
        erc20Manager = _erc20Manager;
    }

    /**
     * @dev Handles message originated from Vara Network.
     * @param source Source of the message (`ActorId` from Vara Network).
     * @param payload Payload of the message (message from Vara Network).
     */
    function handleMessage(bytes32 source, bytes calldata payload) external {
        if (msg.sender != address(messageQueue)) {
            revert InvalidSender();
        }

        if (source != governance) {
            revert InvalidSource();
        }

        if (!_tryParseAndApplyMessage(payload)) {
            revert InvalidPayload();
        }
    }

    /// forge-lint: disable-next-item(cyclomatic-complexity)
    /**
     * @dev Tries to parse and apply message originated from Vara Network.
     *
     *      Payload format:
     *      ```solidity
     *      uint8 discriminant;
     *      ```
     *
     *      `discriminant` can be:
     *      - `GovernanceConstants.CHANGE_GOVERNANCE = 0x00` - change governance address to `newGovernance`
     *          ```solidity
     *          bytes32 newGovernance;
     *          ```
     *
     *      - `GovernanceConstants.PAUSE_PROXY = 0x01` - pause `proxy`
     *          ```solidity
     *          address proxy;
     *          ```
     *
     *      - `GovernanceConstants.UNPAUSE_PROXY = 0x02` - unpause `proxy`
     *          ```solidity
     *          address proxy;
     *          ```
     *
     *      - `GovernanceConstants.UPGRADE_PROXY = 0x03` - upgrade `proxy` to `newImplementation` and call `data` on it
     *          ```solidity
     *          address proxy;
     *          address newImplementation;
     *          bytes data;
     *          ```
     *
     * @param payload Payload of the message (message from Vara Network).
     * @return success `true` if the message is parsed and applied, `false` otherwise.
     */
    function _tryParseAndApplyMessage(bytes calldata payload) private returns (bool) {
        if (!(payload.length > 0)) {
            return false;
        }

        uint256 discriminant;
        // forge-lint: disable-next-item(inline-assembly)
        assembly ("memory-safe") {
            /* reviewed: ... */
            // `DISCRIMINANT_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            discriminant := shr(DISCRIMINANT_BIT_SHIFT, calldataload(payload.offset))
        }

        if (!(discriminant >= GovernanceConstants.CHANGE_GOVERNANCE
                    && discriminant <= GovernanceConstants.UPGRADE_PROXY)) {
            return false;
        }

        if (discriminant == GovernanceConstants.CHANGE_GOVERNANCE) {
            if (!(payload.length == GovernanceConstants.CHANGE_GOVERNANCE_SIZE)) {
                return false;
            }

            // we use offset `OFFSET1 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
            bytes32 newGovernance;
            // forge-lint: disable-next-item(inline-assembly)
            assembly ("memory-safe") {
                /* reviewed: ... */
                newGovernance := calldataload(add(payload.offset, OFFSET1))
            }

            bytes32 previousGovernance = governance;
            governance = newGovernance;

            // forge-lint: disable-next-item(reentrancy-events)
            emit GovernanceChanged(previousGovernance, newGovernance);

            return true;
        }

        if (!(payload.length > GovernanceConstants.PROXY_ADDRESS_SIZE)) {
            return false;
        }

        // we use offset `OFFSET1 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
        address proxy;
        // forge-lint: disable-next-item(inline-assembly)
        assembly ("memory-safe") {
            /* reviewed: ... */
            // `PROXY_ADDRESS_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            proxy := shr(PROXY_ADDRESS_BIT_SHIFT, calldataload(add(payload.offset, OFFSET1)))
        }

        if (!(proxy == address(wrappedVara) || proxy == address(messageQueue) || proxy == address(erc20Manager))) {
            return false;
        }

        if (discriminant >= GovernanceConstants.PAUSE_PROXY && discriminant <= GovernanceConstants.UNPAUSE_PROXY) {
            if (!(payload.length == GovernanceConstants.PAUSE_UNPAUSE_PROXY_SIZE)) {
                return false;
            }

            if (discriminant == GovernanceConstants.PAUSE_PROXY) {
                IPausable(proxy).pause();
            } else if (discriminant == GovernanceConstants.UNPAUSE_PROXY) {
                IPausable(proxy).unpause();
            }

            return true;
        }

        // `discriminant == GovernanceConstants.UPGRADE_PROXY` is guaranteed by previous checks
        if (!(payload.length >= GovernanceConstants.UPGRADE_PROXY_SIZE)) {
            return false;
        }

        // we use offset `OFFSET2 = DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE` to skip `uint8 discriminant` and `address proxy`
        address newImplementation;
        // forge-lint: disable-next-item(inline-assembly)
        assembly ("memory-safe") {
            /* reviewed: ... */
            // `NEW_IMPLEMENTATION_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            newImplementation := shr(NEW_IMPLEMENTATION_BIT_SHIFT, calldataload(add(payload.offset, OFFSET2)))
        }
        // we use offset `OFFSET3 = DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE + NEW_IMPLEMENTATION_SIZE`
        // to skip `uint8 discriminant`, `address proxy` and `address newImplementation`
        // and get `bytes data`
        bytes calldata data = payload[GovernanceConstants.OFFSET3:];

        IUUPSUpgradeable(proxy).upgradeToAndCall(newImplementation, data);

        return true;
    }
}
