// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Script} from "forge-std/Script.sol";
import {DeploymentBase} from "script/DeploymentBase.sol";

contract DeploymentScript is Script, DeploymentBase {
    function run() public {
        deployBridgeFromEnvironment();
    }
}
