// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageQueueProcessor} from "./IMessageQueueProcessor.sol";

/**
 * @dev Interface for the Governance contract.
 */
interface IGovernance is IMessageQueueProcessor {
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
     * @dev Emitted when the governance address is changed.
     * @param previousGovernance The previous governance address.
     * @param newGovernance The new governance address.
     */
    event GovernanceChanged(bytes32 indexed previousGovernance, bytes32 indexed newGovernance);

    /**
     * @dev Returns the governance address.
     * @return governance The governance address.
     */
    function governance() external view returns (bytes32);

    /**
     * @dev Returns the message queue address.
     * @return messageQueue The message queue address.
     */
    function messageQueue() external view returns (address);
}
