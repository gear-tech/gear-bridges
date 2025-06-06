// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {WrappedEther} from "../src/erc20/WrappedEther.sol";

contract DeployWrappedEtherScript is Script {
    function setUp() public {}

    function run() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(privateKey);

        WrappedEther wrappedEther = new WrappedEther();
        console.log("WETH:", address(wrappedEther));

        vm.stopBroadcast();
    }
}
