// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {VerifierMainnetScript} from "script/upgrades/VerifierMainnet.s.sol";

contract VerifierMainnetScriptTest is Test {
    function test_Upgrade() public {
        // forge-lint: disable-next-item(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        VerifierMainnetScript upgradeScript = new VerifierMainnetScript();
        upgradeScript.run();
    }
}
