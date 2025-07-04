// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {Base} from "./Base.sol";

contract MainTest is Test, Base {
    function setUp() public {
        deployBridgeFromConstants();
    }

    function test_Bridge() public {}
}
