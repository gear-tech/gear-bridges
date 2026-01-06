// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {Test} from "forge-std/Test.sol";
import {VerifierMockScript} from "script/upgrades/VerifierMock.s.sol";

contract VerifierMockScriptTest is Test {
    function setUp() public {}

    function test_Upgrade() public {
        /// forge-lint: disable-next-line(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        VerifierMockScript upgradeScript = new VerifierMockScript();
        upgradeScript.setUp();
        upgradeScript.run();
    }
}
