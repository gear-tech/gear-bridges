// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {console, Script} from "forge-std/Script.sol";
import {MessageQueue} from "src/MessageQueue.sol";

contract MessageQueueScript is Script {
    MessageQueue public messageQueue;

    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(privateKey);

        messageQueue = new MessageQueue();
        console.log("MessageQueue:", address(messageQueue));

        vm.stopBroadcast();
    }
}
