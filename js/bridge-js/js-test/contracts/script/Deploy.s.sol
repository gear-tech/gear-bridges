// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script, console} from "forge-std/Script.sol";
import {MessageHandler} from "src/MessageHandler.sol";

contract Deploy is Script {
    function run() external {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address queue = vm.envAddress("MESSAGE_QUEUE");
        bytes32 expectedVaraSource = vm.envBytes32("EXPECTED_VARA_SOURCE");
        address ethereumSender = vm.envAddress("ETHEREUM_SENDER");
        vm.startBroadcast(privateKey);

        MessageHandler messageHandler = new MessageHandler(queue, expectedVaraSource, ethereumSender);

        vm.stopBroadcast();

        console.log("MessageHandler deployed at:", address(messageHandler));
    }
}
