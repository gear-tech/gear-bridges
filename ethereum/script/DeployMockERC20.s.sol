// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";

contract DeployMockERC20Script is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        ERC20Mock token_1 = new ERC20Mock("USDC");
        ERC20Mock token_2 = new ERC20Mock("USDT");

        console.log("USDC:", address(token_1));
        console.log("USDT:", address(token_2));

        vm.stopBroadcast();
    }
}
