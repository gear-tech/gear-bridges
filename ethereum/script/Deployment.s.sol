// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {Base} from "test/Base.sol";

contract DeploymentScript is Script, Base {
    function setUp() public {}

    function run() public {
        deployBridgeFromEnvironment();
    }
}
