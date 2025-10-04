// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {MessageQueueScript} from "script/upgrades/MessageQueue.s.sol";

contract MessageQueueScriptTest is Test {
    function setUp() public {}

    function test_Upgrade() public {
        vm.setEnv("PRIVATE_KEY", "1");
        MessageQueueScript upgradeScript = new MessageQueueScript();
        upgradeScript.setUp();
        upgradeScript.run();
    }
}
