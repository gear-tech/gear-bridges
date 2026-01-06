// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {IMessageHandler} from "ethereum/src/interfaces/IMessageHandler.sol";

contract MessageHandler is IMessageHandler {
    event MessageHandled(bytes32 indexed source, bytes payload);

    function handleMessage(bytes32 source, bytes calldata payload) external {
        emit MessageHandled(source, payload);
    }
}
