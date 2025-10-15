// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {WrappedVaraScript} from "script/upgrades/WrappedVara.s.sol";

contract WrappedVaraScriptTest is Test {
    function setUp() public {}

    function test_Upgrade() public {
        /// forge-lint: disable-next-line(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        WrappedVaraScript upgradeScript = new WrappedVaraScript();
        upgradeScript.setUp();
        upgradeScript.run();
    }
}
