// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {console, Script} from "forge-std/Script.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";

contract ERC20ManagerScript is Script {
    ERC20Manager public erc20Manager;

    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(privateKey);

        erc20Manager = new ERC20Manager();
        console.log("ERC20Manager:", address(erc20Manager));

        vm.stopBroadcast();
    }
}
