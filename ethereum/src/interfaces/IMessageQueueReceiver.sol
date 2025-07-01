// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

interface IMessageQueueReceiver {
    function processVaraMessage(bytes32 sender, bytes calldata payload) external;
}
