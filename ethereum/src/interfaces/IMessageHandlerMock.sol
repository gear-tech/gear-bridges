// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {IMessageHandler} from "./IMessageHandler.sol";

/**
 * @dev Interface for the MessageHandlerMock contract.
 */
interface IMessageHandlerMock is IMessageHandler {
    /**
     * @dev Emitted when the message is handled.
     * @param source Source of the message (`ActorId` from Vara Network).
     * @param payload Payload of the message (message from Vara Network).
     */
    event MessageHandled(bytes32 indexed source, bytes payload);
}
