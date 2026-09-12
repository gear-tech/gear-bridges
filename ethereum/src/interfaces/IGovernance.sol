// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {ERC20Manager} from "src/ERC20Manager.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IMessageHandler} from "src/interfaces/IMessageHandler.sol";

/**
 * @dev Interface for the Governance contract.
 */
interface IGovernance is IMessageHandler {
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
     * @dev Returns the WrappedVara contract.
     * @return wrappedVara The WrappedVara contract.
     */
    function wrappedVara() external view returns (WrappedVara);

    /**
     * @dev Returns the MessageQueue contract.
     * @return messageQueue The MessageQueue contract.
     */
    function messageQueue() external view returns (MessageQueue);

    /**
     * @dev Returns the ERC20Manager contract.
     * @return erc20Manager The ERC20Manager contract.
     */
    function erc20Manager() external view returns (ERC20Manager);
}
