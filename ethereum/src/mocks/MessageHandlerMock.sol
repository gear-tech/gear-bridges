// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {IMessageHandler} from "src/interfaces/IMessageHandler.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";

/**
 * @dev Mock MessageHandler smart contract is responsible for
 *      handling messages originated from Vara Network.
 *      It is used for testing purposes.
 */
contract MessageHandlerMock is IMessageHandler, IMessageHandlerMock {
    /**
     * @dev Handles message originated from Vara Network.
     * @param source Source of the message (`ActorId` from Vara Network).
     * @param payload Payload of the message (message from Vara Network).
     */
    function handleMessage(bytes32 source, bytes calldata payload) external {
        emit MessageHandled(source, payload);
    }
}
