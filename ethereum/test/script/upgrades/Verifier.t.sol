// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {VerifierScript} from "script/upgrades/Verifier.s.sol";

contract VerifierScriptTest is Test {
    function setUp() public {}

    function test_Upgrade() public {
        vm.setEnv("PRIVATE_KEY", "1");
        VerifierScript upgradeScript = new VerifierScript();
        upgradeScript.setUp();
        upgradeScript.run();
    }
}
