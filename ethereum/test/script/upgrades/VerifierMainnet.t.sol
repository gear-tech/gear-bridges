// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {Test} from "forge-std/Test.sol";
import {VerifierMainnetScript} from "script/upgrades/VerifierMainnet.s.sol";

contract VerifierMainnetScriptTest is Test {
    function setUp() public {}

    function test_Upgrade() public {
        /// forge-lint: disable-next-line(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        VerifierMainnetScript upgradeScript = new VerifierMainnetScript();
        upgradeScript.setUp();
        upgradeScript.run();
    }
}
