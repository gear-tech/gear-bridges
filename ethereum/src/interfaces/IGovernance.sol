// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageQueueProcessor} from "./IMessageQueueProcessor.sol";

/**
 * @dev Governance constants.
 */
library GovernanceConstants {
    uint256 internal constant CHANGE_GOVERNANCE = 0x00;
    uint256 internal constant PAUSE_PROXY = 0x01;
    uint256 internal constant UNPAUSE_PROXY = 0x02;
    uint256 internal constant UPGRADE_PROXY = 0x03;

    uint256 internal constant DISCRIMINANT_SIZE = 1; // `uint8 discriminant`
    uint256 internal constant NEW_GOVERNANCE_SIZE = 32; // `bytes32 newGovernance`
    uint256 internal constant PROXY_ADDRESS_SIZE = 20; // `address proxy`
    uint256 internal constant NEW_IMPLEMENTATION_SIZE = 20; // `address newImplementation`

    uint256 internal constant OFFSET1 = 1; // DISCRIMINANT_SIZE
    uint256 internal constant OFFSET2 = 21; // DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE
    uint256 internal constant OFFSET3 = 41; // DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE + NEW_IMPLEMENTATION_SIZE

    uint256 internal constant CHANGE_GOVERNANCE_SIZE = 33; // DISCRIMINANT_SIZE + NEW_GOVERNANCE_SIZE
    uint256 internal constant PAUSE_UNPAUSE_PROXY_SIZE = 21; // DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE
    uint256 internal constant UPGRADE_PROXY_SIZE = 41; // DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE + NEW_IMPLEMENTATION_SIZE
}

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
