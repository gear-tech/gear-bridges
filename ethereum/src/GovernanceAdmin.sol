// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IGovernance} from "./interfaces/IGovernance.sol";
import {IMessageQueueProcessor} from "./interfaces/IMessageQueueProcessor.sol";
import {IPausable} from "./interfaces/IPausable.sol";
import {IUUPSUpgradeable} from "./interfaces/IUUPSUpgradeable.sol";

/**
 * @dev GovernanceAdmin smart contract is responsible for processing messages
 *      originated from Vara Network. It is used to change governance address,
 *      upgrade proxies and pause/unpause them.
 */
contract GovernanceAdmin is IMessageQueueProcessor, IGovernance {
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
     *      - `0x00` - change governance address to `newGovernance`
     *          ```solidity
     *          bytes32 newGovernance;
     *          ```
     *
     *      - `0x01` - pause `proxy`
     *          ```solidity
     *          address proxy;
     *          ```
     *
     *      - `0x02` - unpause `proxy`
     *          ```solidity
     *          address proxy;
     *          ```
     *
     *      - `0x03` - upgrade `proxy` to `newImplementation` and call `data` on it
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
            // `248` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            discriminant := shr(248, calldataload(payload.offset))
        }

        if (!(discriminant >= 0x00 && discriminant <= 0x03)) {
            return false;
        }

        if (discriminant == 0x00) {
            if (!(payload.length == 33)) {
                return false;
            }

            // we use offset 1 to skip `uint8 discriminant`
            bytes32 newGovernance;
            assembly ("memory-safe") {
                newGovernance := calldataload(add(payload.offset, 1))
            }

            bytes32 previousGovernance = governance;
            governance = newGovernance;

            emit GovernanceChanged(previousGovernance, newGovernance);

            return true;
        }

        if (!(payload.length > 20)) {
            return false;
        }

        // we use offset 1 to skip `uint8 discriminant`
        address proxy;
        assembly ("memory-safe") {
            // `96` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
            proxy := shr(96, calldataload(add(payload.offset, 1)))
        }

        if (!(_proxies[proxy])) {
            return false;
        }

        if (discriminant >= 0x01 && discriminant <= 0x02) {
            if (!(payload.length == 21)) {
                return false;
            }

            if (discriminant == 0x01) {
                IPausable(proxy).pause();
            } else if (discriminant == 0x02) {
                IPausable(proxy).unpause();
            }

            return true;
        }

        if (discriminant == 0x03) {
            if (!(payload.length > 40)) {
                return false;
            }

            // we use offset 21 to skip `uint8 discriminant` and `address proxy`
            address newImplementation;
            assembly ("memory-safe") {
                // `96` right bit shift is required to remove extra bits since `calldataload` returns `uint256`
                newImplementation := shr(96, calldataload(add(payload.offset, 21)))
            }
            bytes calldata data = payload[41:];

            IUUPSUpgradeable(proxy).upgradeToAndCall(newImplementation, data);

            return true;
        }

        return true;
    }
}
