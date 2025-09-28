// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {GovernanceConstants, IGovernance} from "./interfaces/IGovernance.sol";
import {IMessageHandler} from "./interfaces/IMessageHandler.sol";
import {IPausable} from "./interfaces/IPausable.sol";

/**
 * @dev GovernancePauser smart contract is responsible for processing messages
 *      originated from Vara Network. It is used to change governance address,
 *      and pause/unpause proxies.
 */
contract GovernancePauser is IMessageHandler, IGovernance {
    /**
     * @dev `uint8 discriminant` bit shift.
     */
    uint256 internal constant DISCRIMINANT_BIT_SHIFT = 248;
    /**
     * @dev `address proxy` bit shift.
     */
    uint256 internal constant PROXY_ADDRESS_BIT_SHIFT = 96;

    /**
     * @dev `DISCRIMINANT_SIZE` offset.
     */
    uint256 internal constant OFFSET1 = 1;

    bytes32 public governance;
    address public wrappedVara;
    address public messageQueue;
    address public erc20Manager;

    /**
     * @dev Initializes the GovernancePauser contract.
     * @param _governance The governance address (Vara Network address).
     * @param _wrappedVara The WrappedVara address.
     * @param _messageQueue The message queue address.
     * @param _erc20Manager The ERC20Manager address.
     */
    constructor(bytes32 _governance, address _wrappedVara, address _messageQueue, address _erc20Manager) {
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
            !(discriminant >= GovernanceConstants.CHANGE_GOVERNANCE && discriminant <= GovernanceConstants.UNPAUSE_PROXY)
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

        if (!(proxy == wrappedVara || proxy == messageQueue || proxy == erc20Manager)) {
            return false;
        }

        // `discriminant >= GovernanceConstants.PAUSE_PROXY && discriminant <= GovernanceConstants.UNPAUSE_PROXY`
        // is guaranteed by previous checks
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
}
