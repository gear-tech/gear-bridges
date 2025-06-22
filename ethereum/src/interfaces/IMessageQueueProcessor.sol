// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the message processor.
 */
interface IMessageQueueProcessor {
    /**
     * @dev Processes message originated from Vara Network.
     * @param source Source of the message.
     * @param payload Payload of the message.
     */
    function processMessage(bytes32 source, bytes calldata payload) external;
}
