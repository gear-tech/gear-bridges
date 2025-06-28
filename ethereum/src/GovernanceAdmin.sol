// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {GovernanceConstants, IGovernance} from "./interfaces/IGovernance.sol";
import {IMessageQueueProcessor} from "./interfaces/IMessageQueueProcessor.sol";
import {IPausable} from "./interfaces/IPausable.sol";
import {IUUPSUpgradeable} from "./interfaces/IUUPSUpgradeable.sol";

/**
 * @dev GovernanceAdmin smart contract is responsible for processing messages
 *      originated from Vara Network. It is used to change governance address,
 *      upgrade proxies and pause/unpause them.
 */
contract GovernanceAdmin is IMessageQueueProcessor, IGovernance {
    uint256 internal constant DISCRIMINANT_BIT_SHIFT = 248;
    uint256 internal constant PROXY_ADDRESS_BIT_SHIFT = 96;
    uint256 internal constant NEW_IMPLEMENTATION_BIT_SHIFT = 96;

    uint256 internal constant OFFSET1 = 1; // DISCRIMINANT_SIZE
    uint256 internal constant OFFSET2 = 21; // DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE

    bytes32 public governance;
    address public messageQueue;
    mapping(address proxy => bool isKnownProxy) private _proxies;

    /**
     * @dev Initializes the GovernanceAdmin contract.
     * @param _governance The governance address (Vara Network address).
     * @param _messageQueue The message queue address.
     * @param proxies The proxies addresses (Relayer, MessageQueue, ERC20Manager).
     */
    constructor(bytes32 _governance, address _messageQueue, address[] memory proxies) {
        governance = _governance;
        messageQueue = _messageQueue;

        for (uint256 i = 0; i < proxies.length; i++) {
            address proxy = proxies[i];
            _proxies[proxy] = true;
        }
    }

    /**
     * @dev Processes message originated from Vara Network.
     * @param source Source of the message (`ActorId` from Vara Network).
     * @param payload Payload of the message (message from Vara Network).
     */
    function processMessage(bytes32 source, bytes calldata payload) external {
        if (msg.sender != messageQueue) {
            revert InvalidSender();
        }

        if (source != governance) {
            revert InvalidSource();
        }

        if (!_tryParseAndApplyMessage(payload)) {
            revert InvalidPayload();
        }
    }

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
        assembly ("memory-safe") {
            // `DISCRIMINANT_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            discriminant := shr(DISCRIMINANT_BIT_SHIFT, calldataload(payload.offset))
        }

        if (
            !(discriminant >= GovernanceConstants.CHANGE_GOVERNANCE && discriminant <= GovernanceConstants.UPGRADE_PROXY)
        ) {
            return false;
        }

        if (discriminant == GovernanceConstants.CHANGE_GOVERNANCE) {
            if (!(payload.length == GovernanceConstants.CHANGE_GOVERNANCE_SIZE)) {
                return false;
            }

            // we use offset `OFFSET1 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
            bytes32 newGovernance;
            assembly ("memory-safe") {
                newGovernance := calldataload(add(payload.offset, OFFSET1))
            }

            bytes32 previousGovernance = governance;
            governance = newGovernance;

            emit GovernanceChanged(previousGovernance, newGovernance);

            return true;
        }

        if (!(payload.length > GovernanceConstants.PROXY_ADDRESS_SIZE)) {
            return false;
        }

        // we use offset `OFFSET1 = DISCRIMINANT_SIZE` to skip `uint8 discriminant`
        address proxy;
        assembly ("memory-safe") {
            // `PROXY_ADDRESS_BIT_SHIFT` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            proxy := shr(PROXY_ADDRESS_BIT_SHIFT, calldataload(add(payload.offset, OFFSET1)))
        }

        if (!(_proxies[proxy])) {
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

        if (discriminant == GovernanceConstants.UPGRADE_PROXY) {
            if (!(payload.length >= GovernanceConstants.UPGRADE_PROXY_SIZE)) {
                return false;
            }

            // we use offset `OFFSET2 = DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE` to skip `uint8 discriminant` and `address proxy`
            address newImplementation;
            assembly ("memory-safe") {
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

        return true;
    }
}
