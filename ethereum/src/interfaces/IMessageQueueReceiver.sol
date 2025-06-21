// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the message receiver.
 */
interface IMessageQueueReceiver {
    /**
     * @dev Processes message originated from Vara Network.
     * @param sender Sender of the message.
     * @param payload Payload of the message.
     */
    function processVaraMessage(bytes32 sender, bytes calldata payload) external;
}
