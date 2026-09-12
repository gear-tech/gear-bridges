// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {ERC20ManagerScript} from "script/upgrades/ERC20Manager.s.sol";

contract ERC20ManagerScriptTest is Test {
    function test_Upgrade() public {
        // forge-lint: disable-next-item(unsafe-cheatcode)
        vm.setEnv("PRIVATE_KEY", "1");
        ERC20ManagerScript upgradeScript = new ERC20ManagerScript();
        upgradeScript.run();
    }
}
